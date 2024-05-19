use crate::{
    appraiser::Appraiser,
    balance::{BalanceError, BalanceSheet},
    config::Config,
    cooldown::Cooldown,
    countdown::Countdown,
    exchange_client::ExchangeClientMessage,
    f,
    id::{HandleId, HandleOrderKind, OrderId, SmartId, SmartIdError},
    instruction::{Instruction, InstructionError, InstructionExecutionResult, InstructionKind, InstructionMetadata, InstructionState, ManagedInstruction},
    leverage::Leverage,
    model::Symbol,
    order::{Order, OrderError, OrderFill, OrderMetadata, OrderMode, OrderState, StoplossMetadata},
    plot_action::{PlotActionError, PlotActionInput, PlotActionInputType},
    position::Position,
    ratelimiter::UnifiedRateLimiter,
    timeframe::Timeframe,
    trade::{Trade, TradeBuilder, TradeBuilderConfig, TradeError, TradeType},
    trade_closer::{Closer, CloserError, CloserKind, CloserMetadata},
    trigger::{InstantTrigger, Trigger, TriggerError, TriggerKind, TriggerMetadata},
    types::{
        DataPoint, NormalizedBalanceEvent, NormalizedCommission, NormalizedExecutionType, NormalizedOrderEvent, NormalizedOrderStatus, NormalizedOrderType,
        NormalizedSide, NormalizedTimeInForce, OrderedValueType, Origin, Priority, Quantity,
    },
    util::{new_id_u64, round_float_to_precision},
};
use chrono::Utc;
use crossbeam_channel::Sender;
use indexmap::IndexMap;
use itertools::Itertools;
use log::{debug, error, info, warn};
use num_traits::Zero;
use ordered_float::OrderedFloat;
use parking_lot::{MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use portable_atomic::AtomicF64;
use smallvec::SmallVec;
use std::{
    fmt::Debug,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use thiserror::Error;
use ustr::{ustr, Ustr, UstrMap};

#[derive(Error, Debug, PartialEq)]
pub enum TraderError {
    #[error("Symbol not found: {0}")]
    SymbolNotFound(Ustr),

    #[error("Symbol context is already created: {0}")]
    SymbolContextAlreadyExists(Ustr),

    #[error("Insufficient funds: requested={0:?}, available={1:?}")]
    InsufficientFunds(OrderedValueType, OrderedValueType),

    #[error("Invalid order type: {0:?}")]
    InvalidOrderType(NormalizedOrderType),

    #[error("Invalid order price: {0:?}")]
    InvalidOrderPrice(OrderedValueType),

    #[error("Invalid order quantity: {0:?}")]
    InvalidOrderQuantity(OrderedValueType),

    #[error("Max orders per symbol exceeded: {0:?}")]
    MaxOrdersPerSymbolExceeded(Ustr, usize),

    #[error("Max trades per symbol exceeded: {0:?}")]
    MaxTradesPerSymbolExceeded(Ustr, usize),

    #[error("Non-zero trade id: {0:?}")]
    NonZeroTradeId(HandleId),

    #[error("Trade not found: {0:?}")]
    TradeNotFound(Ustr, HandleId),

    #[error("Instruction is not ready for execution: {0:#?}")]
    UnsupportedInstructionExecutionResult(InstructionExecutionResult),

    #[error("Unhandled order event: symbol={0:?} local_order_id={1:?} exchange_order_id={2:?} execution_type={3:?} status={4:?}")]
    UnhandledOrderEvent(Ustr, u64, u64, NormalizedExecutionType, NormalizedOrderStatus),

    #[error("Invalid order state: expected={0:?} actual={1:?}")]
    InvalidOrderState(OrderState, OrderState),

    #[error("Invalid stoploss order type: {0:?}")]
    InvalidStoplossOrderType(NormalizedOrderType),

    #[error("Error: {0}")]
    ErrorDescription(String),

    #[error(transparent)]
    OrderError(#[from] OrderError),

    #[error(transparent)]
    CloserError(#[from] CloserError),

    #[error(transparent)]
    TriggerError(#[from] TriggerError),

    #[error(transparent)]
    InstructionError(#[from] InstructionError),

    #[error(transparent)]
    TradeError(#[from] TradeError),

    #[error(transparent)]
    BalanceError(#[from] BalanceError),

    #[error(transparent)]
    SmartIdError(#[from] SmartIdError),
}

#[derive(Debug, Copy, Clone, Default)]
pub struct TraderStats {
    pub orders_failed_to_open:    u64,
    pub orders_opened:            u64,
    pub orders_rejected:          u64,
    pub orders_expired:           u64,
    pub orders_cancelled:         u64,
    pub orders_closed:            u64,
    pub orders_failed_to_cancel:  u64,
    pub orders_failed_to_replace: u64,
    pub orders_replaced:          u64,
}

#[derive(Debug)]
pub struct TraderSymbolContext {
    pub symbol:                                Ustr,
    pub quote_asset:                           Ustr,
    pub base_asset:                            Ustr,
    pub atomic_market_price:                   Arc<AtomicF64>,
    pub entry_cooldown:                        Cooldown,
    pub exit_cooldown:                         Cooldown,
    pub appraiser:                             Appraiser,
    pub commission:                            NormalizedCommission,
    pub max_quote_allocated:                   OrderedValueType,
    pub min_quote_per_grid:                    OrderedValueType,
    pub max_quote_per_grid:                    OrderedValueType,
    pub max_orders:                            usize,
    pub max_orders_per_trade:                  usize,
    pub min_order_spacing_ratio:               OrderedValueType,
    pub max_orders_per_cooldown:               usize,
    pub max_retry_attempts:                    usize,
    pub retry_delay:                           Duration,
    pub entry_grid_orders:                     usize,
    pub exit_grid_orders:                      usize,
    pub initial_stop_ratio:                    OrderedValueType,
    pub stop_limit_offset_ratio:               OrderedValueType,
    pub initial_trailing_step_ratio:           OrderedValueType,
    pub trailing_step_ratio:                   OrderedValueType,
    pub baseline_risk_ratio:                   OrderedValueType,
    pub position:                              Position,
    pub orders:                                IndexMap<OrderId, Order>,
    pub trades:                                IndexMap<HandleId, Trade>,
    pub is_paused:                             bool,
    pub is_zero_maker_fee:                     bool,
    pub is_zero_taker_fee:                     bool,
    pub is_iceberg_allowed_by_exchange:        bool,
    pub is_iceberg_trading_enabled:            bool,
    pub is_spot_trading_allowed_by_exchange:   bool,
    pub is_margin_trading_allowed_by_exchange: bool,
    pub is_long_trading_enabled:               bool,
    pub is_short_trading_enabled:              bool,
    pub stats:                                 TraderStats,
}

#[derive(Debug)]
struct TraderInner {
    balance:                BalanceSheet,
    ratelimiter:            UnifiedRateLimiter,
    // NOTE: Explicit name to avoid confusion with the Trader, Coin and CoinContext structs
    trader_symbol_contexts: UstrMap<TraderSymbolContext>,
    exchange_client_sender: Sender<ExchangeClientMessage>,
    datapoint_sender:       Sender<DataPoint>,
    is_paused:              bool,
    stats:                  TraderStats,
}

#[derive(Clone)]
pub struct Trader(Arc<RwLock<TraderInner>>);

impl Debug for Trader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trader")
            .field("trader_symbol_contexts", &self.0.read().trader_symbol_contexts)
            .field("is_paused", &self.0.read().is_paused)
            .field("stats", &self.0.read().stats)
            .finish()
    }
}

impl Trader {
    pub fn new(balance: BalanceSheet, exchange_client_sender: Sender<ExchangeClientMessage>, datapoint_sender: Sender<DataPoint>) -> Self {
        Self(Arc::new(RwLock::new(TraderInner {
            balance,
            ratelimiter: Default::default(),
            is_paused: false,
            stats: Default::default(),
            exchange_client_sender,
            datapoint_sender,
            trader_symbol_contexts: Default::default(),
        })))
    }

    /// DEPRECATED: remove this function
    pub(crate) fn external_id_to_internal_id(&self, p0: u64) -> OrderId {
        todo!()
    }

    pub fn validate(&self) -> Result<(), TraderError> {
        // TODO: validate the trader
        Ok(())
    }

    pub fn get_commission(&self, symbol: Ustr, order_type: NormalizedOrderType) -> Result<Option<OrderedValueType>, TraderError> {
        let inner = self.0.read();
        let cx = inner
            .trader_symbol_contexts
            .get(&symbol)
            .ok_or_else(|| TraderError::SymbolNotFound(symbol.clone()))?;

        Ok(if order_type.is_limit_maker() {
            if cx.is_zero_maker_fee {
                None
            } else {
                cx.commission.maker
            }
        } else if order_type.is_taker() {
            if cx.is_zero_taker_fee {
                None
            } else {
                cx.commission.taker
            }
        } else {
            None
        })
    }

    pub fn round_quote(&self, symbol: Ustr, value: OrderedValueType) -> Result<OrderedValueType, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
        Ok(f!(round_float_to_precision(value.0, cx.appraiser.true_quote_precision)))
    }

    pub fn round_base(&self, symbol: Ustr, value: OrderedValueType) -> Result<OrderedValueType, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
        Ok(f!(round_float_to_precision(value.0, cx.appraiser.true_base_precision)))
    }

    pub fn round_normalized_quote(&self, symbol: Ustr, value: OrderedValueType) -> Result<OrderedValueType, TraderError> {
        let inner = self.0.read();
        let appraiser = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?.appraiser;
        Ok(f!(round_float_to_precision(appraiser.normalize_quote(value), appraiser.true_quote_precision)))
    }

    pub fn round_normalized_base(&self, symbol: Ustr, value: OrderedValueType) -> Result<OrderedValueType, TraderError> {
        let inner = self.0.read();
        let appraiser = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?.appraiser;
        Ok(f!(round_float_to_precision(appraiser.normalize_base(value.0), appraiser.true_base_precision)))
    }

    pub fn create_context(&self, new_context: TraderSymbolContext) -> Result<(), TraderError> {
        let mut inner = self.0.write();

        if inner.trader_symbol_contexts.contains_key(&new_context.symbol) {
            return Err(TraderError::SymbolContextAlreadyExists(new_context.symbol));
        }

        inner.trader_symbol_contexts.insert(new_context.symbol, new_context);

        Ok(())
    }

    pub fn remove_context(&self, symbol: Ustr) -> Result<(), TraderError> {
        let mut inner = self.0.write();

        if !inner.trader_symbol_contexts.contains_key(&symbol) {
            return Err(TraderError::SymbolNotFound(symbol));
        }

        inner.trader_symbol_contexts.remove(&symbol);

        Ok(())
    }

    pub fn get_context(&self, symbol: Ustr) -> Result<MappedRwLockReadGuard<'_, TraderSymbolContext>, TraderError> {
        RwLockReadGuard::try_map(self.0.read(), |inner| inner.trader_symbol_contexts.get(&symbol)).map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_context_mut(&self, symbol: Ustr) -> Result<MappedRwLockWriteGuard<'_, TraderSymbolContext>, TraderError> {
        RwLockWriteGuard::try_map(self.0.write(), |inner| inner.trader_symbol_contexts.get_mut(&symbol)).map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn pause(&mut self) {
        self.0.write().is_paused = true;
    }

    pub fn resume(&mut self) {
        self.0.write().is_paused = false;
    }

    pub fn is_paused(&self) -> bool {
        self.0.read().is_paused
    }

    pub fn get_stats(&self) -> MappedRwLockReadGuard<'_, TraderStats> {
        RwLockReadGuard::map(self.0.read(), |inner| &inner.stats)
    }

    fn get_stats_mut(&mut self) -> MappedRwLockWriteGuard<'_, TraderStats> {
        RwLockWriteGuard::map(self.0.write(), |inner| &mut inner.stats)
    }

    pub fn get_stats_for_symbol(&self, symbol: Ustr) -> Result<TraderStats, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
        Ok(cx.stats)
    }

    pub fn get_position(&self, symbol: Ustr) -> Result<MappedRwLockReadGuard<'_, Position>, TraderError> {
        RwLockReadGuard::try_map(self.0.read(), |inner| {
            match inner.trader_symbol_contexts.get(&symbol) {
                Some(cx) => Ok(&cx.position),
                None => Err(TraderError::SymbolNotFound(symbol)),
            }
            .ok()
        })
        .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_position_mut(&self, symbol: Ustr) -> Result<MappedRwLockWriteGuard<'_, Position>, TraderError> {
        RwLockWriteGuard::try_map(self.0.write(), |inner| {
            match inner.trader_symbol_contexts.get_mut(&symbol) {
                Some(cx) => Ok(&mut cx.position),
                None => Err(TraderError::SymbolNotFound(symbol)),
            }
            .ok()
        })
        .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_trade(&self, symbol: Ustr, trade_id: HandleId) -> Result<MappedRwLockReadGuard<'_, Trade>, TraderError> {
        RwLockReadGuard::try_map(self.0.read(), |inner| {
            match inner.trader_symbol_contexts.get(&symbol) {
                Some(cx) => match cx.trades.get(&trade_id) {
                    Some(trade) => Ok(trade),
                    None => Err(TraderError::TradeNotFound(symbol, trade_id)),
                },
                None => Err(TraderError::SymbolNotFound(symbol)),
            }
            .ok()
        })
        .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_trade_mut(&self, symbol: Ustr, trade_id: HandleId) -> Result<MappedRwLockWriteGuard<'_, Trade>, TraderError> {
        RwLockWriteGuard::try_map(self.0.write(), |inner| {
            match inner.trader_symbol_contexts.get_mut(&symbol) {
                Some(cx) => match cx.trades.get_mut(&trade_id) {
                    Some(trade) => Ok(trade),
                    None => Err(TraderError::TradeNotFound(symbol, trade_id)),
                },
                None => Err(TraderError::SymbolNotFound(symbol)),
            }
            .ok()
        })
        .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_orders(&self, symbol: Ustr) -> Result<MappedRwLockReadGuard<'_, IndexMap<OrderId, Order>>, TraderError> {
        let guard = self.0.read();
        let cx = guard.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;

        RwLockReadGuard::try_map(guard, |inner| inner.trader_symbol_contexts.get(&symbol).map(|cx| &cx.orders)).map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_orders_mut(&self, symbol: Ustr) -> Result<MappedRwLockWriteGuard<'_, IndexMap<OrderId, Order>>, TraderError> {
        let guard = self.0.write();
        let cx = guard.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;

        RwLockWriteGuard::try_map(guard, |inner| inner.trader_symbol_contexts.get_mut(&symbol).map(|cx| &mut cx.orders))
            .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_trades(&self, symbol: Ustr) -> Result<MappedRwLockReadGuard<'_, IndexMap<HandleId, Trade>>, TraderError> {
        let guard = self.0.read();
        let cx = guard.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;

        RwLockReadGuard::try_map(guard, |inner| inner.trader_symbol_contexts.get(&symbol).map(|cx| &cx.trades)).map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_trades_mut(&self, symbol: Ustr) -> Result<MappedRwLockWriteGuard<'_, IndexMap<HandleId, Trade>>, TraderError> {
        let guard = self.0.write();
        let cx = guard.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;

        RwLockWriteGuard::try_map(guard, |inner| inner.trader_symbol_contexts.get_mut(&symbol).map(|cx| &mut cx.trades))
            .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_order_by_local_id(&self, symbol: Ustr, local_order_id: OrderId) -> Result<MappedRwLockReadGuard<'_, Order>, TraderError> {
        RwLockReadGuard::try_map(self.0.read(), |inner| {
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol)).unwrap();
            let trade_id = SmartId(local_order_id).handle_id();
            let order_type = SmartId(local_order_id).order_kind();

            let trade = cx.trades.get(&trade_id).ok_or(TradeError::TradeNotFound(symbol, trade_id)).unwrap();

            match order_type {
                HandleOrderKind::EntryOrder => Some(trade.entry.as_ref()),
                HandleOrderKind::ExitOrder => Some(trade.exit.as_ref()),
                HandleOrderKind::StoplossOrder => Some(trade.stoploss.as_ref()),
                _ => {
                    warn!("{}: Local order ID's encoded order type is not specified or is not recognized: {:?}", symbol, order_type);
                    None
                }
            }
        })
        .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    pub fn get_order_mut(&self, symbol: Ustr, local_order_id: OrderId) -> Result<MappedRwLockWriteGuard<'_, Order>, TraderError> {
        RwLockWriteGuard::try_map(self.0.write(), |inner| {
            let cx = inner
                .trader_symbol_contexts
                .get_mut(&symbol)
                .ok_or(TraderError::SymbolNotFound(symbol))
                .unwrap();
            let trade_id = SmartId(local_order_id).handle_id();
            let order_type = SmartId(local_order_id).order_kind();

            let trade = cx.trades.get_mut(&trade_id).ok_or(TradeError::TradeNotFound(symbol, trade_id)).unwrap();

            match order_type {
                HandleOrderKind::EntryOrder => Some(trade.entry.as_mut()),
                HandleOrderKind::ExitOrder => Some(trade.exit.as_mut()),
                HandleOrderKind::StoplossOrder => Some(trade.stoploss.as_mut()),
                _ => {
                    warn!("{}: Local order ID's encoded order type is not specified or is not recognized: {:?}", symbol, order_type);
                    None
                }
            }
        })
        .map_err(|_| TraderError::SymbolNotFound(symbol))
    }

    // IMPORTANT: Trade ID is derived from the order ID
    pub fn order_template(
        &self,
        symbol: Ustr,
        local_order_id: OrderId,
        side: NormalizedSide,
        order_type: NormalizedOrderType,
        quantity: OrderedValueType,
        price: OrderedValueType,
        limit_price: Option<OrderedValueType>,
        is_entry_order: bool,
    ) -> Result<Order, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;

        let template = match side {
            NormalizedSide::Buy => self.buy_order_template(symbol, local_order_id, order_type, quantity, price, limit_price, is_entry_order)?,
            NormalizedSide::Sell => self.sell_order_template(symbol, local_order_id, order_type, quantity, price, limit_price, is_entry_order)?,
        };

        Ok(template)
    }

    pub fn stoploss_template(
        &self,
        symbol: Ustr,
        local_order_id: OrderId,
        side: NormalizedSide,
        order_type: NormalizedOrderType,
        quantity: OrderedValueType,
        reference_price: OrderedValueType,
        stop_price: OrderedValueType,
        limit_price: Option<OrderedValueType>,
        is_trailing_enabled: bool,
        trailing_step_ratio: OrderedValueType,
    ) -> Result<Order, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
        let metadata = OrderMetadata {
            origin:                   Origin::Local,
            is_virtual:               false,
            limit_price_offset_ratio: cx.stop_limit_offset_ratio,
            stoploss:                 Some(StoplossMetadata {
                is_trailing_enabled,
                reference_price,
                initial_stop_price: stop_price,
                trailing_step_ratio,
            }),
        };

        // Initialize the order template
        let mut template = Order {
            local_order_id:                 local_order_id,
            raw_order_id:                   None,
            local_stop_order_id:            None,
            local_orig_order_id:            None,
            local_list_order_id:            None,
            exchange_order_id:              None,
            trade_id:                       SmartId(local_order_id).handle_id(),
            mode:                           OrderMode::Normal,
            pending_mode:                   None,
            symbol:                         cx.symbol,
            atomic_market_price:            cx.atomic_market_price.clone(),
            appraiser:                      cx.appraiser,
            timeframe:                      Timeframe::NA,
            time_in_force:                  NormalizedTimeInForce::GTC,
            side:                           NormalizedSide::Sell,
            order_type:                     order_type,
            contingency_order_type:         Some(NormalizedOrderType::Market),
            activator:                      Box::new(InstantTrigger::new()),
            deactivator:                    Box::new(InstantTrigger::new()),
            market_price_at_initialization: Some(f!(cx.atomic_market_price.load(Ordering::SeqCst))),
            market_price_at_opening:        None,
            price:                          stop_price,
            limit_price:                    limit_price,
            initial_quantity:               quantity,
            accumulated_filled_quantity:    f!(0.0),
            accumulated_filled_quote:       f!(0.0),
            fee_ratio:                      Some(f!(0.001000)),
            retry_counter:                  Countdown::new(cx.max_retry_attempts as u8),
            retry_cooldown:                 Cooldown::new(Duration::from_millis(300), 1),
            lifecycle:                      Default::default(),
            leverage:                       Leverage::no_leverage(),
            state:                          OrderState::Idle,
            termination_flag:               false,
            cancel_and_idle_flag:           false,
            cancel_and_replace_flag:        false,
            fills:                          SmallVec::new(),
            num_updates:                    0,
            is_entry_order:                 false,
            is_created_on_exchange:         false,
            is_finished_by_cancel:          false,
            pending_instruction:            None,
            special_pending_instruction:    None,
            initialized_at:                 Utc::now(),
            opened_at:                      None,
            updated_at:                     None,
            trailing_activated_at:          None,
            trailing_updated_at:            None,
            closed_at:                      None,
            cancelled_at:                   None,
            finalized_at:                   None,
            metadata:                       metadata,
        };

        template.validate()?;

        Ok(template)
    }

    pub fn buy_order_template(
        &self,
        symbol: Ustr,
        local_order_id: OrderId,
        order_type: NormalizedOrderType,
        quantity: OrderedValueType,
        price: OrderedValueType,
        limit_price: Option<OrderedValueType>,
        is_entry_order: bool,
    ) -> Result<Order, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
        let trade_id = SmartId(local_order_id).handle_id();

        let metadata = OrderMetadata {
            origin:                   Origin::Local,
            is_virtual:               false,
            limit_price_offset_ratio: Default::default(),
            stoploss:                 Default::default(),
        };

        let mut template = Order {
            local_order_id:                 local_order_id,
            raw_order_id:                   None,
            local_stop_order_id:            None,
            local_orig_order_id:            None,
            local_list_order_id:            None,
            exchange_order_id:              None,
            trade_id:                       trade_id,
            mode:                           OrderMode::Normal,
            pending_mode:                   None,
            symbol:                         cx.symbol,
            atomic_market_price:            cx.atomic_market_price.clone(),
            appraiser:                      cx.appraiser,
            timeframe:                      Timeframe::NA,
            time_in_force:                  NormalizedTimeInForce::GTC,
            side:                           NormalizedSide::Buy,
            order_type:                     order_type,
            contingency_order_type:         Some(NormalizedOrderType::Market),
            activator:                      Box::new(InstantTrigger::new()),
            deactivator:                    Box::new(InstantTrigger::new()),
            market_price_at_initialization: Some(f!(cx.atomic_market_price.load(Ordering::SeqCst))),
            market_price_at_opening:        None,
            price:                          price,
            limit_price:                    limit_price,
            initial_quantity:               quantity,
            accumulated_filled_quantity:    f!(0.0),
            accumulated_filled_quote:       f!(0.0),
            fee_ratio:                      Some(f!(0.001000)),
            retry_counter:                  Countdown::new(cx.max_retry_attempts as u8),
            retry_cooldown:                 Cooldown::new(Duration::from_millis(300), 1),
            lifecycle:                      Default::default(),
            leverage:                       Leverage::no_leverage(),
            state:                          OrderState::Idle,
            termination_flag:               false,
            cancel_and_idle_flag:           false,
            cancel_and_replace_flag:        false,
            fills:                          SmallVec::new(),
            num_updates:                    0,
            is_entry_order:                 is_entry_order,
            is_created_on_exchange:         false,
            is_finished_by_cancel:          false,
            pending_instruction:            None,
            special_pending_instruction:    None,
            initialized_at:                 Utc::now(),
            opened_at:                      None,
            updated_at:                     None,
            trailing_activated_at:          None,
            trailing_updated_at:            None,
            closed_at:                      None,
            cancelled_at:                   None,
            finalized_at:                   None,
            metadata:                       metadata,
        };

        template.validate()?;

        Ok(template)
    }

    fn sell_order_template(
        &self,
        symbol: Ustr,
        local_order_id: OrderId,
        order_type: NormalizedOrderType,
        quantity: OrderedValueType,
        price: OrderedValueType,
        limit_price: Option<OrderedValueType>,
        is_entry_order: bool,
    ) -> Result<Order, TraderError> {
        let inner = self.0.read();
        let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
        let metadata = OrderMetadata::default();

        // Initialize the order template
        let mut template = Order {
            local_order_id:                 local_order_id,
            raw_order_id:                   None,
            local_stop_order_id:            None,
            local_orig_order_id:            None,
            local_list_order_id:            None,
            exchange_order_id:              None,
            trade_id:                       SmartId(local_order_id).handle_id(),
            mode:                           OrderMode::Normal,
            pending_mode:                   None,
            symbol:                         cx.symbol,
            atomic_market_price:            cx.atomic_market_price.clone(),
            appraiser:                      cx.appraiser,
            timeframe:                      Timeframe::NA,
            time_in_force:                  NormalizedTimeInForce::GTC,
            side:                           NormalizedSide::Sell,
            order_type:                     order_type,
            contingency_order_type:         Some(NormalizedOrderType::Market),
            activator:                      Box::new(InstantTrigger::new()),
            deactivator:                    Box::new(InstantTrigger::new()),
            market_price_at_initialization: Some(f!(cx.atomic_market_price.load(Ordering::SeqCst))),
            market_price_at_opening:        None,
            price:                          price,
            limit_price:                    limit_price,
            initial_quantity:               quantity,
            accumulated_filled_quantity:    f!(0.0),
            accumulated_filled_quote:       f!(0.0),
            fee_ratio:                      Some(f!(0.001000)),
            retry_counter:                  Countdown::new(cx.max_retry_attempts as u8),
            retry_cooldown:                 Cooldown::new(Duration::from_millis(300), 1),
            lifecycle:                      Default::default(),
            leverage:                       Leverage::no_leverage(),
            state:                          OrderState::Idle,
            termination_flag:               false,
            cancel_and_idle_flag:           false,
            cancel_and_replace_flag:        false,
            fills:                          SmallVec::new(),
            num_updates:                    0,
            is_entry_order:                 is_entry_order,
            is_created_on_exchange:         false,
            is_finished_by_cancel:          false,
            pending_instruction:            None,
            special_pending_instruction:    None,
            initialized_at:                 Utc::now(),
            opened_at:                      None,
            updated_at:                     None,
            trailing_activated_at:          None,
            trailing_updated_at:            None,
            closed_at:                      None,
            cancelled_at:                   None,
            finalized_at:                   None,
            metadata:                       metadata,
        };

        template.validate()?;

        Ok(template)
    }

    pub fn trade_template(
        &self,
        mut config: TradeBuilderConfig,
        entry_price: OrderedValueType,
        exit_price: Option<OrderedValueType>,
        stoploss_metadata: StoplossMetadata,
    ) -> Result<Trade, TraderError> {
        let inner = self.0.read();
        let cx = inner
            .trader_symbol_contexts
            .get(&config.symbol)
            .ok_or(TraderError::SymbolNotFound(config.symbol))?;

        let symbol = config.symbol;

        let trade_id = config.specific_trade_id.unwrap_or_else(|| SmartId::new_handle_id());
        let entry_order_id = SmartId::new_order_id(trade_id, HandleOrderKind::EntryOrder);
        let exit_order_id = SmartId::new_order_id(trade_id, HandleOrderKind::ExitOrder);
        let stoploss_order_id = SmartId::new_order_id(trade_id, HandleOrderKind::StoplossOrder);

        // IMPORTANT: Overriding specific trade id in the builder config
        config.specific_trade_id = Some(trade_id);

        // ----------------------------------------------------------------------------
        // Initializing the entry order
        // ----------------------------------------------------------------------------

        let entry_side = match config.trade_type {
            TradeType::Long => NormalizedSide::Buy,
            TradeType::Short => NormalizedSide::Sell,
        };

        let mut entry = self.order_template(symbol, entry_order_id, entry_side, config.entry_order_type, config.quantity, entry_price, None, false)?;

        entry.timeframe = config.timeframe;
        entry.time_in_force = config.entry_time_in_force;
        entry.initial_quantity = config.quantity;

        // ----------------------------------------------------------------------------
        // Initializing the exit order
        // ----------------------------------------------------------------------------

        let mut exit = self.order_template(
            symbol,
            exit_order_id,
            entry_side.opposite(),
            config.exit_order_type,
            config.quantity,
            exit_price.unwrap_or(OrderedFloat::zero()),
            None,
            false,
        )?;

        exit.timeframe = config.timeframe;
        exit.time_in_force = config.exit_time_in_force;

        // ----------------------------------------------------------------------------
        // Initializing the stoploss order
        // ----------------------------------------------------------------------------

        let stoploss_side = exit.side.opposite();
        let stoploss_price = stoploss_metadata.initial_stop_price;

        let stoploss_limit_price = match (stoploss_side, config.stoploss_order_type) {
            (NormalizedSide::Buy, NormalizedOrderType::StopLossLimit) => stoploss_metadata.initial_stop_price * (f!(1.0) - cx.stop_limit_offset_ratio),
            (NormalizedSide::Sell, NormalizedOrderType::StopLossLimit) => stoploss_metadata.initial_stop_price * (f!(1.0) + cx.stop_limit_offset_ratio),

            // NOTE: Market order has no limit price
            _ => return Err(TraderError::InvalidStoplossOrderType(config.stoploss_order_type)),
        };

        let mut stoploss = self.stoploss_template(
            symbol,
            stoploss_order_id,
            exit.side,
            config.stoploss_order_type,
            config.quantity,
            entry.price,
            stoploss_price,
            Some(stoploss_limit_price),
            stoploss_metadata.is_trailing_enabled,
            stoploss_metadata.trailing_step_ratio,
        )?;

        stoploss.timeframe = config.timeframe;

        // ----------------------------------------------------------------------------
        // Building the trade
        // ----------------------------------------------------------------------------

        let closer_metadata = match config.closer_kind {
            CloserKind::NoCloser => {
                // No closer is required, so we just need to add the entry order
                CloserMetadata::no_closer()
            }

            CloserKind::TakeProfit => {
                if exit.price.is_zero() {
                    return Err(CloserError::MissingExitPrice.into());
                }

                CloserMetadata::take_profit(entry_price, false, exit.price)
            }

            CloserKind::TrendFollowing => {
                if config.timeframe == Timeframe::NA {
                    return Err(TradeError::InvalidTimeframe(config.timeframe).into());
                }

                CloserMetadata::trend_following(entry_price, config.timeframe)
            }
        };

        // Initializing the builder
        let trade = TradeBuilder::new(config)
            .with_entry(entry)
            .with_exit(exit)
            .with_stoploss(stoploss)
            .with_closer(closer_metadata.into_closer())
            .build()?;

        Ok(trade)
    }

    pub fn can_create_order(&self, order: &Order) -> Result<bool, TraderError> {
        let inner = self.0.read();
        let cx = inner
            .trader_symbol_contexts
            .get(&order.symbol)
            .ok_or(TraderError::SymbolNotFound(order.symbol))?;

        // Checking if the trader is paused
        if cx.is_paused {
            return Ok(false);
        }

        // Checking if the order is already closed (just in case)
        if order.is_closed() {
            return Ok(false);
        }

        // Checking if the active order limit has been reached
        if cx.orders.len() >= cx.max_orders {
            return Ok(false);
        }

        // TODO: Implement retry counter check which is exclusive with cooldown
        // FIXME: To implement retry counter logic inside trader, Order needs to be a mutable reference

        // Checking the cooldowns
        if order.is_entry_order && !cx.entry_cooldown.is_available() {
            return Ok(false);
        } else if !order.is_entry_order && !cx.exit_cooldown.is_available() {
            return Ok(false);
        }

        // Checking the balance (depending on the order side)
        if order.side == NormalizedSide::Buy {
            let quote_equivalent = f!(cx.appraiser.normalize_quote(order.initial_quantity * order.price));
            if !inner.balance.check(cx.quote_asset, quote_equivalent)? {
                return Ok(false);
            }
        } else {
            if !inner.balance.check(cx.base_asset, order.initial_quantity)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub fn create_order(&mut self, order: Order) -> Result<(), TraderError> {
        order.validate()?;

        if !self.can_create_order(&order)? {
            return Ok(());
        }

        {
            let mut inner = self.0.write();
            let cx = inner
                .trader_symbol_contexts
                .get_mut(&order.symbol)
                .ok_or(TraderError::SymbolNotFound(order.symbol))?;

            match cx.orders.entry(order.local_order_id) {
                indexmap::map::Entry::Occupied(_) => {
                    return Err(TraderError::OrderError(OrderError::OrderLocalIdAlreadyExists(order.local_order_id)));
                }

                indexmap::map::Entry::Vacant(mut entry) => {
                    let cooldown_is_ok =
                        (order.is_entry_order && cx.entry_cooldown.use_cooldown()) || (!order.is_entry_order && cx.exit_cooldown.use_cooldown());

                    if cooldown_is_ok {
                        entry.insert(order);
                    }
                }
            }
        }

        self.compute()
    }

    pub fn create_trade(&mut self, trade: Trade) -> Result<(), TraderError> {
        // Validating the trade before attempting anything
        trade.validate()?;

        if !trade.is_new() {
            return Err(TraderError::TradeError(TradeError::TradeNotReady(trade.symbol, trade.id, trade.state)));
        }

        {
            let mut inner = self.0.write();
            let cx = inner
                .trader_symbol_contexts
                .get_mut(&trade.symbol)
                .ok_or(TraderError::SymbolNotFound(trade.symbol))?;

            match cx.trades.entry(trade.id) {
                indexmap::map::Entry::Occupied(_) => {
                    return Err(TraderError::TradeError(TradeError::TradeIdAlreadyExists(trade.id)));
                }

                indexmap::map::Entry::Vacant(mut entry) => {
                    // Adding the trade to the list of tracked trades
                    entry.insert(trade);
                }
            }
        }

        self.compute()
    }

    /// This function is responsible for processing the trades and orders in the system.
    /// It iterates through the trades and orders, updating their states and handling
    /// any pending instructions.
    pub fn compute(&mut self) -> Result<(), TraderError> {
        let mut closed_orders = Vec::with_capacity(4);
        let mut closed_trades = Vec::with_capacity(4);
        let mut instructions = Vec::with_capacity(16);

        {
            let mut inner = self.0.write();

            for (&symbol, cx) in inner.trader_symbol_contexts.iter_mut() {
                let current_market_price = f!(cx.atomic_market_price.load(Ordering::SeqCst));

                // eprintln!("cx.orders.len() = {:#?}", cx.orders.len());

                // Processing orders
                for (_, order) in cx.orders.iter_mut() {
                    if order.is_closed() {
                        closed_orders.push(order.clone());
                        continue;
                    }

                    if let Err(err) = order.compute() {
                        error!("{}: Failed to process order {}: {:#?}", symbol, order.local_order_id, err);
                    }

                    // Handling the special instruction
                    if let Some(special_instruction) = order.get_special_pending_instruction_mut() {
                        if special_instruction.is_new() {
                            special_instruction.set_state(InstructionState::Running)?;
                            instructions.push(special_instruction.clone());
                        }

                        // WARNING:
                        // Special instruction supresses all other instructions for
                        // that order, so we're skipping the rest
                        continue;
                    } else {
                        // Processing the order's instruction (if it has any)
                        if let Some(instruction) = order.get_pending_instruction_mut() {
                            // Saving the instruction to be processed later
                            if instruction.is_new() {
                                instruction.set_state(InstructionState::Running)?;
                                instructions.push(instruction.clone());
                            }
                        }
                    }
                }

                // Processing trades
                for (_, trade) in cx.trades.iter_mut() {
                    if trade.is_closed() {
                        closed_trades.push(trade.clone());
                        continue;
                    }

                    // ----------------------------------------------------------------------------
                    // IMPORTANT: Order Creation and Tracking in Trader
                    // ----------------------------------------------------------------------------
                    // When a new order is instructed by a trade, it is created and added to the Trader's list of
                    // tracked orders. However, these orders are not processed immediately. Instead, they are
                    // processed later during the execution of the compute() function.
                    //
                    // Example Scenario: Stoploss Order Handling
                    // Consider a scenario where a trade instructs the creation of a stoploss order. In cases
                    // where the stoploss order involves handing over a degree of control to the exchange (like with
                    // StoplossLimit orders), the order handling process is as follows:
                    //
                    // 1. Stop Order Creation: When the stoploss order is a StoplossLimit order, the exchange first
                    //    creates a 'Stop' order. This order remains dormant until the market price reaches a predefined
                    //    stop level.
                    //
                    // 2. Order Execution Types:
                    //    - Market Order: If the stop order type is 'Market', it executes immediately when the market
                    //      price hits the stop level.
                    //    - Limit Order: If the stop order type is 'Limit', reaching the stop level triggers the creation
                    //      of a new limit order.
                    //
                    // 3. System Tracking: It is crucial for our system to track every order, especially those we expect
                    //    the exchange to create with predefined IDs. This ensures consistent tracking and management
                    //    of orders throughout their lifecycle, from creation to execution.
                    //
                    // This approach guarantees that the system maintains a comprehensive overview of all active and
                    // pending orders, enabling effective monitoring and strategy execution.
                    // ----------------------------------------------------------------------------
                    if let Err(err) = trade.compute() {
                        error!("{}: Failed to process trade {}: {:#?}", cx.symbol, trade.id, err);
                        // TODO: Decide what to do here
                    }

                    // Handling the special instruction
                    if let Some(special_instruction) = trade.get_special_pending_instruction_mut() {
                        if special_instruction.is_new() {
                            special_instruction.set_state(InstructionState::Running)?;
                            instructions.push(special_instruction.clone());
                        }

                        // WARNING:
                        // Special instruction supresses all other instructions for
                        // that trade, so we're skipping the rest
                        continue;
                    } else {
                        // TODO: Do I want to always allow this or only when the trade is NOT in OCO mode?
                        // Processing the trade's instruction (if it has any)
                        if let Some(instruction) = trade.get_pending_instruction_mut() {
                            // Saving the instruction to be processed later
                            if instruction.is_new() {
                                instruction.set_state(InstructionState::Running)?;
                                instructions.push(instruction.clone());
                            }
                        }
                    }
                }

                // Removing the closed trades
                for trade in closed_trades.drain(..) {
                    // TODO: Finalize the trade
                    info!("{}: Removing closed trade: {:#?}, reason={:?}", symbol, trade.id, trade.close_reason);
                    cx.trades.shift_remove(&trade.id);
                }
            }
        }

        {
            // ----------------------------------------------------------------------------
            // Processing the instructions
            // ----------------------------------------------------------------------------
            let mut inner = self.0.write();

            for instruction in instructions {
                {
                    let new_job = ExchangeClientMessage::Instruction(instruction.id, instruction.metadata, instruction.instruction, instruction.priority);

                    // Sending the instruction to the exchange client which is
                    // responsible for the communication with the exchange
                    let result = inner.exchange_client_sender.send(new_job);

                    if let Err(err) = result {
                        error!("{}: Failed to send instruction: instruction_id={}, error={:#?}", instruction.symbol.as_str(), instruction.id, err);
                        continue;
                    }
                }
            }
        }

        Ok(())
    }

    fn update_position(&mut self, e: &NormalizedOrderEvent) -> Result<(), TraderError> {
        match (e.current_order_status, e.current_execution_type) {
            (NormalizedOrderStatus::PartiallyFilled | NormalizedOrderStatus::Filled, NormalizedExecutionType::Trade) => {
                let latest_fill =
                    e.latest_fill
                        .ok_or(TraderError::OrderError(OrderError::OrderMissingLatestFill(e.symbol, e.local_order_id, e.exchange_order_id)))?;

                self.get_position_mut(e.symbol)?
                    .register_trade(e.side, latest_fill.metadata.quantity.0, latest_fill.metadata.price.0);
            }

            _ => {
                // Do nothing because the event is not relevant to the positions' update
            }
        }

        Ok(())
    }

    pub fn handle_order_event(&mut self, e: NormalizedOrderEvent) -> Result<(), TraderError> {
        let (handle_id, order_kind, sequence_number, unique_order_id) = SmartId(e.local_order_id).decode();

        eprintln!(
            "handle_id = {:#?} order_kind = {:#?} sequence_number = {:#?} unique_order_id = {:#?}",
            handle_id, order_kind, sequence_number, unique_order_id
        );

        // Updating the position with the latest trade
        self.update_position(&e)?;

        match order_kind {
            HandleOrderKind::None => {}
            HandleOrderKind::EntryOrder => {}
            HandleOrderKind::ExitOrder => {}
            HandleOrderKind::StoplossOrder => {}
            HandleOrderKind::SpecialOrder => {}
            HandleOrderKind::StandaloneOrder => {}
            HandleOrderKind::UnexpectedOrder => {}
            HandleOrderKind::Unknown => {}
        }

        if order_kind == HandleOrderKind::UnexpectedOrder {
            debug!("{}: Ignoring unexpected order: {:?}", e.symbol, e.exchange_order_id);
            return Ok(());
        }

        match (e.current_execution_type, e.current_order_status) {
            (NormalizedExecutionType::New, NormalizedOrderStatus::New) => {
                match (handle_id, order_kind, sequence_number, unique_order_id) {
                    (0, HandleOrderKind::None, 0, _) => {
                        // This combination is used for orders that are created on the exchange
                        // and are not associated with any trade or its order kind in the system.
                        // TODO: Decide whether I want to internalize external orders
                    }

                    (trade_id, order_kind, 0, unique_order_id) => {
                        // This combination is used for orders that are created on the exchange
                        // and are associated with a trade in the system.
                        match self.confirm_order_opened(e.symbol, e.local_order_id, Some(e.exchange_order_id)) {
                            Ok(()) => {
                                // The order is confirmed as opened (first iteration of the order)
                            }

                            Err(err) => {
                                error!(
                                    "{}: Failed to confirm order as a new order: local_order_id={}, exchange_order_id={}",
                                    e.symbol, e.local_order_id, e.exchange_order_id
                                );
                                // Some other error occurred
                                return Err(err);
                            }
                        }
                    }

                    (trade_id, order_kind, sequence_number, unique_order_id) if sequence_number > 0 => {
                        // So, the dilemma is that we don't know whether the order is a replacement or not.
                        // For now, we assume that it is a replacement order. If it is not, we will get an
                        // error when we try to confirm the order as a replacement order. In that case, we
                        // will then attempt to confirm it as an order being opened.

                        let symbol = e.symbol;
                        let local_order_id = e.local_order_id;
                        let local_parent_order_id_for_replacement = SmartId(local_order_id).decremented_id()?;
                        let exchange_order_id = e.exchange_order_id;

                        info!(
                            "{}: Order is confirmed as replaced: local_order_id={}, local_parent_order_id_for_replacement={}, exchange_order_id={}",
                            symbol, local_order_id, local_parent_order_id_for_replacement, exchange_order_id
                        );

                        // First, let's try to confirm the order as a replacement order
                        match self.confirm_order_replaced(e.symbol, local_parent_order_id_for_replacement, e.local_order_id, e.clone()) {
                            Ok(()) => {
                                // The order is confirmed as a replacement order
                            }

                            /*
                            Err(TraderError::TradeError(TradeError::OrderNotFound(_, _, _))) => {
                                warn!("{}: Failed to confirm order as a replacement, so we will try to confirm it as a new order (probably created by OCO): local_order_id={}, local_parent_order_id_for_replacement={}, exchange_order_id={}", symbol, local_order_id, local_parent_order_id_for_replacement, exchange_order_id);

                                // The order is not found by the local parent order id, so we will try to confirm it
                                // as an Opened order instead (the function will find the relevant order by the)
                                // NOTE:
                                // Confirming as Opened because we don't know whether it is a replacement or not.
                                // This could also be caused by OCO when creating its legs on the exchange.
                                match self.confirm_order_opened(symbol, local_order_id, Some(exchange_order_id)) {
                                    Ok(()) => {
                                        // WARNING: This particular case is for orders that are created on the exchange (e.g. OCO orders)
                                        // The order is confirmed as opened
                                    }

                                    Err(err) => {
                                        error!(
                                            "{}: Failed to confirm order as a new order (first tried to replace): local_order_id={}, exchange_order_id={}",
                                            symbol, local_order_id, exchange_order_id
                                        );
                                        // Some other error occurred
                                        return Err(err);
                                    }
                                }
                            }
                             */
                            Err(err) => {
                                // Some other error occurred
                                return Err(err);
                            }
                        }
                    }

                    _ => {
                        warn!("{}: Unhandled SmartId combination: {:?}", e.symbol, SmartId(e.local_order_id).decode());
                    }
                }
            }

            (NormalizedExecutionType::Trade, NormalizedOrderStatus::PartiallyFilled) => {
                let latest_fill =
                    e.latest_fill
                        .ok_or(TraderError::OrderError(OrderError::OrderMissingLatestFill(e.symbol, e.local_order_id, e.exchange_order_id)))?;
                self.confirm_order_filled(e.symbol, e.local_order_id, latest_fill)?;
            }

            (NormalizedExecutionType::Trade, NormalizedOrderStatus::Filled) => {
                let latest_fill =
                    e.latest_fill
                        .ok_or(TraderError::OrderError(OrderError::OrderMissingLatestFill(e.symbol, e.local_order_id, e.exchange_order_id)))?;
                self.confirm_order_filled(e.symbol, e.local_order_id, latest_fill)?;
            }

            (NormalizedExecutionType::Cancelled, NormalizedOrderStatus::Cancelled) => {
                self.confirm_order_cancelled(e.symbol, e.local_order_id)?;
            }

            (NormalizedExecutionType::New, NormalizedOrderStatus::Rejected) => {
                self.confirm_order_rejected(e.symbol, e.local_order_id)?;
            }

            (NormalizedExecutionType::Expired, NormalizedOrderStatus::Expired) => {
                self.confirm_order_expired(e.symbol, e.local_order_id)?;
            }

            (NormalizedExecutionType::Cancelled, NormalizedOrderStatus::PartiallyFilled) => {
                self.confirm_order_cancelled(e.symbol, e.local_order_id)?;
            }

            (execution_type, order_status) =>
                return Err(TraderError::UnhandledOrderEvent(e.symbol, e.local_order_id, e.exchange_order_id, execution_type, order_status)),
        }

        self.compute()
    }

    pub fn import_existing_open_order(&mut self, order: Order) -> Result<(), TraderError> {
        // TODO: Decide whether I want to internalize external orders
        unimplemented!();
    }

    pub fn market_buy(&mut self, symbol: Ustr, quantity: OrderedValueType, handle_order_kind: HandleOrderKind) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Buy, NormalizedOrderType::Market, quantity, OrderedFloat::zero(), None, false)?
        };

        self.create_order(order)
    }

    pub fn market_buy_quote_quantity(&mut self, symbol: Ustr, quote_quantity: OrderedValueType, handle_order_kind: HandleOrderKind) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);
            let current_market_price = f!(cx.atomic_market_price.load(Ordering::SeqCst));
            let quantity = f!(cx.appraiser.round_normalized_base(quote_quantity.0 / current_market_price.0));

            self.order_template(symbol, local_order_id, NormalizedSide::Buy, NormalizedOrderType::Market, quantity, OrderedFloat::zero(), None, false)?
        };

        self.create_order(order)
    }

    pub fn market_sell(&mut self, symbol: Ustr, quantity: OrderedValueType, handle_order_kind: HandleOrderKind) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Sell, NormalizedOrderType::Market, quantity, OrderedFloat::zero(), None, false)?
        };

        self.create_order(order)
    }

    pub fn limit_buy(
        &mut self,
        symbol: Ustr,
        price: OrderedValueType,
        limit_price: OrderedValueType,
        quantity: OrderedValueType,
        handle_order_kind: HandleOrderKind,
    ) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Buy, NormalizedOrderType::Limit, quantity, price, Some(limit_price), false)?
        };

        self.create_order(order)
    }

    pub fn limit_sell(
        &mut self,
        symbol: Ustr,
        price: OrderedValueType,
        limit_price: OrderedValueType,
        quantity: OrderedValueType,
        handle_order_kind: HandleOrderKind,
    ) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Sell, NormalizedOrderType::Limit, quantity, price, Some(limit_price), false)?
        };

        self.create_order(order)
    }

    pub fn stoploss_limit_buy(
        &mut self,
        symbol: Ustr,
        price: OrderedValueType,
        stop_price: OrderedValueType,
        limit_price: OrderedValueType,
        quantity: OrderedValueType,
        handle_order_kind: HandleOrderKind,
    ) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Buy, NormalizedOrderType::StopLossLimit, quantity, price, Some(limit_price), false)?
        };

        self.create_order(order)
    }

    pub fn stoploss_limit_sell(
        &mut self,
        symbol: Ustr,
        price: OrderedValueType,
        stop_price: OrderedValueType,
        limit_price: OrderedValueType,
        quantity: OrderedValueType,
        handle_order_kind: HandleOrderKind,
    ) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Sell, NormalizedOrderType::StopLossLimit, quantity, price, Some(limit_price), false)?
        };

        self.create_order(order)
    }

    pub fn stoploss_market_buy(
        &mut self,
        symbol: Ustr,
        price: OrderedValueType,
        stop_price: OrderedValueType,
        quantity: OrderedValueType,
        handle_order_kind: HandleOrderKind,
    ) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Buy, NormalizedOrderType::StopLoss, quantity, price, None, false)?
        };

        self.create_order(order)
    }

    pub fn stoploss_market_sell(
        &mut self,
        symbol: Ustr,
        price: OrderedValueType,
        stop_price: OrderedValueType,
        quantity: OrderedValueType,
        handle_order_kind: HandleOrderKind,
    ) -> Result<(), TraderError> {
        let order = {
            let mut inner = self.0.read();
            let cx = inner.trader_symbol_contexts.get(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;
            let local_order_id = SmartId::new_order_id(0, handle_order_kind);

            self.order_template(symbol, local_order_id, NormalizedSide::Sell, NormalizedOrderType::StopLoss, quantity, price, None, false)?
        };

        self.create_order(order)
    }

    pub fn confirm_instruction_execution(&mut self, result: InstructionExecutionResult) -> Result<(), TraderError> {
        debug!("TRADER: Confirming instruction execution: {:#?}", result);
        // FIXME: remove this
        // eprintln!("result.metadata.local_order_id = {:#?}", SmartId(result.metadata.local_order_id.unwrap()).decode());

        let (handle_id, order_kind, sequence_number, unique_order_id) = SmartId(result.metadata.local_order_id.unwrap()).decode();

        // If the instruction is associated with a trade, then confirming the instruction execution
        // through the trade. Otherwise, the instruction is considered as a standalone instruction.
        match (result.symbol, result.metadata.local_order_id) {
            (Some(symbol), Some(local_order_id)) => {
                let mut inner = self.0.write();
                let cx = inner.trader_symbol_contexts.get_mut(&symbol).ok_or(TraderError::SymbolNotFound(symbol))?;

                if matches!(order_kind, HandleOrderKind::None | HandleOrderKind::UnexpectedOrder) {
                    warn!("{}: Ignoring instruction execution result: local_order_id={}", symbol, local_order_id);
                    // FIXME: Instruction must be confirmed no matter what
                    // FIXME: Instruction must be confirmed no matter what
                    // FIXME: Instruction must be confirmed no matter what
                    /*
                    match cx.orders.entry(local_order_id) {
                        indexmap::map::Entry::Occupied(mut entry) => {
                            entry.get_mut().confirm_instruction_execution(result)?;
                        }

                        indexmap::map::Entry::Vacant(mut entry) => {
                            error!("{}: Failed to find order by local_order_id={}", symbol, local_order_id);
                            return Err(TraderError::OrderError(OrderError::OrderNotFound(local_order_id)));
                        }
                    }
                     */
                } else {
                    let Some(trade_id) = result.metadata.trade_id else {
                        error!("{}: Trade ID is missing: local_order_id={}", symbol, local_order_id);
                        return Err(TraderError::TradeError(TradeError::TradeIdMissing(symbol, local_order_id)));
                    };

                    match cx.trades.entry(trade_id) {
                        indexmap::map::Entry::Occupied(mut entry) => {
                            entry.get_mut().confirm_instruction_execution(result)?;
                        }

                        indexmap::map::Entry::Vacant(mut entry) => {
                            error!("{}: Failed to find trade by trade_id={}", symbol, trade_id);
                            return Err(TraderError::TradeError(TradeError::TradeNotFound(symbol, trade_id)));
                        }
                    }
                }
            }

            _ => {
                return Err(TraderError::UnsupportedInstructionExecutionResult(result));
            }
        }

        self.compute()
    }

    pub fn confirm_order_open_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), TraderError> {
        info!("{}: Confirming order open failed: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_open_failed()?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_open_failed(local_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
            .confirm_open_failed(local_order_id)?;

        self.get_stats_mut().orders_failed_to_open += 1;
        self.compute()
    }

    pub fn confirm_order_opened(&mut self, symbol: Ustr, local_order_id: OrderId, exchange_order_id: Option<OrderId>) -> Result<(), TraderError> {
        info!("{}: Confirming order opened: local_order_id={} exchange_order_id={:?}", symbol, local_order_id, exchange_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_opened(exchange_order_id)?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_opened(local_order_id, exchange_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_opened += 1;
        self.compute()
    }

    pub fn confirm_order_filled(&mut self, symbol: Ustr, local_order_id: OrderId, last_fill: OrderFill) -> Result<(), TraderError> {
        info!("{}: Confirming order filled: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_filled(last_fill)?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_filled(local_order_id, last_fill)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_closed += 1;
        self.compute()
    }

    pub fn confirm_order_replace_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), TraderError> {
        info!("{}: Confirming order replace failed: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_replace_failed()?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_replace_failed(local_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_failed_to_replace += 1;
        self.compute()
    }

    pub fn confirm_order_replaced(
        &mut self,
        symbol: Ustr,
        local_original_order_id: OrderId,
        local_order_id: OrderId,
        event: NormalizedOrderEvent,
    ) -> Result<(), TraderError> {
        info!("{}: Confirming order replaced: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_replaced(event)?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                // WARNING: `original` in this case is actually a parent order that was replaced
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_replaced(local_original_order_id, event)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_replaced += 1;
        self.compute()
    }

    pub fn confirm_order_cancel_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), TraderError> {
        info!("{}: Confirming order cancel failed: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_cancel_failed()?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_cancel_failed(local_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_failed_to_cancel += 1;
        self.compute()
    }

    pub fn confirm_order_cancelled(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), TraderError> {
        info!("{}: Confirming order cancelled: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_cancelled()?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_cancelled(local_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_cancelled += 1;
        self.compute()
    }

    pub fn confirm_order_rejected(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), TraderError> {
        info!("{}: Confirming order rejected: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_rejected()?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_rejected(local_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_rejected += 1;
        self.compute()
    }

    pub fn confirm_order_expired(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), TraderError> {
        info!("{}: Confirming order expired: local_order_id={}", symbol, local_order_id);

        match SmartId(local_order_id).decode() {
            (0, _, _, _) => {
                self.get_order_mut(symbol, local_order_id)?.confirm_expired()?;
            }

            (trade_id, order_kind, sequence_number, unique_order_id) => {
                self.get_trade_mut(symbol, SmartId(local_order_id).handle_id())?
                    .confirm_expired(local_order_id)?;
            }

            _ => {
                warn!("{}: Unhandled SmartId combination: {:?}", symbol, SmartId(local_order_id).decode());
            }
        }

        self.get_stats_mut().orders_expired += 1;
        self.compute()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_test_setup, f, trade::TradeType};

    #[cfg(test)]
    mod tests {
        use super::*;
        use ustr::ustr;

        #[test]
        fn test_trades() {
            let (symbol, trader, price) = create_test_setup!("TESTCOIN", true);

            let atomic_market_price = Arc::new(AtomicF64::new(98.0));
            let timeframe = Timeframe::M1;
            let quantity = f!(0.01);
            let entry_price = f!(100.0);
            let exit_price = f!(110.0);
            let stoploss_price = f!(95.0);
            let stoploss_metadata = StoplossMetadata::new(true, entry_price, stoploss_price, f!(0.01));

            let trade_config =
                TradeBuilderConfig::long_with_exit_and_stop(symbol, atomic_market_price, timeframe, quantity, entry_price, exit_price, stoploss_metadata)
                    .unwrap();

            let trade = trader
                .trade_template(trade_config, entry_price, Some(exit_price), stoploss_metadata)
                .expect("failed to create trade");

            println!("{:#?}", trade);

            assert_eq!(trade.symbol, symbol);
            assert_eq!(trade.timeframe, timeframe);
            assert_eq!(trade.trade_type, TradeType::Long);

            assert_eq!(trade.entry.side, NormalizedSide::Buy);
            assert_eq!(trade.entry.initial_quantity, quantity);
            assert_eq!(trade.entry.price, entry_price);

            assert_eq!(trade.exit.price, exit_price);
            assert_eq!(trade.exit.side, NormalizedSide::Sell);
            assert_eq!(trade.exit.initial_quantity, trade.entry.initial_quantity);

            assert_eq!(trade.stoploss.price, stoploss_price);
            assert_eq!(trade.stoploss.side, NormalizedSide::Sell);
            assert_eq!(trade.stoploss.initial_quantity, trade.entry.initial_quantity);
        }
    }
}
