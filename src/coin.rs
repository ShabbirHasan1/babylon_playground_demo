use std::{
    borrow::BorrowMut,
    hash::{Hash, Hasher},
    ops::Index,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::position::Position;
use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use crossbeam::channel::Sender;
use indexmap::IndexMap;
use itertools::*;
use log::{error, info, warn};
use num::Float;
use num_traits::{ToPrimitive, Zero};
use ordered_float::OrderedFloat;
use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard};
use portable_atomic::AtomicF64;
use postgres::fallible_iterator::FallibleIterator;
use rand::Rng;
use yata::{
    core::{MovingAverage, MovingAverageConstructor, PeriodType, ValueType, Window},
    prelude::*,
};

use crate::{
    appraiser::Appraiser,
    balance::BalanceSheet,
    candle::Candle,
    candle_manager::CandleManager,
    coin_manager::CoinManager,
    coin_worker::CoinWorkerMessage,
    config::{CoinConfig, TimeframeConfig},
    context::ContextError,
    database::Database,
    exchange_client::ExchangeClientMessage,
    f,
    order::Order,
    orderbook::OrderBook,
    s2f,
    state_indicator::ComputedIndicatorState,
    state_moving_average::{ComputedMovingAverageState, FrozenMovingAverageState},
    support_resistance::SRLevel,
    timeframe::Timeframe,
    timeset::TimeSet,
    types::{DataPoint, ExchangeApi, NormalizedOrderType, NormalizedSide},
    util::{calc_price_distribution, calc_proportional_distribution, round_float_to_ordered_precision},
};

use crate::{
    balance::BalanceError,
    candle_series::{CandleSeries, CandleSeriesError, ComputedState},
    config::{Config, ConfigDefaults},
    cooldown::Cooldown,
    countdown::Countdown,
    database::DatabaseError,
    id::HandleOrderKind,
    instruction::InstructionExecutionResult,
    leverage::Leverage,
    model::{AccountInformation, Depth, ExchangeTradeEvent, Rules, Symbol},
    order::OrderError,
    ratelimiter::UnifiedRateLimiter,
    sliding_window::SlidingWindow,
    timeframe::TimeframeError,
    trader::{Trader, TraderError, TraderSymbolContext},
    trigger::{InstantTrigger, Trigger, TriggerError},
    trigger_distance::DistanceTrigger,
    types::{NormalizedCommission, OrderLifecycle, OrderedValueType, PriceDistribution, QuantityDistribution},
    util::{calc_percentage_of, calc_ratio_x_to_y},
};
use thiserror::Error;
use ustr::{ustr, Ustr};
use yata::core::Action;

#[derive(Error, Debug)]
pub enum CoinError {
    #[error("State not found: timeframe={0:?}, index={1}")]
    StateNotFound(Timeframe, PeriodType),

    #[error("Candle not found: timeframe={0:?}, index={1}")]
    CandleNotFound(Timeframe, PeriodType),

    #[error("Lower price is higher than upper price: lower={0}, upper={1}")]
    LowerPriceHigherThanUpperPrice(OrderedValueType, OrderedValueType),

    #[error(transparent)]
    TimeframeError(#[from] TimeframeError),

    #[error(transparent)]
    DatabaseError(#[from] DatabaseError),

    #[error(transparent)]
    OrderError(#[from] OrderError),

    #[error(transparent)]
    TraderError(#[from] TraderError),

    #[error(transparent)]
    TriggerError(#[from] TriggerError),

    #[error(transparent)]
    BalanceError(#[from] BalanceError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[macro_export]
macro_rules! peek_state {
    ($state:ident, $frozen_state:ident, $field:ident, $val:expr) => {
        if let Some((ma_type, name, period, instance)) = $frozen_state.$field.as_mut() {
            $state.$field = Some(SRLevel::new(ma_type, name, *period, OrderedFloat(instance.peek_next($val))));
        }
    };
}

#[macro_export]
macro_rules! update_state {
    ($state:ident, $frozen_state:ident, $field:ident, $val:expr) => {
        if let Some((ma_type, name, period, instance)) = $frozen_state.$field.as_mut() {
            $state.$field = Some(SRLevel::new(ma_type, name, *period, OrderedFloat(instance.next($val))));
        }
    };
}

pub struct CoinContext {
    pub symbol:            Ustr,
    pub account_info:      Arc<AccountInformation>,
    pub config:            CoinConfig,
    pub timeframe_configs: TimeSet<TimeframeConfig>,
    pub symbol_metadata:   Symbol,
    pub rules:             Rules,
    pub appraiser:         Appraiser,
    pub atomic_price:      Arc<AtomicF64>,
    pub balance:           BalanceSheet,
}

pub struct Coin {
    pub coin_context:           Arc<RwLock<CoinContext>>,
    pub orderbook:              OrderBook,
    pub series:                 TimeSet<CandleSeries>,
    pub trades:                 SlidingWindow<ExchangeTradeEvent>, // FIXME: send this to the event logger
    pub candle_manager:         CandleManager,
    pub quote_asset:            Ustr,
    pub base_asset:             Ustr,
    pub trade_timeframe:        Timeframe,
    pub ratelimiter:            UnifiedRateLimiter,
    pub manager:                Arc<RwLock<CoinManager>>,
    pub trader:                 Trader,
    pub exchange_client_sender: Sender<ExchangeClientMessage>,
    pub coin_worker_sender:     Sender<CoinWorkerMessage>,
    pub datapoint_sender:       Sender<DataPoint>,
    pub buy_cooldown:           Cooldown,
    pub sell_cooldown:          Cooldown,
    pub uptime:                 Instant,
    pub is_loaded:              bool,
}

impl Coin {
    pub fn new(
        config: Config,
        config_defaults: ConfigDefaults,
        timeframe_configs: TimeSet<TimeframeConfig>,
        timeframes: Vec<Timeframe>,
        coin_config: CoinConfig,
        symbol_metadata: Symbol,
        account: Arc<AccountInformation>,
        db: Database,
        exchange_api: ExchangeApi,
        initial_orderbook_state: Depth,
        balance: BalanceSheet,
        ratelimiter: UnifiedRateLimiter,
        manager: Arc<RwLock<CoinManager>>,
        trader: Trader,
        series: TimeSet<CandleSeries>,
        candle_manager: CandleManager,
        exchange_client_sender: Sender<ExchangeClientMessage>,
        coin_worker_sender: Sender<CoinWorkerMessage>,
        datapoint_sender: Sender<DataPoint>,
    ) -> Result<Arc<RwLock<Self>>> {
        // FIXME: obtain `main_signal_timeframe` from the coin config
        let trade_timeframe = coin_config.trade_timeframe.unwrap_or(config_defaults.trade_timeframe);

        info!("{}: Initializing coin", symbol_metadata.symbol);
        info!("{}: Trade Timeframe: {}", symbol_metadata.symbol, trade_timeframe);

        let symbol = ustr(symbol_metadata.symbol.as_str());
        let quote_asset = symbol_metadata.quote_asset;
        let base_asset = symbol_metadata.base_asset;
        let rules = symbol_metadata.rules.clone();
        let head_candle = series
            .get(trade_timeframe)?
            .head_candle()
            .ok_or(CoinError::CandleNotFound(trade_timeframe, 0))?;
        let price = Arc::new(AtomicF64::new(head_candle.close));
        let appraiser = Appraiser::new(rules);
        let orderbook = OrderBook::new(symbol, appraiser, initial_orderbook_state.last_update_id, initial_orderbook_state.bids, initial_orderbook_state.asks);
        let candle_manager = CandleManager::new(coin_config.clone(), config_defaults.clone(), timeframe_configs.clone(), db, exchange_api, ratelimiter.clone());

        // IMPORTANT: Initializing the coin context in the Trader mechanism
        // NOTE: `is_zero_maker_fee` and `is_zero_taker_fee` are set to `false` because we don't know the actual fee rates yet
        // NOTE: Each coin may have some setting overrides, so we need to initialize context with respect to fallbacks

        let entry_cooldown_ms = coin_config.entry_cooldown_ms.unwrap_or(config_defaults.entry_cooldown_ms);
        let exit_cooldown_ms = coin_config.exit_cooldown_ms.unwrap_or(config_defaults.exit_cooldown_ms);
        let max_quote_allocated = coin_config.max_quote_allocated.unwrap_or(config_defaults.max_quote_allocated);
        let max_orders_per_cooldown = coin_config.max_orders_per_cooldown.unwrap_or(config_defaults.max_orders_per_cooldown);
        let min_quote_per_grid = coin_config.min_quote_per_grid.unwrap_or(config_defaults.min_quote_per_grid);
        let max_quote_per_grid = coin_config.max_quote_per_grid.unwrap_or(config_defaults.max_quote_per_grid);
        let max_orders = coin_config.max_orders.unwrap_or(config_defaults.max_orders);
        let max_orders_per_trade = coin_config.max_orders_per_trade.unwrap_or(config_defaults.max_orders_per_trade);
        let max_orders_per_cooldown = coin_config.max_orders_per_cooldown.unwrap_or(config_defaults.max_orders_per_cooldown);
        let max_retry_attempts = coin_config.max_retry_attempts.unwrap_or(config_defaults.max_retry_attempts);
        let retry_delay_ms = coin_config.retry_delay_ms.unwrap_or(config_defaults.retry_delay_ms);
        let entry_grid_orders = coin_config.entry_grid_orders.unwrap_or(config_defaults.entry_grid_orders);
        let exit_grid_orders = coin_config.exit_grid_orders.unwrap_or(config_defaults.exit_grid_orders);
        let min_order_spacing_ratio = coin_config.min_order_spacing_ratio.unwrap_or(config_defaults.min_order_spacing_ratio);
        let initial_stop_ratio = coin_config.initial_stop_ratio.unwrap_or(config_defaults.initial_stop_ratio);
        let stop_limit_offset_ratio = coin_config.stop_limit_offset_ratio.unwrap_or(config_defaults.stop_limit_offset_ratio);
        let initial_trailing_step_ratio = coin_config.initial_trailing_step_ratio.unwrap_or(config_defaults.initial_trailing_step_ratio);
        let trailing_step_ratio = coin_config.trailing_step_ratio.unwrap_or(config_defaults.trailing_step_ratio);
        let baseline_risk_ratio = coin_config.baseline_risk_ratio.unwrap_or(config_defaults.baseline_risk_ratio);
        let is_zero_maker_fee = coin_config.is_zero_maker_fee.unwrap_or(config_defaults.is_zero_maker_fee);
        let is_zero_taker_fee = coin_config.is_zero_taker_fee.unwrap_or(config_defaults.is_zero_taker_fee);
        let is_iceberg_trading_enabled = coin_config.is_iceberg_trading_enabled.unwrap_or(config_defaults.is_iceberg_trading_enabled);
        let is_long_trading_enabled = coin_config.is_long_trading_enabled.unwrap_or(config_defaults.is_long_trading_enabled);
        let is_short_trading_enabled = coin_config.is_short_trading_enabled.unwrap_or(config_defaults.is_short_trading_enabled);

        // Calculating commission rates
        let mut commission = NormalizedCommission::default();

        if !account.maker_commission.is_zero() {
            commission.maker = Some(account.commission_rates.maker);
        }

        if !account.taker_commission.is_zero() {
            commission.taker = Some(account.commission_rates.taker);
        }

        if !account.buyer_commission.is_zero() {
            commission.buyer = Some(account.commission_rates.buyer);
        }

        if !account.seller_commission.is_zero() {
            commission.seller = Some(account.commission_rates.seller);
        }

        info!("{}: Creating trading context", symbol);
        trader.create_context(TraderSymbolContext {
            symbol,
            quote_asset,
            base_asset,
            atomic_market_price: price.clone(),
            entry_cooldown: Cooldown::new(Duration::from_millis(entry_cooldown_ms as u64), max_orders_per_cooldown as u32),
            exit_cooldown: Cooldown::new(Duration::from_millis(exit_cooldown_ms as u64), max_orders_per_cooldown as u32),
            appraiser,
            commission,
            max_quote_allocated,
            min_quote_per_grid,
            max_quote_per_grid,
            max_orders,
            max_orders_per_trade,
            max_orders_per_cooldown,
            max_retry_attempts,
            retry_delay: Duration::from_millis(retry_delay_ms as u64),
            entry_grid_orders,
            exit_grid_orders,
            min_order_spacing_ratio,
            initial_stop_ratio,
            stop_limit_offset_ratio,
            initial_trailing_step_ratio,
            trailing_step_ratio,
            baseline_risk_ratio,
            position: Position::new(symbol, balance.clone()),
            orders: Default::default(),
            trades: IndexMap::new(),
            is_zero_maker_fee,
            is_zero_taker_fee,
            is_paused: false,
            is_iceberg_allowed_by_exchange: symbol_metadata.iceberg_allowed,
            is_iceberg_trading_enabled,
            is_spot_trading_allowed_by_exchange: symbol_metadata.is_spot_trading_allowed,
            is_margin_trading_allowed_by_exchange: symbol_metadata.is_margin_trading_allowed,
            is_long_trading_enabled,
            is_short_trading_enabled,
            stats: Default::default(),
        })?;

        // TODO: Load position information from the database

        info!("{}: Initializing coin context", symbol);
        // NOTE: This context may be shared across multiple threads, so we need to wrap it in an `Arc<RwLock<_>>`.
        let cx = Arc::new(RwLock::new(CoinContext {
            account_info: account,
            symbol,
            config: coin_config.clone(),
            timeframe_configs: timeframe_configs.clone(),
            symbol_metadata: symbol_metadata.clone(),
            rules: rules.clone(),
            appraiser: appraiser.clone(),
            atomic_price: price.clone(),
            balance: balance.clone(),
        }));

        info!("{}: Initializing the coin", symbol);
        let mut coin = Arc::new(RwLock::new(Self {
            coin_context: cx,
            candle_manager,
            quote_asset,
            base_asset,
            orderbook,
            // trades: Window::new(config.main.trade_window_size, ExchangeTradeEvent::default()),
            trades: SlidingWindow::<ExchangeTradeEvent>::new(config.main.trade_window_size),
            series,
            trade_timeframe,
            ratelimiter,
            manager: manager.clone(),
            datapoint_sender,
            buy_cooldown: Cooldown::new(Duration::from_secs(1), 1),
            sell_cooldown: Cooldown::new(Duration::from_secs(1), 1),
            uptime: Instant::now(),
            exchange_client_sender,
            coin_worker_sender,
            is_loaded: false,
            trader,
        }));

        // NOTE: We need to initialize the coin's state after we've created the coin, because the coin's state depends on the coin itself.
        // FIXME: probably this is no longer needed
        // coin.write().compute_all_states()?;

        info!("{}: Registering coin in the shared context", symbol);
        manager.write().register_coin(symbol, coin.clone())?;

        Ok(coin)
    }

    pub fn context(&self) -> MappedRwLockReadGuard<CoinContext> {
        RwLockReadGuard::map(self.coin_context.read(), |cx| cx)
    }

    pub fn symbol(&self) -> Ustr {
        self.context().symbol
    }

    pub fn appraiser(&self) -> Appraiser {
        self.context().appraiser
    }

    pub fn price(&self) -> OrderedValueType {
        f!(self.context().atomic_price.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn set_price(&self, price: OrderedValueType) {
        // TODO: Decide whether the price should be rounded
        let price = self.context().appraiser.round_quote(price.0);
        self.context().atomic_price.store(price, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn set_trade_timeframe(&mut self, timeframe: Timeframe) {
        self.trade_timeframe = timeframe;
    }

    #[inline]
    pub fn head_state(&self, timeframe: Timeframe) -> Result<&ComputedState, CoinError> {
        self.series.get(timeframe)?.head_state().ok_or(CoinError::StateNotFound(timeframe, 0))
    }

    #[inline]
    pub fn head_state_mut(&mut self, timeframe: Timeframe) -> Result<&mut ComputedState, CoinError> {
        self.series.get_mut(timeframe)?.head_state_mut().ok_or(CoinError::StateNotFound(timeframe, 0))
    }

    #[inline]
    pub fn working_candle(&self, timeframe: Timeframe) -> Result<&Candle, CoinError> {
        self.series.get(timeframe)?.head_candle().ok_or(CoinError::CandleNotFound(timeframe, 0))
    }

    #[inline]
    pub fn working_candle_mut(&mut self, timeframe: Timeframe) -> Result<&mut Candle, CoinError> {
        self.series.get_mut(timeframe)?.head_candle_mut().ok_or(CoinError::CandleNotFound(timeframe, 0))
    }

    #[inline]
    pub fn push_instruction_result(&mut self, result: InstructionExecutionResult) -> Result<()> {
        self.trader.confirm_instruction_execution(result)?;
        Ok(())
    }

    #[inline]
    pub fn push_candle(&mut self, c: Candle) -> Result<()> {
        let series = self.series.get_mut(c.timeframe)?;

        match series.push_candle(c) {
            Ok(Some(cs)) => {
                // if series.is_live && series.timeframe() == self.trade_timeframe {
                //     println!("push_candle: {:#?}", cs);
                // }
                self.compute(c.timeframe, cs)?
            }
            Ok(None) => {}
            Err(err) => {
                panic!("{}: failed to push candle: {:?}", self.symbol(), err);
            }
        }

        // IMPORTANT: Computing trader state
        self.trader.compute().expect("failed to compute trader state");

        Ok(())
    }

    #[inline]
    pub fn push_trade(&mut self, trade: ExchangeTradeEvent) -> Result<()> {
        let active_timeframes = self.context().timeframe_configs.iter().map(|x| x.0).collect_vec();
        self.coin_context.write().atomic_price.store(trade.price.0, std::sync::atomic::Ordering::SeqCst);

        // Applying the trade to all active timeframes
        for timeframe in active_timeframes {
            let series = self.series.get_mut(timeframe)?;

            match series.apply_trade(trade) {
                Ok(Some(cs)) => {
                    // if series.is_live && series.timeframe() == self.trade_timeframe {
                    //     println!("push_trade: {:#?}", cs.candle);
                    // }
                    self.compute(timeframe, cs)?
                }
                Ok(None) => {}
                Err(err) => {}
            }
        }

        // IMPORTANT: Computing trader state
        self.trader.compute()?;

        // Pushing the trade to the trade window
        self.trades.push(trade);

        Ok(())
    }

    pub fn update_orderbook_partial_20(&mut self, last_update_id: u64, bids: [[OrderedValueType; 2]; 20], asks: [[OrderedValueType; 2]; 20]) -> Result<()> {
        self.orderbook.0.write().update_partial_20(self.price(), last_update_id, bids, asks)
    }

    pub fn update_orderbook_diff(&mut self, last_update_id: u64, bids: Vec<[OrderedValueType; 2]>, asks: Vec<[OrderedValueType; 2]>) -> Result<()> {
        self.orderbook.0.write().update_diff(self.price(), last_update_id, bids, asks)
    }

    // TODO: when candles are downloaded or gaps filled later, is_final could be false, fix it
    pub fn compute(&mut self, timeframe: Timeframe, cs: ComputedState) -> Result<(), CoinError> {
        // ----------------------------------------------------------------------------
        // NOTE:
        // The following functions mutate the state of the timeframe but do not
        // rotate it when the candle is final, it must be done here, after
        // the trade action is computed
        // ----------------------------------------------------------------------------

        // Setting the price to be accessible globally in the system
        // TODO: consider removing this, do I need this?
        self.set_price(self.series.get(timeframe)?.price());

        // TODO: ...

        // Computing trade action after latest computations on the main signal timeframe
        if timeframe == self.trade_timeframe {
            let trade_action_computation_time = Instant::now();
            if let Err(err) = self.compute_trade_action(cs) {
                // TODO: Return the error instead of logging it and handle it properly in the caller
                warn!("{}: failed to compute trade action: {:?}", self.symbol(), err);
            }
            let trade_action_computation_time = trade_action_computation_time.elapsed();
        }

        // TODO: ...

        Ok(())
    }

    pub fn compute_trade_action(&mut self, cs: ComputedState) -> Result<(), CoinError> {
        let series = self.series.get(self.trade_timeframe)?;

        // TODO: I think it makes sense to confirm that the candle is live by comparing the time of the candle with the current time
        // WARNING: If the series is not live, trade action MUST NOT be computed
        if !series.is_live {
            return Ok(());
        }

        let hc = series.head_candle().ok_or(CoinError::CandleNotFound(self.trade_timeframe, 0))?;
        let pc = series.get_candle(1).ok_or(CoinError::CandleNotFound(self.trade_timeframe, 0))?;
        let pcs = series.get_state(1).ok_or(CoinError::StateNotFound(self.trade_timeframe, 0))?;
        let appraiser = self.appraiser();
        let price = round_float_to_ordered_precision(self.price().0, appraiser.true_quote_precision);
        let quote_balance = self.coin_context.read().balance.get(self.quote_asset)?;
        let base_balance = self.coin_context.read().balance.get(self.base_asset)?;

        // ...

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        helpers::symbol_for_testing,
        id::{HandleOrderKind, SmartId},
        model::{IcebergPartsRule, LotSizeRule, MarketLotSizeRule, MaxNumAlgoOrdersRule, MaxNumOrdersRule, NotionalRule, PriceFilterRule, TrailingDeltaRule},
    };
    use ustr::ustr;

    #[test]
    fn test_spread_buy_orders() {
        let appraiser = Appraiser::new(symbol_for_testing().rules);

        let position_id = SmartId::new_handle_id();
        let qty = OrderedFloat(0.00860);
        let lower_price = 16823.06;
        let upper_price = lower_price + (lower_price * 0.0002);
        let number_of_orders = 3;
        let price_dist = PriceDistribution::Geometric;
        let qty_dist = QuantityDistribution::Degressive;

        if lower_price >= upper_price {
            panic!("lower_price cannot be >= upper_price when selling: lower_price={} upper_price={}", lower_price, upper_price);
        }

        // TODO: tell trader to pause trading for X amount of time

        let orders = calc_price_distribution(price_dist, lower_price, upper_price, number_of_orders)
            .into_iter()
            .zip(calc_proportional_distribution(qty.0, qty_dist, number_of_orders))
            .map(|(price, qty)| Order {
                local_order_id: SmartId::new_order_id(position_id, HandleOrderKind::EntryOrder),
                raw_order_id: None,
                local_stop_order_id: None,
                local_orig_order_id: None,
                local_list_order_id: None,
                exchange_order_id: None,
                symbol: ustr("TESTCOIN"),
                side: NormalizedSide::Buy,
                timeframe: Timeframe::M1,
                atomic_market_price: Default::default(),
                appraiser,
                order_type: NormalizedOrderType::LimitMaker,
                price: round_float_to_ordered_precision(price, 2),
                limit_price: Default::default(),
                accumulated_filled_quantity: Default::default(),
                accumulated_filled_quote: Default::default(),
                fee_ratio: None,
                retry_counter: Countdown::new(2),
                retry_cooldown: Cooldown::new(Duration::from_millis(300), 1),
                lifecycle: OrderLifecycle::default(),
                leverage: Leverage::no_leverage(),
                state: Default::default(),
                termination_flag: false,
                cancel_and_idle_flag: false,
                pending_instruction: None,
                special_pending_instruction: None,
                metadata: Default::default(),
                initialized_at: Default::default(),
                opened_at: None,
                updated_at: None,
                trailing_activated_at: None,
                trailing_updated_at: None,
                closed_at: None,
                cancelled_at: None,
                activator: Box::new(InstantTrigger::new()),
                deactivator: Box::new(DistanceTrigger::new(OrderedFloat(0.0), OrderedFloat(0.10)).unwrap()),
                market_price_at_initialization: None,
                initial_quantity: Default::default(),
                trade_id: 0,
                mode: Default::default(),
                fills: Default::default(),
                num_updates: 0,
                market_price_at_opening: None,
                finalized_at: None,
                contingency_order_type: Some(NormalizedOrderType::Market),
                is_created_on_exchange: false,
                time_in_force: Default::default(),
                pending_mode: None,
                cancel_and_replace_flag: false,
                is_finished_by_cancel: false,
                is_entry_order: true,
            })
            .collect_vec();

        eprintln!("qty = {:?}", qty);
        eprintln!("lower_price = {:?}", lower_price);
        eprintln!("upper_price = {:?}", upper_price);
        eprintln!("number_of_orders = {:?}", number_of_orders);
        eprintln!("price_dist = {:?}", price_dist);
        eprintln!("qty_dist = {:?}", qty_dist);
        println!();

        println!("{:#?}", orders);
        println!("{:?}", orders.iter().fold(OrderedFloat(0.0), |acc, b| acc + b.initial_quantity));
    }

    #[test]
    fn test_spread_sell_orders() {
        let appraiser = Appraiser::new(symbol_for_testing().rules);

        let position_id = SmartId::new_handle_id();
        let qty = OrderedFloat(0.00860);
        let lower_price = 16823.06;
        let upper_price = lower_price + (lower_price * 0.0002);
        let number_of_orders = 3;
        let price_dist = PriceDistribution::Geometric;
        let qty_dist = QuantityDistribution::Progressive;

        if lower_price >= upper_price {
            panic!("lower_price cannot be >= upper_price when selling: lower_price={} upper_price={}", lower_price, upper_price);
        }

        // TODO: tell trader to pause trading for X amount of time

        let orders = calc_price_distribution(price_dist, lower_price, upper_price, number_of_orders)
            .into_iter()
            .zip(calc_proportional_distribution(qty.0, qty_dist, number_of_orders))
            .map(|(price, qty)| Order {
                local_order_id: SmartId::new_order_id(position_id, HandleOrderKind::EntryOrder),
                raw_order_id: None,
                local_stop_order_id: None,
                local_orig_order_id: None,
                symbol: ustr("TESTCOIN"),
                side: NormalizedSide::Sell,
                timeframe: Timeframe::M1,
                atomic_market_price: Default::default(),
                appraiser,
                order_type: NormalizedOrderType::LimitMaker,
                price: round_float_to_ordered_precision(price, 2),
                limit_price: Default::default(),
                initial_quantity: Default::default(),
                accumulated_filled_quantity: Default::default(),
                accumulated_filled_quote: Default::default(),
                fee_ratio: None,
                retry_counter: Countdown::new(2),
                retry_cooldown: Cooldown::new(Duration::from_millis(300), 1),
                lifecycle: OrderLifecycle::default(),
                leverage: Leverage::no_leverage(),
                state: Default::default(),
                termination_flag: false,
                cancel_and_idle_flag: false,
                pending_instruction: None,
                special_pending_instruction: None,
                metadata: Default::default(),
                initialized_at: Default::default(),
                opened_at: None,
                updated_at: None,
                trailing_activated_at: None,
                trailing_updated_at: None,
                closed_at: None,
                cancelled_at: None,
                activator: Box::new(InstantTrigger::new()),
                deactivator: Box::new(DistanceTrigger::new(OrderedFloat(0.0), OrderedFloat(0.10)).unwrap()),
                market_price_at_initialization: None,
                trade_id: 0,
                mode: Default::default(),
                fills: Default::default(),
                num_updates: 0,
                market_price_at_opening: None,
                finalized_at: None,
                exchange_order_id: None,
                contingency_order_type: Some(NormalizedOrderType::Market),
                is_created_on_exchange: false,
                local_list_order_id: None,
                time_in_force: Default::default(),
                pending_mode: None,
                cancel_and_replace_flag: false,
                is_finished_by_cancel: false,
            })
            .collect_vec();

        eprintln!("qty = {:?}", qty);
        eprintln!("lower_price = {:?}", lower_price);
        eprintln!("upper_price = {:?}", upper_price);
        eprintln!("number_of_orders = {:?}", number_of_orders);
        eprintln!("price_dist = {:?}", price_dist);
        eprintln!("qty_dist = {:?}", qty_dist);
        println!();

        println!("{:#?}", orders);
        println!("{:?}", orders.iter().fold(OrderedFloat(0.0), |acc, b| acc + b.initial_quantity));
    }

    #[test]
    fn test_rules_and_appraiser() {
        let rules = Symbol {
            symbol:                              ustr("BTCUSDT"),
            status:                              ustr("TRADING"),
            base_asset:                          ustr("BTC"),
            base_asset_precision:                8,
            quote_asset:                         ustr("USDT"),
            quote_precision:                     8,
            quote_asset_precision:               8,
            base_commission_precision:           8,
            quote_commission_precision:          8,
            order_types:                         vec![
                NormalizedOrderType::Limit,
                NormalizedOrderType::LimitMaker,
                NormalizedOrderType::Market,
                NormalizedOrderType::StopLossLimit,
                NormalizedOrderType::TakeProfitLimit,
            ],
            iceberg_allowed:                     true,
            oco_allowed:                         true,
            quote_order_qty_market_allowed:      true,
            allow_trailing_stop:                 true,
            cancel_replace_allowed:              true,
            rules:                               Rules {
                price_filter:           Some(PriceFilterRule {
                    min_price: OrderedFloat(0.01),
                    max_price: OrderedFloat(1000000.0),
                    tick_size: OrderedFloat(0.01),
                }),
                percent_price:          None,
                lot_size:               Some(LotSizeRule {
                    min_qty:   OrderedFloat(1e-5),
                    max_qty:   OrderedFloat(9000.0),
                    step_size: OrderedFloat(1e-5),
                }),
                market_lot_size:        Some(MarketLotSizeRule {
                    min_qty:   OrderedFloat(0.0),
                    max_qty:   OrderedFloat(39.53598857),
                    step_size: OrderedFloat(0.0),
                }),
                min_notional:           None,
                notional:               Some(NotionalRule {
                    min_notional:        OrderedFloat(5.0),
                    apply_min_to_market: true,
                    max_notional:        OrderedFloat(9000000.0),
                    apply_max_to_market: false,
                    avg_price_mins:      5,
                }),
                iceberg_parts:          Some(IcebergPartsRule { limit: 10 }),
                max_num_iceberg_orders: None,
                max_num_orders:         Some(MaxNumOrdersRule { max_num_orders: 200 }),
                max_num_algo_orders:    Some(MaxNumAlgoOrdersRule { max_num_algo_orders: 5 }),
                max_position:           None,
                trailing_delta:         Some(TrailingDeltaRule {
                    min_trailing_above_delta: 10,
                    max_trailing_above_delta: 2000,
                    min_trailing_below_delta: 10,
                    max_trailing_below_delta: 2000,
                }),
            },
            is_spot_trading_allowed:             true,
            is_margin_trading_allowed:           true,
            permissions:                         vec![
                ustr("SPOT"),
                ustr("MARGIN"),
                ustr("TRD_GRP_004"),
                ustr("TRD_GRP_005"),
                ustr("TRD_GRP_009"),
                ustr("TRD_GRP_010"),
                ustr("TRD_GRP_011"),
                ustr("TRD_GRP_012"),
                ustr("TRD_GRP_013"),
                ustr("TRD_GRP_014"),
                ustr("TRD_GRP_015"),
                ustr("TRD_GRP_016"),
                ustr("TRD_GRP_017"),
                ustr("TRD_GRP_018"),
                ustr("TRD_GRP_019"),
                ustr("TRD_GRP_020"),
                ustr("TRD_GRP_021"),
                ustr("TRD_GRP_022"),
                ustr("TRD_GRP_023"),
                ustr("TRD_GRP_024"),
                ustr("TRD_GRP_025"),
            ],
            default_self_trade_prevention_mode:  ustr("EXPIRE_MAKER"),
            allowed_self_trade_prevention_modes: vec![ustr("EXPIRE_TAKER"), ustr("EXPIRE_MAKER"), ustr("EXPIRE_BOTH")],
        };

        /*
        let rules = Rules {
            base_precision:            8,
            quote_precision:           8,
            price_max:                 OrderedFloat(1000000.0),
            price_min:                 OrderedFloat(0.01),
            price_tick_size:           OrderedFloat(0.01),
            bid_percent_mult_up:       OrderedFloat(0.0),
            bid_percent_mult_down:     OrderedFloat(0.0),
            ask_percent_mult_up:       OrderedFloat(0.0),
            ask_percent_mult_down:     OrderedFloat(0.0),
            percent_avg_price_mins:    5,
            quantity_min:              OrderedFloat(0.0),
            quantity_max:              OrderedFloat(34.67080092),
            quantity_step_size:        OrderedFloat(0.0),
            market_quantity_min:       OrderedFloat(5.0),
            market_quantity_max:       OrderedFloat(9000000.0),
            market_quantity_step_size: OrderedFloat(0.0),
            iceberg_parts_limit:       10,
            max_num_orders:            200,
            max_num_algo_orders:       5,
            max_num_iceberg_orders:    0,
            max_position:              OrderedFloat(0.0),
            apply_max_to_market:       false,
            min_trailing_above_delta:  10,
            max_trailing_above_delta:  2000,
            min_trailing_below_delta:  10,
            max_trailing_below_delta:  2000,
            min_notional:              Default::default(),
            max_notional:              Default::default(),
        };
         */

        // let appraiser = Appraiser::new(rules);
        //
        // eprintln!("rules = {:#?}", rules);
        // eprintln!("appraiser = {:#?}", appraiser);
    }
}
