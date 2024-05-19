use crate::{
    countdown::Countdown,
    f,
    id::{HandleId, HandleOrderKind, InstructionId, OrderId, SmartId},
    instruction::{
        ExecutionOutcome, Instruction, InstructionError, InstructionExecutionResult, InstructionKind, InstructionMetadata, InstructionState, ManagedInstruction,
    },
    leverage::Leverage,
    order::{Order, OrderError, OrderFill, OrderFillStatus, OrderMode, OrderState, StoplossMetadata},
    plot_action::{PlotActionError, PlotActionInput, PlotActionInputType},
    timeframe::Timeframe,
    trade_closer::{Closer, CloserError, CloserKind, CloserMetadata},
    trade_opener::{NoOpener, Opener},
    trigger::{Trigger, TriggerError, TriggerKind, TriggerMetadata},
    trigger_proximity::ProximityTrigger,
    types::{NormalizedOrderEvent, NormalizedOrderStatus, NormalizedOrderType, NormalizedSide, NormalizedTimeInForce, OrderedValueType, Priority, Quantity},
    util::new_id_u64,
};
use chrono::{DateTime, Utc};
use log::{error, info, warn};
use num_traits::Zero;
use ordered_float::OrderedFloat;
use portable_atomic::AtomicF64;
use std::{
    fmt::{Display, Formatter},
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use thiserror::Error;
use ustr::Ustr;

#[derive(Debug, Clone, Eq, PartialEq, Error)]
pub enum TradeError {
    #[error("Trade is is not repaid: symbol={0}, trade_id{1}")]
    LeveragedTradeNotRepaid(Ustr, HandleId),

    #[error("Trade is not leveraged but marked as repaid: {0}")]
    NonLeveragedTradeRepaid(Ustr, HandleId),

    #[error("Trade not found: symbol={0} trade_id{1}")]
    TradeNotFound(Ustr, HandleId),

    #[error("Trade ID already exists: {0}")]
    TradeIdAlreadyExists(HandleId),

    #[error("Cannot create trade from non-zero trade id: symbol={0}, order_id={1}, trade_id={2}")]
    CannotCreateFromNonZeroTradeId(Ustr, OrderId, HandleId),

    #[error("Trade is locked: symbol={0}, trade_id={1}")]
    TradeIsLocked(Ustr, HandleId),

    #[error("Trade is closed: symbol={0}, trade_id={1}")]
    TradeIsAlreadyClosed(Ustr, HandleId),

    #[error("Order is not finalized: {0:?}")]
    OrderNotFinalized(OrderId),

    #[error("Failed to process finalized order: order_id={0}")]
    FailedToProcessFinalizedOrder(OrderId),

    #[error("Trade is not ready for closing: symbol={0}, trade_id={1}, reason={2}")]
    TradeNotReadyForClosing(Ustr, HandleId, String),

    #[error("Pending instruction not found: symbol={0}, instruction_id={1}")]
    PendingInstructionNotFound(Ustr, InstructionId),

    #[error("Trade cannot be closed: symbol={0}, trade_id={1}")]
    CannotBeClosed(Ustr, HandleId),

    #[error("Trade is already locked: symbol={0}, trade_id={1}")]
    AlreadyLocked(Ustr, HandleId),

    #[error("Trade cannot be locked with pending orders: symbol={0}, trade_id={1}, num_orders_pending={2}")]
    CannotLockWithPendingOrders(Ustr, HandleId, usize),

    #[error("Trade cannot be locked: symbol={0}, trade_id={1}, reason={2}")]
    CannotBeLocked(Ustr, HandleId, String),

    #[error("Trade mismatch: symbol={0}, trade_id={1}, order_trade_id={2}")]
    TradeMismatch(Ustr, HandleId, HandleId),

    #[error("Trade ID is zero: {0}")]
    TradeIdIsZero(Ustr),

    #[error("Invalid entry side: symbol={0}, trade_id={1}, trade_side={2:?}, order_id={3}, order_side={4:?}")]
    TradeEntryOrderSideMismatch(Ustr, HandleId, TradeType, OrderId, NormalizedSide),

    #[error("Invalid exit side: symbol={0}, trade_id={1}, trade_side={2:?}, order_id={3}, order_side={4:?}")]
    TradeExitSideMismatch(Ustr, HandleId, TradeType, OrderId, NormalizedSide),

    #[error("Invalid stoploss side: symbol={0}, trade_id={1}, trade_side={2:?}, order_id={3}, order_side={4:?}")]
    PositioStoplossSideMismatch(Ustr, HandleId, TradeType, OrderId, NormalizedSide),

    #[error("Invalid timeframe: symbol={0}, trade_id={1}, trade_timeframe={2:?}, order_timeframe={3:?}")]
    TradeTimeframeMismatch(Ustr, HandleId, Timeframe, Timeframe),

    #[error("Symbol is empty")]
    SymbolIsEmpty,

    #[error("Order not found: symbol={0}, trade_id={1}, order_id={2}")]
    OrderNotFound(Ustr, HandleId, OrderId),

    #[error("Closer is missing")]
    CloserIsMissing,

    #[error("Stoploss is missing")]
    StoplossIsMissing,

    #[error("Entry order is missing")]
    EntryOrderIsMissing,

    #[error("Exit order is missing")]
    ExitOrderIsMissing,

    #[error("Stoploss order is missing")]
    StoplossOrderIsMissing,

    #[error("Atomic market price is missing")]
    AtomicMarketPriceIsMissing,

    #[error("Atomic market price is zero")]
    AtomicMarketPriceIsZero,

    #[error("Invalid entry order type: {0:?}")]
    InvalidEntryOrderType(NormalizedOrderType),

    #[error("Invalid exit order type: {0:?}")]
    InvalidExitOrderType(NormalizedOrderType),

    #[error("Invalid stoploss order type: {0:?}")]
    InvalidStoplossOrderType(NormalizedOrderType),

    #[error("Missing entry order")]
    MissingEntryOrder,

    #[error("Missing exit order")]
    MissingExitOrder,

    #[error("Missing stoploss order")]
    MissingStoplossOrder,

    #[error("Trade is not ready: {0}")]
    TradeNotReady(Ustr, HandleId, TradeState),

    #[error("Invalid timeframe: {0:?}")]
    InvalidTimeframe(Timeframe),

    #[error("Trade has no pending instruction")]
    TradeHasNoPendingInstruction,

    #[error("Exit order instruction is missing")]
    ExitOrderInstructionIsMissing,

    #[error("Stoploss order instruction is missing")]
    StoplossOrderInstructionIsMissing,

    #[error("Entry order instruction is expected but missing")]
    EntryOrderInstructionIsExpectedButMissing,

    #[error("Invalid trade state transition: symbol={symbol}, trade_id={trade_id}, from={from:?}, to={to:?}")]
    InvalidTradeStateTransition {
        symbol:   Ustr,
        trade_id: HandleId,
        from:     TradeState,
        to:       TradeState,
    },

    #[error("Unexpected order kind: {0:?}")]
    UnexpectedOrderKind(HandleOrderKind),

    #[error("Trade ID is missing: symbol={0}, local_order_id={1}")]
    TradeIdMissing(Ustr, OrderId),

    #[error("Cannot exit trade while entering: symbol={0}, trade_id={1}")]
    CannotExitTradeWhileEntering(Ustr, HandleId),

    #[error("Exit order instruction is invalid: symbol={0}, trade_id={1}, instruction={2:#?}")]
    ExitInstructionIsInvalid(Ustr, HandleId, ManagedInstruction),

    #[error("Stoploss order instruction is invalid: symbol={0}, trade_id={1}, instruction={2:#?}")]
    StoplossInstructionIsInvalid(Ustr, HandleId, ManagedInstruction),

    #[error(transparent)]
    InstructionError(#[from] InstructionError),

    #[error("Unexpected trade status: {0:?}")]
    UnexpectedTradeStatus(TradeStatus),

    #[error("The special pending instruction on the trade is missing: symbol={0}, trade_id={1}, instruction={2:#?}")]
    SpecialInstructionIsMissing(Ustr, HandleId, InstructionExecutionResult),

    #[error("The special instruction IDs do not match: symbol={0}, trade_id={1}, expected={2}, actual={3}")]
    SpecialInstructionIdMismatch(Ustr, HandleId, u64, u64),

    #[error("The special instruction outcome is missing: symbol={0}, trade_id={1}, instruction={2:#?}")]
    NoSpecialInstructionExecutionOutcome(Ustr, HandleId, InstructionExecutionResult),

    #[error("Instruction local order ID is missing: symbol={0}, trade_id={1}, instruction={2:#?}")]
    InstructionResultLocalOrderIdIsMissing(Ustr, HandleId, InstructionExecutionResult),

    #[error("Instruction local order ID mismatch: symbol={0}, trade_id={1}, trade_mode={2:?}, exit_instruction={3:#?}, stoploss_instruction={4:#?}")]
    UnexpectedCombinationOfExitAndStoplossInstructions(Ustr, HandleId, TradeStatus, Option<ManagedInstruction>, Option<ManagedInstruction>),

    #[error("Initial quantity mismatch: symbol={0}, trade_id={1}, expected={2}, actual={3}")]
    InitialQuantityMismatch(Ustr, HandleId, OrderedValueType, OrderedValueType),

    #[error("Pending instruction order ID is invalid: {0}")]
    PendingInstructionOrderIdIsInvalid(OrderId),

    #[error("Pending instruction order type is invalid: {0:?}")]
    InvalidPendingInstructionOrderType(HandleOrderKind),

    #[error(transparent)]
    TriggerError(#[from] TriggerError),

    #[error(transparent)]
    CloserError(#[from] CloserError),

    #[error(transparent)]
    OrderError(#[from] OrderError),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TradeProfitAndLoss {
    pub price_delta:       OrderedValueType,
    pub price_delta_ratio: OrderedValueType,
    pub quote_delta:       OrderedValueType,
    pub quote_delta_ratio: OrderedValueType,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum TradeCloseReason {
    #[default]
    None,
    Normal,
    Liquidation,
    Cancelled,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum TradeState {
    /// The trade is not ready to enter and cannot be opened.
    #[default]
    NotReady,

    /// The trade is new and is ready to enter and can be opened.
    New,

    /// The trade is open and can be closed. This is the default state.
    /// New orders can be added to the trade.
    Entering,

    /// The trade has failed and must be either retried or failed and closed.
    EntryFailed,

    /// The trade has an order sent to the exchange and is waiting for it to be filled.
    /// This is a temporary state until the order is filled, then trade can be closed.
    Exiting,

    /// The trade has failed to close and must either retry its normal closing
    /// or fail and close using a contingency plan.
    ExitFailed,

    /// The trade has failed and is closed prematurely.
    Failed,

    /// The closing order has been filled and the trade is closed.
    Closed,
}

impl Display for TradeState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeState::NotReady => write!(f, "NotReady"),
            TradeState::New => write!(f, "New"),
            TradeState::Entering => write!(f, "Entering"),
            TradeState::EntryFailed => write!(f, "EntryFailed"),
            TradeState::Exiting => write!(f, "Exiting"),
            TradeState::ExitFailed => write!(f, "ExitFailed"),
            TradeState::Failed => write!(f, "Failed"),
            TradeState::Closed => write!(f, "Closed"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TradeMetadata {
    pub id:               OrderId,
    pub leverage:         Leverage,
    pub symbol:           Ustr,
    pub timeframe:        Timeframe,
    pub side:             NormalizedSide,
    pub order_type:       NormalizedOrderType,
    pub price:            OrderedValueType,
    pub initial_quantity: OrderedValueType,
    pub filled_quantity:  OrderedValueType,
    pub closer_metadata:  CloserMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TradeType {
    #[default]
    Long,
    Short,
}

#[derive(Debug, Clone)]
pub struct TradeBuilderConfig {
    pub specific_trade_id:    Option<HandleId>,
    pub symbol:               Ustr,
    pub timeframe:            Timeframe,
    pub fee_ratio:            Option<OrderedValueType>,
    pub trade_type:           TradeType,
    pub atomic_market_price:  Arc<AtomicF64>,
    pub quantity:             OrderedValueType,
    pub entry_order_type:     NormalizedOrderType,
    pub entry_activator:      TriggerMetadata,
    pub entry_deactivator:    TriggerMetadata,
    pub entry_time_in_force:  NormalizedTimeInForce,
    pub exit_order_type:      NormalizedOrderType,
    pub exit_activator:       TriggerMetadata,
    pub exit_deactivator:     TriggerMetadata,
    pub exit_time_in_force:   NormalizedTimeInForce,
    pub stoploss_order_type:  NormalizedOrderType,
    pub stoploss_activator:   TriggerMetadata,
    pub closer_kind:          CloserKind,
    pub is_closer_required:   bool,
    pub is_stoploss_required: bool,
}

impl TradeBuilderConfig {
    pub fn long_with_exit_and_stop(
        symbol: Ustr,
        atomic_market_price: Arc<AtomicF64>,
        timeframe: Timeframe,
        quantity: OrderedValueType,
        entry_price: OrderedValueType,
        exit_price: OrderedValueType,
        stoploss_metadata: StoplossMetadata,
    ) -> Result<Self, TradeError> {
        let market_price_snapshot = f!(atomic_market_price.load(Ordering::SeqCst));

        Ok(Self {
            specific_trade_id: None,
            symbol,
            timeframe,
            fee_ratio: Some(f!(0.001)),
            trade_type: TradeType::Long,
            atomic_market_price,
            quantity,
            entry_order_type: NormalizedOrderType::LimitMaker,
            entry_activator: ProximityTrigger::new(entry_price, exit_price, f!(0.10))?.metadata(),
            entry_deactivator: ProximityTrigger::new(entry_price, exit_price, f!(0.80))?.metadata(),
            entry_time_in_force: NormalizedTimeInForce::GTC,
            exit_order_type: NormalizedOrderType::Limit,
            exit_activator: ProximityTrigger::new(exit_price, entry_price, f!(0.10))?.metadata(),
            exit_deactivator: TriggerMetadata::none(),
            exit_time_in_force: NormalizedTimeInForce::GTC,
            stoploss_order_type: NormalizedOrderType::StopLossLimit,
            stoploss_activator: TriggerMetadata::instant(),
            closer_kind: CloserKind::TakeProfit,
            is_closer_required: true,
            is_stoploss_required: true,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TradeBuilder {
    pub config:            TradeBuilderConfig,
    pub entry:             Option<Order>,
    pub exit:              Option<Order>,
    pub stoploss:          Option<Order>,
    pub stoploss_metadata: Option<StoplossMetadata>,
    pub closer:            Option<Box<dyn Closer>>,
}

impl TradeBuilder {
    pub fn new(config: TradeBuilderConfig) -> Self {
        TradeBuilder {
            config,
            entry: None,
            exit: None,
            stoploss: None,
            stoploss_metadata: None,
            closer: None,
        }
    }

    pub fn is_shorting(&self) -> bool {
        self.config.trade_type == TradeType::Short
    }

    pub fn with_entry(mut self, entry: Order) -> Self {
        self.entry = Some(entry);
        self
    }

    pub fn with_exit(mut self, exit: Order) -> Self {
        self.exit = Some(exit);
        self
    }

    pub fn with_stoploss(mut self, stoploss: Order) -> Self {
        self.stoploss = Some(stoploss);
        self
    }

    pub fn with_closer(mut self, closer: Box<dyn Closer>) -> Self {
        self.closer = Some(closer);
        self
    }

    // IMPORTANT: This validation is only for the builder, not for the trade itself.
    fn validate(&self) -> Result<(), TradeError> {
        if self.config.symbol.is_empty() {
            return Err(TradeError::SymbolIsEmpty);
        }

        if !self.config.atomic_market_price.load(Ordering::SeqCst).is_normal() {
            return Err(TradeError::AtomicMarketPriceIsZero);
        }

        if self.entry.is_none() {
            return Err(TradeError::EntryOrderIsMissing);
        }

        if self.stoploss.is_none() && self.config.is_stoploss_required {
            return Err(TradeError::StoplossIsMissing);
        }

        if self.closer.is_none() && self.config.is_closer_required {
            return Err(TradeError::CloserIsMissing);
        }

        if let Some(entry) = &self.entry {
            if matches!(entry.order_type, NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit) {
                return Err(TradeError::InvalidEntryOrderType(entry.order_type));
            }

            match self.config.trade_type {
                TradeType::Long =>
                    if entry.side != NormalizedSide::Buy {
                        return Err(TradeError::TradeEntryOrderSideMismatch(self.config.symbol, 0, self.config.trade_type, entry.local_order_id, entry.side));
                    },
                TradeType::Short =>
                    if entry.side != NormalizedSide::Sell {
                        return Err(TradeError::TradeEntryOrderSideMismatch(self.config.symbol, 0, self.config.trade_type, entry.local_order_id, entry.side));
                    },
            }

            if entry.timeframe != self.config.timeframe {
                return Err(TradeError::TradeTimeframeMismatch(self.config.symbol, 0, self.config.timeframe, entry.timeframe));
            }
        }

        if let Some(exit) = &self.exit {
            if matches!(exit.order_type, NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit) {
                return Err(TradeError::InvalidExitOrderType(exit.order_type));
            }

            match self.config.trade_type {
                TradeType::Long =>
                    if exit.side != NormalizedSide::Sell {
                        return Err(TradeError::TradeExitSideMismatch(self.config.symbol, 0, self.config.trade_type, exit.local_order_id, exit.side));
                    },
                TradeType::Short =>
                    if exit.side != NormalizedSide::Buy {
                        return Err(TradeError::TradeExitSideMismatch(self.config.symbol, 0, self.config.trade_type, exit.local_order_id, exit.side));
                    },
            }

            if exit.timeframe != self.config.timeframe {
                return Err(TradeError::TradeTimeframeMismatch(self.config.symbol, 0, self.config.timeframe, exit.timeframe));
            }
        }

        if let Some(stoploss) = &self.stoploss {
            if !matches!(stoploss.order_type, NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit) {
                return Err(TradeError::InvalidStoplossOrderType(stoploss.order_type));
            }

            /*
            // IMPORTANT: The stoploss order side is always the same as the trade side.
            if stoploss_metadata.side != self.config.side {
                return Err(TradeError::PositioStoplossSideMismatch(self.config.symbol, 0, self.config.side, stoploss_metadata.id, stoploss_metadata.side));
            }
             */

            match self.config.trade_type {
                TradeType::Long =>
                    if stoploss.side != NormalizedSide::Sell {
                        return Err(TradeError::PositioStoplossSideMismatch(
                            self.config.symbol,
                            0,
                            self.config.trade_type,
                            stoploss.local_order_id,
                            stoploss.side,
                        ));
                    },
                TradeType::Short =>
                    if stoploss.side != NormalizedSide::Buy {
                        return Err(TradeError::PositioStoplossSideMismatch(
                            self.config.symbol,
                            0,
                            self.config.trade_type,
                            stoploss.local_order_id,
                            stoploss.side,
                        ));
                    },
            }
        }

        Ok(())
    }

    pub fn build(self) -> Result<Trade, TradeError> {
        // Validating the builder
        self.validate()?;

        let mut entry = self.entry.ok_or(TradeError::EntryOrderIsMissing)?;
        let mut exit = self.exit.ok_or(TradeError::ExitOrderIsMissing)?;
        let mut stoploss = self.stoploss.ok_or(TradeError::StoplossIsMissing)?;
        let mut closer = self.closer.ok_or(TradeError::CloserIsMissing)?;

        // Using either the provided trade ID or generating a new one
        let trade_id = self.config.specific_trade_id.unwrap_or_else(|| SmartId::new_handle_id());

        // Set the trade ID on the entry order
        entry.trade_id = trade_id;

        let trade = Trade {
            id:                           trade_id,
            state:                        TradeState::New,
            status:                       TradeStatus::Idle,
            symbol:                       self.config.symbol,
            timeframe:                    self.config.timeframe,
            atomic_market_price:          self.config.atomic_market_price.clone(),
            leverage:                     Leverage::no_leverage(),
            trade_type:                   self.config.trade_type,
            opener:                       Box::new(NoOpener::default()),
            entry:                        entry,
            stoploss:                     stoploss,
            closer:                       closer,
            exit:                         exit,
            initialized_at:               Utc::now(),
            opened_at:                    None,
            updated_at:                   None,
            closed_at:                    None,
            pending_instruction_order_id: None,
            special_pending_instruction:  None,
            pnl:                          Default::default(),
            close_reason:                 None,
        };

        // Validating the trade
        trade.validate()?;

        Ok(trade)
    }
}

/// The `TradeMode` enum represents the current operational mode of a trading trade.
/// It indicates which type of order is actively engaged or if the trade is in a specific
/// operational state.
///
/// - `Idle`: The trade is open and live but currently has no active orders on the exchange.
///    This mode is used when the trade is being tracked but not actively participating in trades.
/// - `Entry`: Indicates that an entry order is active for the trade. The trade is in the
///    process of establishing or increasing its presence in the market.
/// - `Exit`: An exit order is active, signifying that the trade is in the process of closing
///    or reducing its market presence.
/// - `Stoploss`: A stop-loss order is active, providing risk management by limiting potential losses.
///
/// IMPORTANT:
/// Do not to confuse `TradeMode` with the overall state of the trade.
/// `TradeMode` specifically relates to the type of orders active or the operational approach
/// currently being employed for the trade.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum TradeStatus {
    #[default]
    Idle,
    Entry,
    Exit,
    Stoploss,
}

/// The `avg_entry_price` is the price at which the trade was opened,
/// it can be exact or an average price (in case of multiple or a market order).
/// NOTE: The trade is responsible for closing itself.
#[derive(Debug, Clone)]
pub struct Trade {
    /// Unique identifier for the trade.
    pub id: HandleId,

    /// Current state of the trade (e.g., Open, Closed, etc.).
    pub state: TradeState,

    /// Current operational mode of the trade (e.g., Idle, Entry, Exit, etc.).
    /// NOTE: This is not the same as the trade state.
    pub status: TradeStatus,

    /// Trading symbol for the trade, representing the asset pair.
    pub symbol: Ustr,

    /// Timeframe on which the trade is based, if applicable.
    pub timeframe: Timeframe,

    /// Shared reference to the latest market price of the asset.
    pub atomic_market_price: Arc<AtomicF64>,

    /// Leverage applied to the trade. No leverage is used by default.
    pub leverage: Leverage,

    /// The strategic direction of the trade: Long or Short.
    /// - Long trades profit from price increases (buy low, sell high).
    /// - Short trades profit from price decreases (sell high, buy low).
    ///
    /// NOTE:
    /// The trade type dictates the nature of the entry and exit orders
    /// as well as the side of the stoploss order, which always aligns with the
    /// trade type.
    pub trade_type: TradeType,

    /// The initial order used to enter the trade.
    pub entry: Order,

    /// Optional exit order for closing the trade.
    pub exit: Order,

    /// Associated stoploss order for risk management.
    pub stoploss: Order,

    /// Strategy or mechanism used to open the trade.
    pub opener: Box<dyn Opener>,

    /// Strategy or mechanism used to close the trade.
    pub closer: Box<dyn Closer>,

    /// Current profit and loss of the trade.
    pub pnl: TradeProfitAndLoss,

    /// The reason of why the trade has been closed.
    pub close_reason: Option<TradeCloseReason>,

    /// The Order ID of the pending instruction.
    pub pending_instruction_order_id: Option<OrderId>,

    /// A special pending instruction that belongs to the trade.
    pub special_pending_instruction: Option<ManagedInstruction>,

    /// Timestamp indicating when the trade was initialized.
    pub initialized_at: DateTime<Utc>,

    /// Timestamp indicating when the trade was opened.
    pub opened_at: Option<DateTime<Utc>>,

    /// Timestamp for the most recent update to the trade.
    pub updated_at: Option<DateTime<Utc>>,

    /// Timestamp indicating when the trade was closed.
    pub closed_at: Option<DateTime<Utc>>,
}

impl Trade {
    pub fn validate(&self) -> Result<(), TradeError> {
        let (expected_entry_side, expected_exit_side, expected_stoploss_side) = match self.trade_type {
            TradeType::Long => (NormalizedSide::Buy, NormalizedSide::Sell, NormalizedSide::Sell),
            TradeType::Short => (NormalizedSide::Sell, NormalizedSide::Buy, NormalizedSide::Buy),
        };

        if self.state == TradeState::New {
            if self.entry.trade_id != self.id {
                return Err(TradeError::TradeMismatch(self.symbol, self.id, self.entry.trade_id));
            }

            if self.exit.trade_id != self.id {
                return Err(TradeError::TradeMismatch(self.symbol, self.id, self.exit.trade_id));
            }

            if self.exit.initial_quantity != self.entry.initial_quantity {
                return Err(TradeError::InitialQuantityMismatch(self.symbol, self.id, self.entry.initial_quantity, self.exit.initial_quantity));
            }

            if self.stoploss.trade_id != self.id {
                return Err(TradeError::TradeMismatch(self.symbol, self.id, self.stoploss.trade_id));
            }
        }

        if self.id == 0 {
            return Err(TradeError::TradeIdIsZero(self.symbol));
        }

        if self.entry.trade_id != self.id {
            return Err(TradeError::TradeMismatch(self.symbol, self.id, self.entry.trade_id));
        }

        if self.entry.side != expected_entry_side {
            return Err(TradeError::TradeEntryOrderSideMismatch(self.symbol, self.id, self.trade_type, self.entry.local_order_id, self.entry.side));
        }

        if self.entry.timeframe != self.timeframe {
            return Err(TradeError::TradeTimeframeMismatch(self.symbol, self.id, self.timeframe, self.entry.timeframe));
        }

        if self.exit.trade_id != self.id {
            return Err(TradeError::TradeMismatch(self.symbol, self.id, self.exit.trade_id));
        }

        if self.exit.side != expected_exit_side {
            return Err(TradeError::TradeExitSideMismatch(self.symbol, self.id, self.trade_type, self.exit.local_order_id, self.exit.side));
        }

        if self.exit.timeframe != self.timeframe {
            return Err(TradeError::TradeTimeframeMismatch(self.symbol, self.id, self.timeframe, self.exit.timeframe));
        }

        if self.stoploss.trade_id != self.id {
            return Err(TradeError::TradeMismatch(self.symbol, self.id, self.stoploss.trade_id));
        }

        if self.stoploss.side != expected_stoploss_side {
            return Err(TradeError::PositioStoplossSideMismatch(self.symbol, self.id, self.trade_type, self.stoploss.local_order_id, self.stoploss.side));
        }

        if self.stoploss.timeframe != self.timeframe {
            return Err(TradeError::TradeTimeframeMismatch(self.symbol, self.id, self.timeframe, self.stoploss.timeframe));
        }

        Ok(())
    }

    pub fn id(&self) -> HandleId {
        self.id
    }

    pub fn has_active_orders(&self) -> bool {
        self.entry.is_active() || self.exit.is_active() || self.stoploss.is_active()
    }

    pub fn has_local_order(&self, order_id: OrderId) -> bool {
        self.entry.local_order_id == order_id || self.exit.local_order_id == order_id || self.stoploss.local_order_id == order_id
    }

    pub fn get_active_order(&self) -> Option<&Order> {
        if self.entry.is_active() {
            return Some(self.entry.as_ref());
        }

        if self.exit.is_active() {
            return Some(self.exit.as_ref());
        }

        if self.stoploss.is_active() {
            return Some(self.stoploss.as_ref());
        }

        None
    }

    pub fn get_local_order(&self, order_id: OrderId) -> Option<&Order> {
        if self.entry.local_order_id == order_id {
            return Some(self.entry.as_ref());
        }

        if self.exit.local_order_id == order_id {
            return Some(self.exit.as_ref());
        }

        if self.stoploss.local_order_id == order_id {
            return Some(self.stoploss.as_ref());
        }

        None
    }

    pub fn get_local_order_mut(&mut self, order_id: OrderId) -> Option<&mut Order> {
        // TODO: implement comparison by unique order id
        // TODO: implement comparison by unique order id
        // TODO: implement comparison by unique order id
        // TODO: implement comparison by unique order id
        // TODO: implement comparison by unique order id

        if self.entry.local_order_id == order_id {
            return Some(self.entry.as_mut());
        }

        if self.exit.local_order_id == order_id {
            return Some(self.exit.as_mut());
        }

        if self.stoploss.local_order_id == order_id {
            return Some(self.stoploss.as_mut());
        }

        None
    }

    // TODO: Need to handle the Trade's special pending instruction.
    pub fn get_pending_instruction(&self) -> Option<&ManagedInstruction> {
        // WARNING:
        // This is a special case, the trade's special pending instruction
        // is not stored in the trade's pending instruction order ID.
        // Instead, it is stored in a separate field and has the highest priority.
        if let Some(special_pending_instruction) = self.special_pending_instruction.as_ref() {
            return Some(special_pending_instruction);
        }

        let Some(instruction_order_id) = self.pending_instruction_order_id else {
            return None;
        };

        match SmartId(instruction_order_id).order_kind() {
            HandleOrderKind::EntryOrder => self.entry.pending_instruction.as_ref(),
            HandleOrderKind::ExitOrder => self.exit.pending_instruction.as_ref(),
            HandleOrderKind::StoplossOrder => self.stoploss.pending_instruction.as_ref(),

            _ => {
                error!("Pending instruction order type is invalid: {:#?}", SmartId(instruction_order_id).order_kind());
                return None;
            }
        }
    }

    // TODO: Need to handle the Trade's special pending instruction.
    pub fn get_pending_instruction_mut(&mut self) -> Option<&mut ManagedInstruction> {
        let Some(instruction_order_id) = self.pending_instruction_order_id else {
            return None;
        };

        match SmartId(instruction_order_id).order_kind() {
            HandleOrderKind::EntryOrder => self.entry.pending_instruction.as_mut(),
            HandleOrderKind::ExitOrder => self.exit.pending_instruction.as_mut(),
            HandleOrderKind::StoplossOrder => self.stoploss.pending_instruction.as_mut(),

            _ => {
                error!("Pending instruction order type is invalid: {:#?}", SmartId(instruction_order_id).order_kind());
                return None;
            }
        }
    }

    pub fn get_special_pending_instruction(&self) -> Option<&ManagedInstruction> {
        self.special_pending_instruction.as_ref()
    }

    pub fn get_special_pending_instruction_mut(&mut self) -> Option<&mut ManagedInstruction> {
        self.special_pending_instruction.as_mut()
    }

    pub fn has_remote_order(&self, exchange_order_id: u64) -> bool {
        self.entry.exchange_order_id == Some(exchange_order_id)
            || self.exit.exchange_order_id == Some(exchange_order_id)
            || self.stoploss.exchange_order_id == Some(exchange_order_id)
    }

    fn set_state(&mut self, new_state: TradeState) -> Result<(), TradeError> {
        use TradeState::*;
        let old_state = self.state;

        self.state = match (old_state, new_state) {
            // New state transitions
            (New, Entering) => Entering,

            // Entering transitions
            (Entering, EntryFailed) => EntryFailed,
            (Entering, Exiting) => Exiting,
            (Entering, Cancelled) => Cancelled,

            // Handling EntryFailed
            (EntryFailed, Exiting) => Exiting, // Move to exiting if recovery is not possible

            // Exiting transitions
            (Exiting, Closed) => Closed,
            (Exiting, ExitFailed) => ExitFailed,
            (Exiting, Cancelled) => Cancelled,

            // Handling ExitFailed
            (ExitFailed, Failed) => Failed, // Move to failed state if recovery is not possible

            // If there is no change in the state of the trade
            (same, state) if same == state => state,

            // For any other transition, throw an error
            _ => {
                error!("invalid trade state transition {:?} -> {:?}", old_state, new_state);
                return Err(TradeError::InvalidTradeStateTransition {
                    symbol:   self.symbol,
                    trade_id: self.id,
                    from:     old_state,
                    to:       new_state,
                });
            }
        };

        if self.state != old_state {
            println!("Trade state transition {:?} -> {:?}", old_state, self.state);
            self.updated_at = Some(Utc::now());
        }

        Ok(())
    }

    pub fn is_oco_finished(&self) -> bool {
        self.exit.is_finished() && self.stoploss.is_finished()
    }

    pub fn is_leveraged(&self) -> bool {
        self.leverage.is_leveraged()
    }

    pub fn is_repaid(&self) -> bool {
        self.leverage.is_repaid()
    }

    pub fn is_new(&self) -> bool {
        self.state == TradeState::New
    }

    pub fn is_idle(&self) -> bool {
        self.status == TradeStatus::Idle
    }

    pub fn is_entering(&self) -> bool {
        self.state == TradeState::Entering
    }

    pub fn is_exiting(&self) -> bool {
        self.state == TradeState::Exiting
    }

    pub fn is_closed(&self) -> bool {
        self.state == TradeState::Closed
    }

    pub fn is_failed(&self) -> bool {
        self.state == TradeState::Failed
    }

    pub fn maintenance(&mut self) -> Result<(), TradeError> {
        // NOTE: A just in case check to ensure that the pending instruction order ID is consistent.
        if self.is_exiting() {
            if let Some(pending_instruction_order_id) = self.pending_instruction_order_id {
                if self.entry.local_order_id == pending_instruction_order_id {
                    warn!("ENTRY({}): Clearing the pending instruction order ID", self.entry.order_type);
                    self.pending_instruction_order_id = None;
                }
            }
        }

        Ok(())
    }

    pub fn compute_lifecycle(&mut self) -> Result<(), TradeError> {
        // ----------------------------------------------------------------------------
        // Checking the order states to determine the trade state
        // WARNING: This is a very important step, it determines the trade state.
        // ----------------------------------------------------------------------------
        // IMPORTANT: Checking entry order state
        if self.entry.is_failed() {
            self.set_state(TradeState::EntryFailed)?;
        }

        // TODO: Test this condition
        // TODO: Test this condition
        // TODO: Test this condition
        if self.entry.is_finished_by_cancel && !self.entry.is_partially_filled() {
            // The entry order has been cancelled, preventing the trade from re-entering
            self.entry.terminate()?;

            // The reason for closing the trade is that the entry order has been cancelled
            self.close_reason = Some(TradeCloseReason::Cancelled);

            // The trade is closed
            self.set_state(TradeState::Closed)?;

            return Ok(());
        }

        if self.is_entering() && self.entry.is_finished() {
            // The entry order has been filled, switching back to idle
            self.status = TradeStatus::Idle;

            // Entering the trade is done, switching the trade state to Exiting
            self.set_state(TradeState::Exiting)?;
        }

        if self.is_exiting() {
            if self.exit.is_finished() || self.stoploss.is_finished() {
                info!("Exiting the trade is done, switching the trade state to Closed");

                // Closed normally
                self.close_reason = Some(TradeCloseReason::Normal);

                // Exiting the trade is done, switching the trade state to Closed
                self.set_state(TradeState::Closed)?;

                // The exit order has been filled, switching back to idle
                self.status = TradeStatus::Idle;
            }
        }

        Ok(())
    }

    // TODO: Need to update Trade's quantity based on the exit and stoploss order's filled quantity.
    pub fn compute(&mut self) -> Result<(), TradeError> {
        let market_price = f!(self.atomic_market_price.load(Ordering::SeqCst));

        // ----------------------------------------------------------------------------
        // Handling the pending instruction ID
        // If there is a pending instruction ID, it means that the trade is waiting
        // for an instruction to be executed. If the instruction is executed, the
        // trade will be updated accordingly and the pending instruction ID will
        // be cleared.
        // ----------------------------------------------------------------------------
        if let Some(pending_instruction_order_id) = self.pending_instruction_order_id {
            // If the relevant order's pending instruction is missing, it means that
            // the instruction has been executed and the trade should be updated.
            match self.get_pending_instruction_mut() {
                None => {
                    /*
                    // FIXME: Not sure if this is correct to clear the pending instruction ID here.
                    match SmartId(pending_instruction_order_id).order_kind() {
                        TradeOrderKind::EntryOrder => {
                            info!("ENTRY({}): Clearing the pending instruction order ID", self.entry.order_type);
                            self.entry.pending_instruction = None;
                        }
                        TradeOrderKind::ExitOrder => {
                            info!("EXIT({}): Clearing the pending instruction order ID", self.exit.order_type);
                            self.exit.pending_instruction = None;
                        }
                        TradeOrderKind::StoplossOrder => {
                            info!("STOPLOSS({}): Clearing the pending instruction order ID", self.stoploss.order_type);
                            self.stoploss.pending_instruction = None;
                        }
                        _ => {
                            error!("Pending instruction order type is invalid: {:#?}", SmartId(pending_instruction_order_id).order_kind());
                            return Err(TradeError::InvalidPendingInstructionOrderType(SmartId(pending_instruction_order_id).order_kind()));
                        }
                    }
                     */
                }
                Some(pending_instruction) => {
                    // Otherwise, we could check the instruction state and handle it accordingly.
                    // TODO: ... Handle the instruction state (if needed)
                }
            }
        }

        // Also checking the special pending instruction
        if let Some(special_pending_instruction) = self.special_pending_instruction.as_mut() {
            // If the special pending instruction is missing, it means that
            // the instruction has been executed and the trade should be updated.
            if special_pending_instruction.is_executed() {
                self.special_pending_instruction = None;
            }
        }

        // ----------------------------------------------------------------------------
        // Compute the trade's lifecycle then check whether the
        // trade is ready to be closed or not.
        // ----------------------------------------------------------------------------
        self.compute_lifecycle()?;

        if self.is_closed() {
            return Ok(());
        }

        // ----------------------------------------------------------------------------
        // IMPORTANT:
        // If the trade state is New, it means that the trade has not been
        // opened yet and the entry order has not been yet sent to the exchange.
        // ----------------------------------------------------------------------------

        if self.is_entering() || self.is_new() {
            self.entry.compute()?;

            if self.pending_instruction_order_id.is_some() {
                return Ok(());
            }

            if !self.is_idle() {
                return Ok(());
            }

            // Switching the trade state to Entering
            self.set_state(TradeState::Entering)?;

            // Assigning the order ID of an active instruction to the trade
            if self.entry.has_pending_instruction() {
                self.pending_instruction_order_id = Some(self.entry.local_order_id);
            }

            return Ok(());
        }

        // WARNING: Checking and fixing any inconsistencies in the trade (fixing if possible)
        self.maintenance()?;

        // ----------------------------------------------------------------------------
        // Computing the profit and loss of the trade
        // ----------------------------------------------------------------------------
        self.pnl.price_delta = market_price - self.entry.price;
        self.pnl.price_delta_ratio = self.pnl.price_delta / self.entry.price;
        self.pnl.quote_delta = self.pnl.price_delta * self.entry.initial_quantity;
        self.pnl.quote_delta_ratio = self.pnl.price_delta_ratio * self.entry.initial_quantity;

        // ----------------------------------------------------------------------------
        // Computing both closer and stoploss instructions is done in `compute_exit`.
        // NOTE: Checking pending instructions BEFORE computing the exit and stoploss
        // IMPORTANT: The trade MUST wait for the special instruction to be executed first.
        // ----------------------------------------------------------------------------
        self.exit.compute()?;
        self.stoploss.compute()?;

        if self.special_pending_instruction.is_some() {
            return Ok(());
        }

        // TODO: I'm not completely sure about this, but I think it is correct to stop with a regular pending instruction.
        if self.pending_instruction_order_id.is_some() {
            return Ok(());
        }

        // TODO: Need to handle the case where there is an active exit order.
        // TODO: Need to handle the case where the stoploss instruction is a market order.
        // FIXME: I think computation must be done even if there is a pending instruction.
        match (self.exit.pending_instruction, self.stoploss.pending_instruction) {
            (None, None) => {
                // No pending instructions for either exit or stoploss orders.
            }

            // IMPORTANT: Always let the stoploss cancellation instruction to be executed first.
            (_, Some(stoploss_instruction)) if stoploss_instruction.is_cancellation() || self.stoploss.is_pending_cancel() => {
                self.pending_instruction_order_id = Some(self.stoploss.local_order_id);
                return Ok(());
            }

            (Some(exit_instruction), _) => {
                // If there is an active exit order, then do nothing.
                if !self.exit.is_pending_open() && self.exit.is_active() {
                    return Ok(());
                }

                if self.exit.is_pending_cancel() {
                    return Ok(());
                }

                if self.stoploss.is_pending_cancel() {
                    return Ok(());
                }

                if self.stoploss.is_flagged_cancel_and_replace() {
                    self.stoploss.cancel_pending_instruction()?;
                    return Ok(());
                }

                if self.stoploss.is_flagged_cancel_and_idle() {
                    return Ok(());
                }

                // If there is an active stoploss order, it must be cancelled first.
                if self.stoploss.is_active() {
                    self.stoploss.cancel_pending_instruction()?;
                    // Flag the stoploss order for cancellation before proceeding
                    self.stoploss.cancel_and_idle()?;
                    return Ok(());
                }

                if !self.exit.is_pending_open() && !self.stoploss.is_idle() {
                    return Ok(());
                }

                // IMPORTANT: Attempt to cancel any pending instruction
                // Instruction will be cancelled if it is not yet executed. Otherwise, it will be ignored.
                self.stoploss.cancel_pending_instruction()?;

                // Assigning the order ID of an active instruction to the trade
                self.pending_instruction_order_id = Some(self.exit.local_order_id);
            }

            (_, Some(stoploss_instruction)) if self.exit.is_active() => {
                // If there is an active exit order, stoploss instruction should be ignored.
                return Ok(());
            }

            (None, Some(stoploss_instruction)) if self.stoploss.is_active() => {
                self.pending_instruction_order_id = Some(self.stoploss.local_order_id);
                return Ok(());
            }

            _ => {}
        }

        Ok(())
    }

    pub fn cancel_active_order_and_idle(&mut self) -> Result<(), TradeError> {
        if self.entry.is_active() {
            info!("Cancelling the entry order to idle");
            return Ok(self.entry.cancel_and_idle()?);
        }

        if self.exit.is_active() {
            info!("Cancelling the exit order to idle");
            return Ok(self.exit.cancel_and_idle()?);
        }

        if self.stoploss.is_active() {
            info!("Cancelling the stoploss order to idle");
            return Ok(self.stoploss.cancel_and_idle()?);
        }

        Ok(())
    }

    pub fn cancel_active_order_and_terminate(&mut self) -> Result<(), TradeError> {
        if self.entry.is_active() {
            info!("Cancelling the entry order to terminate");
            return Ok(self.entry.terminate()?);
        }

        if self.exit.is_active() {
            info!("Cancelling the exit order to terminate");
            return Ok(self.exit.terminate()?);
        }

        if self.stoploss.is_active() {
            info!("Cancelling the stoploss order to terminate");
            return Ok(self.stoploss.terminate()?);
        }

        Ok(())
    }

    pub fn confirm_instruction_execution(&mut self, result: InstructionExecutionResult) -> Result<(), TradeError> {
        info!("Confirming instruction execution for trade {}...", self.id);

        let Some(local_order_id) = result.metadata.local_order_id else {
            return Err(TradeError::InstructionResultLocalOrderIdIsMissing(self.symbol, self.id, result));
        };

        let (trade_id, encoded_order_kind, _, _) = SmartId(local_order_id).decode();

        if trade_id != self.id {
            return Err(TradeError::TradeMismatch(self.symbol, self.id, trade_id));
        }

        if result.metadata.is_special {
            let Some(special_pending_instruction) = self.special_pending_instruction.as_mut() else {
                return Err(TradeError::SpecialInstructionIsMissing(self.symbol, self.id, result));
            };

            if special_pending_instruction.id != result.instruction_id {
                return Err(TradeError::SpecialInstructionIdMismatch(self.symbol, self.id, special_pending_instruction.id, result.instruction_id));
            }

            match &result.outcome {
                Some(outcome) => {
                    match outcome {
                        ExecutionOutcome::Success => {
                            self.updated_at = Some(Utc::now());
                            special_pending_instruction.set_state(InstructionState::Executed)?;
                            special_pending_instruction.executed_at = result.executed_at;
                        }

                        ExecutionOutcome::Failure(desc) => {
                            special_pending_instruction.set_state(InstructionState::Failed)?;

                            // IMPORTANT: If the instruction failed, we should start terminating procedure
                            error!("Special pending instruction {} failed with status {:?}, terminating trade...", result.instruction_id, outcome);
                            // TODO: Terminate the trade (use emergency instruction)
                            // TODO: Or reset relevant orders and start over
                        }
                    }
                }

                None => {
                    error!("Special instruction execution result has no outcome, this should never happen");
                    return Err(TradeError::NoSpecialInstructionExecutionOutcome(self.symbol, self.id, result));
                }
            }

            // TODO: Finalize special instruction
            // ...

            // NOTE: Clearing the pending instruction here is vital to avoid double execution
            info!("Clearing the special pending instruction");
            self.special_pending_instruction = None;
        } else {
            match encoded_order_kind {
                HandleOrderKind::EntryOrder => {
                    self.entry.confirm_instruction_execution(result)?;
                }

                HandleOrderKind::ExitOrder => {
                    self.exit.confirm_instruction_execution(result)?;
                }

                HandleOrderKind::StoplossOrder => {
                    self.stoploss.confirm_instruction_execution(result)?;
                }

                // TODO: Possibly handle the case the `SpecialOrder` order kind
                _ => {
                    error!("Attempting to confirm instruction execution for an invalid order type: {:#?}", encoded_order_kind);
                    return Err(TradeError::UnexpectedOrderKind(encoded_order_kind));
                }
            }

            // TODO: Finalize instruction
            // ...

            info!("Clearing the pending instruction order ID: {:?}", encoded_order_kind);
            self.pending_instruction_order_id = None;
        }

        self.compute_lifecycle();
        self.compute()
    }

    pub fn confirm_opened(&mut self, local_order_id: OrderId, exchange_order_id: Option<OrderId>) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_opened(exchange_order_id)?;

        // Updating the trade's status
        self.status = match SmartId(local_order_id).order_kind() {
            HandleOrderKind::EntryOrder => TradeStatus::Entry,
            HandleOrderKind::ExitOrder => TradeStatus::Exit,
            HandleOrderKind::StoplossOrder => TradeStatus::Stoploss,
            order_kind => {
                error!("Attempting to confirm order opened for an invalid order kind: {:#?}", order_kind);
                return Err(TradeError::UnexpectedOrderKind(order_kind));
            }
        };

        self.compute()
    }

    pub fn confirm_open_failed(&mut self, local_order_id: OrderId) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_open_failed()?;

        // Resetting the trade's status to Idle since the order has failed to open
        self.status = TradeStatus::Idle;

        // Otherwise, we need to check if the trade is entering or exiting
        if self.is_entering() {
            self.set_state(TradeState::EntryFailed)?;
        } else {
            // NOTE: At this point (and for now) the `else` scenario is obvious: the trade is exiting
            self.set_state(TradeState::ExitFailed)?;
        }

        self.compute()
    }

    pub fn confirm_replace_failed(&mut self, local_order_id: OrderId) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_replace_failed()?;

        self.compute()
    }

    pub fn confirm_filled(&mut self, local_order_id: OrderId, latest_fill: OrderFill) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_filled(latest_fill)?;

        // IMPORTANT: Updating the trade's status only if the order is an entry order
        if SmartId(local_order_id).order_kind() == HandleOrderKind::EntryOrder {
            // Updating the quantity for the exit and stoploss orders
            self.exit.initial_quantity = self.entry.accumulated_filled_quantity;
            self.stoploss.initial_quantity = self.entry.accumulated_filled_quantity;
        }

        self.compute()
    }

    pub fn confirm_cancelled(&mut self, local_order_id: OrderId) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_cancelled()?;

        // Resetting the trade's status to Idle since the order has been cancelled
        self.status = TradeStatus::Idle;

        self.compute()
    }

    pub fn confirm_cancel_failed(&mut self, local_order_id: OrderId) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_cancel_failed()?;

        self.compute()
    }

    pub fn confirm_rejected(&mut self, local_order_id: OrderId) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_rejected()?;

        // Resetting the trade's status to Idle since the order
        // has been rejected (not created on the exchange)
        self.status = TradeStatus::Idle;

        self.compute()
    }

    pub fn confirm_expired(&mut self, local_order_id: OrderId) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_order_id));
        };

        relevant_order.confirm_expired()?;

        self.compute()
    }

    pub fn confirm_replaced(&mut self, local_original_order_id: OrderId, event: NormalizedOrderEvent) -> Result<(), TradeError> {
        let Some(relevant_order) = self.get_local_order_mut(local_original_order_id) else {
            return Err(TradeError::OrderNotFound(self.symbol, self.id, local_original_order_id));
        };

        relevant_order.confirm_replaced(event)?;

        // Updating the trade's status to the relevant mode
        self.status = match SmartId(local_original_order_id).order_kind() {
            HandleOrderKind::EntryOrder => TradeStatus::Entry,
            HandleOrderKind::ExitOrder => TradeStatus::Exit,
            HandleOrderKind::StoplossOrder => TradeStatus::Stoploss,
            order_kind => {
                error!("Attempting to confirm order replaced for an invalid order kind: {:#?}", order_kind);
                return Err(TradeError::UnexpectedOrderKind(order_kind));
            }
        };

        self.compute()
    }
}
