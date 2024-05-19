use std::{
    fmt,
    fmt::{Debug, Display, Formatter},
    ops::Range,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

use crate::{
    appraiser::Appraiser,
    cooldown::Cooldown,
    countdown::Countdown,
    f,
    id::{CoinPairId, HandleId, OrderId, SmartId, SmartIdError},
    instruction::{
        ExecutionOutcome, Instruction, InstructionError, InstructionExecutionResult,
        InstructionKind, InstructionMetadata, InstructionState, ManagedInstruction,
    },
    leverage::Leverage,
    timeframe::Timeframe,
    trade_closer::{Closer, CloserMetadata},
    trigger::{Trigger, TriggerError, TriggerMetadata},
    types::{
        NormalizedOrderEvent, NormalizedOrderType, NormalizedSide, NormalizedTimeInForce,
        OrderLifecycle, OrderLifecycleAction, OrderedValueType, Origin, Priority, Status,
    },
};
use chrono::{DateTime, TimeZone, Utc};
use log::{error, info, warn};
use num_traits::{ToPrimitive, Zero};
use ordered_float::OrderedFloat;
use portable_atomic::AtomicF64;
use smallvec::SmallVec;
use thiserror::Error;
use ustr::Ustr;
use yata::core::ValueType;

#[derive(Error, Clone, Debug, Eq, PartialEq)]
pub enum OrderError {
    #[error("Local order id is zero")]
    LocalOrderIdIsZero,

    #[error("Entry price is not set")]
    NoEntryPrice,

    #[error("Exit price is not set")]
    NoExitPrice,

    #[error("Stop is not set")]
    NoStop,

    #[error("Stop is invalid")]
    InvalidStop,

    #[error("Stop is invalid")]
    NonPositiveQuantity,

    #[error("Non-positive price")]
    NonPositivePrice,

    #[error("Operation is ignored due to rate limiter")]
    RateLimited,

    #[error("Order not found: {0}")]
    OrderNotFound(OrderId),

    #[error("Order is closed: {0}")]
    OrderIsClosed(OrderId),

    #[error("Roundtrip not found: {0}")]
    RoundtripNotFound(OrderId),

    #[error("Roundtrip id is already registered: {0}")]
    ExistingRoundtripId(OrderId),

    #[error("Invalid roundtrip id is empty: {0}")]
    EmptyRoundtripId(OrderId),

    #[error("One-time order id is already registered: {0}")]
    ExistingOrderId(OrderId),

    #[error("One-time order id is empty: {0}")]
    EmptyOrderId(OrderId),

    #[error("Trade not found: {0}")]
    TradeNotFound(HandleId),

    #[error("Leveraged trade is not repaid: {0}")]
    LeveragedTradeNotRepaid(HandleId),

    #[error("Trade id is already registered: {0}")]
    ExistingTradeId(HandleId),

    #[error("Invalid trade is already closed: {0}")]
    TradeIsClosed(HandleId),

    #[error("Order has no price: {0}")]
    OrderHasNoPrice(OrderId),

    #[error("Quantity is zero")]
    ZeroQuantity,

    #[error("The price is zero")]
    ZeroPrice,

    #[error("The move-to price is zero")]
    ZeroMoveToPrice,

    #[error("Entry price is zero")]
    ZeroEntryPrice,

    #[error("Exit price is zero")]
    ZeroExitPrice,

    #[error("Stop price is zero")]
    ZeroStopPrice,

    #[error("StopLimit price is zero")]
    ZeroStopLimitPrice,

    #[error("Order state cannot transition from {from:?} to {to:?}")]
    InvalidOrderStateTransition { from: OrderState, to: OrderState },

    #[error("An error has occured: {0:#?}")]
    Error(String),

    #[error("Filled and PartiallyFilled must be accompanied by at least one of: Quote or Base")]
    InvalidOrderFill,

    #[error("Symbol mismatch: {0:?} != {1:?}, order_id: {2:?}, trade_id: {3:?}")]
    SymbolMismatch(Ustr, Ustr, OrderId, HandleId),

    #[error("Order is not finalized: {0:?} {1:?}")]
    OrderNotFinalized(Ustr, OrderId),

    #[error("Order is already finalized: {0:#?}")]
    OrderIsAlreadyFinalized(Ustr, OrderId),

    #[error("Order is already finalized: {0:?} {1:?}")]
    SideMismatch(NormalizedSide, NormalizedSide),

    #[error("Timeframe is required but is missing")]
    MissingTimeframe,

    #[error("Remote order id is zero")]
    ZeroRemoteOrderId,

    #[error("Roundtrip is already finalized: {0:#?}")]
    RoundtripIsAlreadyFinalized(OrderId),

    #[error("Missing order fill metadata: {0} {1}")]
    MissingOrderFillMetadata(Ustr, OrderId),

    #[error("Invalid timestamp: {0}")]
    InvalidTimestamp(DateTime<Utc>),

    #[error("Order has no pending instruction")]
    OrderHasNoPendingInstruction,

    #[error("Wrong instruction execution result: local_order_id={0:?} managed_instruction={1:#?} execution_result={2:#?}")]
    WrongInstructionExecutionResult(OrderId, ManagedInstruction, InstructionExecutionResult),

    #[error("Unsupported instruction execution status: {0:?}")]
    UnsupportedInstructionExecutionStatus(Status),

    #[error("Order with local id already exists: {0:?}")]
    OrderLocalIdAlreadyExists(OrderId),

    #[error("Order with remote id already exists: {0:?}")]
    OrderRemoteIdAlreadyExists(OrderId),

    #[error("Order already exists in trade: {0:?}")]
    OrderAlreadyExistsInTrade((Ustr, HandleId, OrderId)),

    #[error("No instruction execution outcome")]
    NoInstructionExecutionOutcome,

    #[error("Order is missing latest fill: {0} {1} {2}")]
    OrderMissingLatestFill(Ustr, u64, u64),

    #[error("Stop order id is zero")]
    StopOrderIdIsZero,

    #[error("Invalid stop price: {0}")]
    InvalidStopPrice(OrderedValueType),

    #[error("Invalid limit price: {0}")]
    InvalidLimitPrice(OrderedValueType),

    #[error("Invalid stop or limit price")]
    InvalidStopOrLimitPrice,

    #[error("Invalid current market price: {0}")]
    InvalidCurrentMarketPrice(OrderedValueType),

    #[error("Invalid initial stop price: {0}")]
    InvalidInitialStopPrice(OrderedValueType),

    #[error("Invalid stop price limit offset ratio: {0}")]
    InvalidLimitPriceOffsetRatio(OrderedValueType),

    #[error("Invalid initial trailing stop price offset ratio: {0}")]
    InvalidInitialStepRatio(OrderedValueType),

    #[error("Invalid trailing stop price offset ratio: {0}")]
    InvalidTrailingStepRatio(OrderedValueType),

    #[error("Limit price not specified")]
    LimitPriceNotSpecified,

    #[error("Stoploss metadata is missing")]
    StoplossMissingMetadata,

    #[error("Invalid buy stop price: {stop_price} (reference price: {reference_price})")]
    InvalidBuyStopPrice {
        reference_price: OrderedValueType,
        stop_price: OrderedValueType,
    },

    #[error("Invalid buy limit price: {limit_price} (stop price: {stop_price})")]
    InvalidBuyLimitPrice {
        stop_price: OrderedValueType,
        limit_price: OrderedValueType,
    },

    #[error("Invalid sell stop price: {stop_price} (reference price: {reference_price})")]
    InvalidSellStopPrice {
        reference_price: OrderedValueType,
        stop_price: OrderedValueType,
    },

    #[error("Invalid sell limit price: {limit_price} (stop price: {stop_price})")]
    InvalidSellLimitPrice {
        stop_price: OrderedValueType,
        limit_price: OrderedValueType,
    },

    #[error("Order is not stoploss")]
    OrderIsNotStoploss,

    #[error("Invalid order type: {0}")]
    InvalidOrderType(NormalizedOrderType),

    #[error("Invalid reference price: {0}")]
    InvalidReferencePrice(OrderedValueType),

    #[error("Invalid replacement confirmation state: got={0}, expected={1}")]
    InvalidReplaceConfirmationState(OrderState, OrderState),

    #[error("Pending instruction is in progress: {0:#?}")]
    PendingInstructionIsInProgress(ManagedInstruction),

    #[error("Cannot cancel active pending instruction: trade_id={0:?} order_id={1:?} managed_instruction={2:#?}")]
    CannotCancelActivePendingInstruction(HandleId, OrderId, ManagedInstruction),

    #[error("Invalid cancel handling state: got={0}, expected={1}")]
    InvalidCancelState(OrderState, OrderState),

    #[error(transparent)]
    TriggerError(#[from] TriggerError),

    #[error(transparent)]
    IdError(#[from] SmartIdError),

    #[error(transparent)]
    InstructionError(#[from] InstructionError),
}

#[derive(Error, Debug, Clone)]
pub enum OrderConfigError {
    #[error("Invalid symbol: {0}")]
    InvalidSymbol(String),

    #[error("Price is zero")]
    ZeroPrice,

    #[error("Quantity is zero")]
    ZeroQuantity,

    #[error("Invalid stop limit price offset ratio: {0}")]
    InvalidStopLimitPriceOffsetRatio(ValueType),
}

#[derive(Debug, Clone)]
pub struct OrderConfig {
    pub leverage: Leverage,
    pub symbol: Ustr,
    pub trade_id: HandleId,
    pub timeframe: Option<Timeframe>,
    pub side: NormalizedSide,
    pub order_type: NormalizedOrderType,
    pub price: OrderedValueType,
    pub quantity: OrderedValueType,
    pub closer_metadata: CloserMetadata,
    pub activator: TriggerMetadata,
    pub deactivator: TriggerMetadata,
}

impl OrderConfig {
    pub fn validate(&self) -> Result<(), OrderConfigError> {
        if self.symbol.is_empty() {
            return Err(OrderConfigError::InvalidSymbol(self.symbol.to_string()));
        }

        if self.price.is_zero() {
            return Err(OrderConfigError::ZeroPrice);
        }

        if self.quantity.is_zero() {
            return Err(OrderConfigError::ZeroQuantity);
        }

        Ok(())
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum OrderState {
    #[default]
    Idle,
    PendingOpen,
    OpenFailed,
    Opened,
    PartiallyFilled,
    Filled,
    Closed,
    PendingCancel,
    Cancelled,
    CancelledAndPendingReplace,
    CancelFailed,
    ReplaceFailed,
    Replaced,
    Rejected,
    Expired,
    Finished,
}

impl Display for OrderState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            OrderState::Idle => write!(f, "Idle"),
            OrderState::PendingOpen => write!(f, "PendingOpen"),
            OrderState::PendingCancel => write!(f, "PendingCancel"),
            OrderState::OpenFailed => write!(f, "OpenFailed"),
            OrderState::Opened => write!(f, "Opened"),
            OrderState::PartiallyFilled => write!(f, "PartiallyFilled"),
            OrderState::Filled => write!(f, "Filled"),
            OrderState::Closed => write!(f, "Closed"),
            OrderState::Cancelled => write!(f, "Cancelled"),
            OrderState::CancelledAndPendingReplace => write!(f, "CancelledAndPendingReplace"),
            OrderState::Rejected => write!(f, "Rejected"),
            OrderState::Expired => write!(f, "Expired"),
            OrderState::ReplaceFailed => write!(f, "ReplaceFailed"),
            OrderState::Replaced => write!(f, "Replaced"),
            OrderState::CancelFailed => write!(f, "CancelFailed"),
            OrderState::Finished => write!(f, "Finished"),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum OrderFillStatus {
    PartiallyFilled,
    Filled,
}

#[derive(Debug, Copy, Clone)]
pub struct OrderFill {
    pub status: OrderFillStatus,
    pub metadata: OrderFillMetadata,
}

impl OrderFill {
    pub fn new(
        status: OrderFillStatus,
        price: OrderedValueType,
        quantity: OrderedValueType,
        accumulated_filled_quantity: OrderedValueType,
        accumulated_filled_quote: OrderedValueType,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            status,
            metadata: OrderFillMetadata {
                price,
                quantity,
                quote_equivalent: price * quantity,
                accumulated_filled_quantity,
                accumulated_filled_quote,
                timestamp,
            },
        }
    }

    pub fn validate(&self) -> Result<(), OrderError> {
        self.metadata.validate()
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct StoplossMetadata {
    /// Flag indicating whether the trailing stop is enabled or not, at all.
    pub is_trailing_enabled: bool,

    /// The reference price used to calculate the stop price. (e.g., entry price)
    pub reference_price: OrderedValueType,

    /// The initial price at which the stop is set when it is first created.
    pub initial_stop_price: OrderedValueType,

    /// The ratio from the market price at which the trailing stop updates the stop price, after the initial trailing activation.
    pub trailing_step_ratio: OrderedValueType,
}

impl StoplossMetadata {
    pub fn new(
        is_trailing_enabled: bool,
        reference_price: OrderedValueType,
        initial_stop_price: OrderedValueType,
        trailing_step_ratio: OrderedValueType,
    ) -> Self {
        Self {
            is_trailing_enabled,
            reference_price,
            initial_stop_price,
            trailing_step_ratio,
        }
    }
}

impl Debug for StoplossMetadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoplossMetadata")
            .field("is_trailing_enabled", &self.is_trailing_enabled)
            .field("reference_price", &self.reference_price.0)
            .field("initial_stop_price", &self.initial_stop_price.0)
            .field("trailing_step_ratio", &self.trailing_step_ratio.0)
            .finish()
    }
}

/// The metadata for an order fill e.g., the price, quantity, timestamp, etc.
#[derive(Copy, Clone)]
pub struct OrderFillMetadata {
    pub price: OrderedValueType,
    pub quantity: OrderedValueType,
    pub quote_equivalent: OrderedValueType,
    pub accumulated_filled_quantity: OrderedValueType,
    pub accumulated_filled_quote: OrderedValueType,
    pub timestamp: DateTime<Utc>,
}

impl Debug for OrderFillMetadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrderFillMetadata")
            .field("price", &self.price.0)
            .field("quantity", &self.quantity.0)
            .field("quote_equivalent", &self.quote_equivalent.0)
            .field(
                "accumulated_filled_quantity",
                &self.accumulated_filled_quantity.0,
            )
            .field("accumulated_filled_quote", &self.accumulated_filled_quote.0)
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

impl OrderFillMetadata {
    pub fn validate(&self) -> Result<(), OrderError> {
        if self.quantity.is_zero() {
            return Err(OrderError::ZeroQuantity);
        }

        if self.price.is_zero() {
            return Err(OrderError::ZeroPrice);
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct OrderMetadata {
    /// Where the order originated from. Whether it was created by the system or the exchange.
    pub origin: Origin,

    /// Specifies whether the orders for this trade are virtual or not.
    /// NOTE: A virtual order is not placed on the exchange and is only used for tracking purposes.
    pub is_virtual: bool,

    /// The ratio used to calculate the limit price relative to the current stop price.
    pub limit_price_offset_ratio: OrderedValueType,

    /// The stoploss metadata. This is only used for stoploss orders.
    pub stoploss: Option<StoplossMetadata>,
}

impl Default for OrderMetadata {
    fn default() -> Self {
        Self {
            origin: Default::default(),
            is_virtual: false,
            limit_price_offset_ratio: f!(0.001),
            stoploss: None,
        }
    }
}

impl Debug for OrderMetadata {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OrderMetadata")
            .field("origin", &self.origin)
            .field("is_virtual", &self.is_virtual)
            .field("limit_price_offset_ratio", &self.limit_price_offset_ratio.0)
            .field("stoploss", &self.stoploss)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderMode {
    #[default]
    Normal,
    Paused,
}

impl Display for OrderMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            OrderMode::Normal => write!(f, "Normal"),
            OrderMode::Paused => write!(f, "Paused"),
        }
    }
}

/// ### Incremental Trailing Stop Strategy
///
/// #### Objective:
///
/// To create a stop-loss mechanism that incrementally trails the best market price. The goal is to reduce the risk of premature trade exits while maximizing profit potential.
///
/// #### Strategy Mechanics:
///
/// 1. **Initialization**: Establish an initial stop price (e.g., 98.0) upon entering the trade.
///
/// 2. **Market Monitoring**: Continuously track the current market price.
///
/// 3. **Condition Evaluation**: When the market price surpasses a level that is 'X%' greater than the current stop price (known as the "trailing step ratio"), the stop price is updated.
///
/// #### Variables:
///
/// - `initial_stop_price`: 98.0
/// - `current_market_price`: 100.0
/// - `trailing_step_ratio`: 0.02 (or 2%)
///
/// #### Example:
///
/// - Initial Stop Price: 98.0
/// - Current Market Price: 100.0
/// - Market moves to: 102.01
/// - Trailing Step Ratio: 0.02
///
/// Based on the above, since the market price has moved up by at least 2% relative to the current stop price, we update the stop price. The key idea is to make the stop price crawl bottom-up, starting from the initial stop price, instead of dynamically updating it based on the current market price.
///
/// In this example, the new stop price would move to 100.0 (i.e., break-even point), as opposed to immediately jumping to 102.0 and risking a premature exit.
///
/// This approach minimizes the risk of the stop getting hit immediately after being updated.
///
/// Binance uses the following fields for the client order IDs:
///
/// `clientOrderId`: A general unique identifier for orders.
/// `origClientOrderId`: Used for modifying or canceling orders.
/// `listClientOrderId`: A unique ID for the entire OCO order list.
/// `limitClientOrderId`: A unique ID for the limit order leg of an OCO.
/// `stopClientOrderId`: A unique ID for the stop-loss leg of an OCO.
/// `newClientOrderId`: Used when modifying an order to assign a new ID.

#[derive(Clone)]
pub struct Order {
    /// Unique identifier for the order set by the client.
    /// Maps to `clientOrderId` in the Binance API for standard orders.
    /// NOTE: This is the primary internal ID used to identify the order in the system.
    pub local_order_id: OrderId,

    /// If the client order ID is not an integer, this field is used to store the raw order ID.
    pub raw_order_id: Option<String>,

    /// Optional identifier for the stop order part of an OCO.
    /// Maps to `stopClientOrderId` in the Binance API for OCO orders.
    /// NOTE: This is used to identify the stop order in the system.
    pub local_stop_order_id: Option<OrderId>,

    /// Optional original client order ID, used for order modification or cancellation.
    /// Maps to `origClientOrderId` in the Binance API.
    pub local_orig_order_id: Option<OrderId>,

    /// Optional identifier for the list order in OCO.
    /// Maps to `listClientOrderId` in the Binance API for OCO orders.
    pub local_list_order_id: Option<OrderId>,

    /// Remote identifier for the order as recognized by the exchange.
    /// NOTE: This can be None, possibly due to the order not being acknowledged by the exchange yet.
    pub exchange_order_id: Option<OrderId>,

    /// Unique identifier for the position.
    /// NOTE: Zero id means the new position needs to be created for this order.
    pub trade_id: HandleId,

    /// The order mode.
    pub mode: OrderMode,

    /// If the order is in pending mode, this field indicates the mode to switch to once the order is Idle again.
    pub pending_mode: Option<OrderMode>,

    /// Symbol for the pair of assets involved in the trade.
    pub symbol: Ustr,

    /// The latest known market price of the asset being traded.
    pub atomic_market_price: Arc<AtomicF64>,

    /// Responsible for determining the price of the asset.
    pub appraiser: Appraiser,

    /// The timeframe on which the order is traded, that is, the primary timeframe for this order.
    /// NOTE: This is used to determine the timeframe on which the order is traded.
    pub timeframe: Timeframe,

    /// The time in force for the order. This defines the lifetime of the order on the exchange.
    pub time_in_force: NormalizedTimeInForce,

    /// Order side.
    pub side: NormalizedSide,

    /// The order type used for the order. This is the primary order type used for the order.
    pub order_type: NormalizedOrderType,

    /// The fallback order type used for the order. This is used when retries are exhausted for the primary order type.
    /// NOTE: This can be None if there's no fallback order type.
    pub contingency_order_type: Option<NormalizedOrderType>,

    /// Order activator (when the actual order is created).
    pub activator: Box<dyn Trigger>,

    /// Order deactivator (when the actual order is cancelled).
    pub deactivator: Box<dyn Trigger>,

    /// The market price at the time of order initialization (initialization is always local).
    pub market_price_at_initialization: Option<OrderedValueType>,

    /// The market price at the time of order creation (creation on the exchange).
    pub market_price_at_opening: Option<OrderedValueType>,

    /// The price at which the order should be executed. This field is used to specify the execution price
    /// for limit orders and the stop price for stop orders.
    pub price: OrderedValueType,

    /// The limit price for the order. This is used to specify the maximum price for limit orders.
    /// NOTE: This can be None if the order type does not require a limit price.
    pub limit_price: Option<OrderedValueType>,

    /// Order quantity.
    pub initial_quantity: OrderedValueType,

    /// Order quantity that is filled so far (base).
    pub accumulated_filled_quantity: OrderedValueType,

    /// Order quantity that is filled so far (quote).
    pub accumulated_filled_quote: OrderedValueType,

    /// Order fee (ratio).
    pub fee_ratio: Option<OrderedValueType>,

    /// Number of retries.
    pub retry_counter: Countdown<u8>,

    /// The cooldown between retries.
    pub retry_cooldown: Cooldown,

    /// The lifecycle actions of the order.
    pub lifecycle: OrderLifecycle,

    /// Leverage information for this order. It is recommended to use leverage cautiously.
    pub leverage: Leverage,

    /// The current state of the order.
    pub state: OrderState,

    /// Indicates if the order is flagged for termination (e.g. by the user).
    pub termination_flag: bool,

    /// Indicates if the order is flagged for cancellation and idle.
    pub cancel_and_idle_flag: bool,

    /// Indicates if the order is flagged for cancellation and replacement.
    pub cancel_and_replace_flag: bool,

    /// The order fills.
    pub fills: SmallVec<[OrderFill; 16]>,

    /// The number of times the stop has been updated.
    pub num_updates: u64,

    /// Whether the order is an entry order.
    pub is_entry_order: bool,

    /// Whether the order is created on the exchange. This is used to signal whether the exchange is aware of this order.
    pub is_created_on_exchange: bool,

    /// This is used to signal whether the order is finished by cancel.
    pub is_finished_by_cancel: bool,

    /// The pending instruction for this order.
    /// This is used to determine the next action to take for this order.
    pub pending_instruction: Option<ManagedInstruction>,

    /// The pending special instruction for this order.
    /// Special instructions are used to handle special cases like OCO orders.
    pub special_pending_instruction: Option<ManagedInstruction>,

    /// The order metadata.
    pub metadata: OrderMetadata,

    /// The time when the order was initialized.
    pub initialized_at: DateTime<Utc>,

    /// The time when the order was opened (created on the exchange and confirmed).
    pub opened_at: Option<DateTime<Utc>>,

    /// The time when the order was updated anyhow.
    pub updated_at: Option<DateTime<Utc>>,

    /// The time at which the stop was last confirmed (in case this is a trailing stop).
    pub trailing_activated_at: Option<DateTime<Utc>>,

    /// The time at which the stop update was last confirmed (in case this is a trailing stop).
    pub trailing_updated_at: Option<DateTime<Utc>>,

    /// The time when the order was closed (fully filled).
    pub closed_at: Option<DateTime<Utc>>,

    /// The time when the order was cancelled.
    pub cancelled_at: Option<DateTime<Utc>>,

    /// The time when the order was finalized (processed after being closed).
    pub finalized_at: Option<DateTime<Utc>>,
}

impl AsRef<Order> for Order {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsMut<Order> for Order {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

impl Debug for Order {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Order")
            .field("local_order_id", &self.local_order_id)
            .field("local_stop_order_id", &self.local_stop_order_id)
            .field("local_orig_order_id", &self.local_orig_order_id)
            .field("local_list_order_id", &self.local_list_order_id)
            .field("exchange_order_id", &self.exchange_order_id)
            .field("trade_id", &self.trade_id)
            .field("mode", &self.mode)
            .field("pending_mode", &self.pending_mode)
            .field("symbol", &self.symbol)
            .field("atomic_market_price", &self.atomic_market_price)
            // .field("appraiser", &self.appraiser)
            .field("timeframe", &self.timeframe)
            .field("side", &self.side)
            .field("order_type", &self.order_type)
            .field("contingency_order_type", &self.contingency_order_type)
            .field("activator", &self.activator)
            .field("deactivator", &self.deactivator)
            .field(
                "market_price_at_initialization",
                &self.market_price_at_initialization.unwrap_or(f!(0.0)).0,
            )
            .field(
                "market_price_at_opening",
                &self.market_price_at_opening.unwrap_or(f!(0.0)).0,
            )
            .field("price", &self.price.0)
            .field("limit_price", &self.limit_price.unwrap_or(f!(0.0)).0)
            .field("initial_quantity", &self.initial_quantity.0)
            .field(
                "accumulated_filled_quantity",
                &self.accumulated_filled_quantity.0,
            )
            .field("accumulated_filled_quote", &self.accumulated_filled_quote.0)
            .field("fee_ratio", &self.fee_ratio)
            .field("retry_counter", &self.retry_counter)
            .field("leverage", &self.leverage)
            .field("state", &self.state)
            .field("termination_flag", &self.termination_flag)
            .field("cancel_and_idle_flag", &self.cancel_and_idle_flag)
            .field("fills", &self.fills)
            .field("num_updates", &self.num_updates)
            .field("is_created_on_exchange", &self.is_created_on_exchange)
            .field("pending_instruction", &self.pending_instruction)
            .field("metadata", &self.metadata)
            .field("initialized_at", &self.initialized_at)
            .field("opened_at", &self.opened_at)
            .field("updated_at", &self.updated_at)
            .field("trailing_activated_at", &self.trailing_activated_at)
            .field("trailing_updated_at", &self.trailing_updated_at)
            .field("closed_at", &self.closed_at)
            .field("cancelled_at", &self.cancelled_at)
            .field("finalized_at", &self.finalized_at)
            .finish()
    }
}

impl Order {
    pub const ID_RANGE: Range<CoinPairId> = 1..1024000000;

    /*
    pub fn validate_stoploss(&self) -> Result<(), OrderError> {
        if self.local_stop_order_id == 0 {
            return Err(OrderError::StopOrderIdIsZero);
        }

        if self.metadata.stoploss.reference_price <= f!(0.0) {
            return Err(OrderError::InvalidReferencePrice(self.metadata.stoploss.reference_price));
        }

        if self.initial_stop_price <= f!(0.0) {
            return Err(OrderError::InvalidInitialStopPrice(self.initial_stop_price));
        }

        // Ensure that all ratios are within valid range
        if self.limit_price_offset_ratio < f!(0.0) || self.limit_price_offset_ratio > f!(1.0) {
            return Err(OrderError::InvalidLimitPriceOffsetRatio(self.limit_price_offset_ratio));
        }

        if self.initial_trailing_step_ratio < f!(0.0) || self.initial_trailing_step_ratio > f!(1.0) {
            return Err(OrderError::InvalidInitialStepRatio(self.initial_trailing_step_ratio));
        }

        if self.metadata.stoploss.trailing_step_ratio < f!(0.0) || self.metadata.stoploss.trailing_step_ratio > f!(1.0) {
            return Err(OrderError::InvalidTrailingStepRatio(self.metadata.stoploss.trailing_step_ratio));
        }

        // Ensure stop price and limit price are reasonable
        match self.side {
            NormalizedSide::Buy => {
                if self.price <= self.metadata.stoploss.reference_price {
                    return Err(OrderError::InvalidBuyStopPrice {
                        reference_price: self.metadata.stoploss.reference_price,
                        stop_price:      self.price,
                    });
                }

                if self.limit_price <= self.price {
                    return Err(OrderError::InvalidBuyLimitPrice {
                        stop_price:  self.price,
                        limit_price: self.limit_price,
                    });
                }
            }
            NormalizedSide::Sell => {
                if self.price >= self.metadata.stoploss.reference_price {
                    return Err(OrderError::InvalidSellStopPrice {
                        reference_price: self.metadata.stoploss.reference_price,
                        stop_price:      self.price,
                    });
                }

                if self.limit_price >= self.price {
                    return Err(OrderError::InvalidSellLimitPrice {
                        stop_price:  self.price,
                        limit_price: self.limit_price,
                    });
                }
            }
        }

        Ok(())
    }
     */

    /// Validates the order based on the order type.
    /// Performs general and specific validation, depending on the order type.
    pub fn validate(&self) -> Result<(), OrderError> {
        if self.price.is_zero() && self.order_type != NormalizedOrderType::Market {
            return Err(OrderError::ZeroPrice);
        }

        if self.initial_quantity.is_zero() {
            return Err(OrderError::ZeroQuantity);
        }

        if self.local_order_id == 0 {
            return Err(OrderError::LocalOrderIdIsZero);
        }

        // Ensure that all ratios are within valid range
        if self.metadata.limit_price_offset_ratio < f!(0.0)
            || self.metadata.limit_price_offset_ratio > f!(1.0)
        {
            return Err(OrderError::InvalidLimitPriceOffsetRatio(
                self.metadata.limit_price_offset_ratio,
            ));
        }

        if self.is_stoploss() {
            let Some(stoploss_metadata) = &self.metadata.stoploss else {
                return Err(OrderError::StoplossMissingMetadata);
            };

            if stoploss_metadata.reference_price <= f!(0.0) {
                return Err(OrderError::InvalidReferencePrice(
                    stoploss_metadata.reference_price,
                ));
            }

            if stoploss_metadata.initial_stop_price <= f!(0.0) {
                return Err(OrderError::InvalidInitialStopPrice(
                    stoploss_metadata.initial_stop_price,
                ));
            }

            if stoploss_metadata.trailing_step_ratio < f!(0.0)
                || stoploss_metadata.trailing_step_ratio > f!(1.0)
            {
                return Err(OrderError::InvalidTrailingStepRatio(
                    stoploss_metadata.trailing_step_ratio,
                ));
            }

            // Ensure stop price and limit price are reasonable
            match self.side {
                NormalizedSide::Buy => {
                    if self.price <= stoploss_metadata.reference_price {
                        return Err(OrderError::InvalidBuyStopPrice {
                            reference_price: stoploss_metadata.reference_price,
                            stop_price: self.price,
                        });
                    }

                    if let Some(limit_price) = self.limit_price {
                        if limit_price <= self.price {
                            return Err(OrderError::InvalidBuyLimitPrice {
                                stop_price: self.price,
                                limit_price: limit_price,
                            });
                        }
                    }
                }

                NormalizedSide::Sell => {
                    if self.price >= stoploss_metadata.reference_price {
                        return Err(OrderError::InvalidSellStopPrice {
                            reference_price: stoploss_metadata.reference_price,
                            stop_price: self.price,
                        });
                    }

                    if let Some(limit_price) = self.limit_price {
                        if limit_price >= self.price {
                            return Err(OrderError::InvalidSellLimitPrice {
                                stop_price: self.price,
                                limit_price: limit_price,
                            });
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn is_off_cooldown(&self) -> bool {
        self.retry_cooldown.is_available()
    }

    pub fn is_paused(&self) -> bool {
        self.mode == OrderMode::Paused
    }

    pub fn can_be_activated(&mut self) -> bool {
        !self.is_created_on_exchange
            && !self
                .activator
                .check(f!(self.atomic_market_price.load(Ordering::SeqCst)))
    }

    pub fn is_active(&self) -> bool {
        self.is_pending_open()
            || self.is_opened()
            || self.is_partially_filled()
            || self.is_pending_cancel()
            || self.is_cancelled()
            || self.is_cancelled_and_pending_replace()
            || self.is_rejected()
            || self.is_expired()
            || self.is_replace_failed()
            || self.is_replaced()
            || self.is_cancel_failed()
    }

    pub fn is_pending_any(&self) -> bool {
        self.is_pending_open() || self.is_pending_cancel()
    }

    pub fn is_flagged_any(&self) -> bool {
        self.is_flagged_cancel_and_replace()
            || self.is_flagged_cancel_and_idle()
            || self.is_flagged_for_termination()
    }

    pub fn is_idle(&self) -> bool {
        self.state == OrderState::Idle
    }

    pub fn is_instruction_pending_confirmation(&self) -> bool {
        self.pending_instruction.is_some()
    }

    pub fn is_pending_open(&self) -> bool {
        self.state == OrderState::PendingOpen
    }

    pub fn is_pending_cancel(&self) -> bool {
        self.state == OrderState::PendingCancel
    }

    pub fn is_pending_cancel_only(&self) -> bool {
        self.state == OrderState::PendingCancel
            && !self.is_flagged_cancel_and_replace()
            && !self.is_flagged_cancel_and_idle()
    }

    pub fn is_flagged_cancel_and_replace(&self) -> bool {
        self.cancel_and_replace_flag
    }

    pub fn is_flagged_cancel_and_idle(&self) -> bool {
        self.cancel_and_idle_flag
    }

    pub fn is_flagged_for_termination(&self) -> bool {
        self.termination_flag
    }

    pub fn is_opened(&self) -> bool {
        self.state == OrderState::Opened
    }

    pub fn is_partially_filled(&self) -> bool {
        self.state == OrderState::PartiallyFilled
    }

    pub fn is_filled(&self) -> bool {
        self.state == OrderState::Filled
    }

    pub fn is_closed(&self) -> bool {
        self.state == OrderState::Closed
    }

    pub fn is_finished(&self) -> bool {
        self.state == OrderState::Finished
    }

    pub fn is_rejected(&self) -> bool {
        self.state == OrderState::Rejected
    }

    pub fn is_expired(&self) -> bool {
        self.state == OrderState::Expired
    }

    pub fn is_replace_failed(&self) -> bool {
        self.state == OrderState::ReplaceFailed
    }

    pub fn is_replaced(&self) -> bool {
        self.state == OrderState::Replaced
    }

    pub fn is_cancel_failed(&self) -> bool {
        self.state == OrderState::CancelFailed
    }

    pub fn is_cancelled_and_pending_replace(&self) -> bool {
        self.state == OrderState::CancelledAndPendingReplace
    }

    pub fn is_cancelled(&self) -> bool {
        self.state == OrderState::Cancelled
    }

    pub fn is_finished_successfully(&self) -> bool {
        self.state == OrderState::Finished && !self.is_finished_by_cancel
    }

    pub fn is_finished_by_cancel(&self) -> bool {
        self.state == OrderState::Finished && self.is_finished_by_cancel
    }

    pub fn is_failed(&self) -> bool {
        matches!(
            self.state,
            OrderState::OpenFailed | OrderState::ReplaceFailed | OrderState::CancelFailed
        )
    }

    pub fn is_limit_order(&self) -> bool {
        matches!(
            self.order_type,
            NormalizedOrderType::Limit
                | NormalizedOrderType::LimitMaker
                | NormalizedOrderType::TakeProfitLimit
                | NormalizedOrderType::StopLossLimit
        )
    }

    pub fn is_stoploss(&self) -> bool {
        matches!(
            self.order_type,
            NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit
        )
    }

    pub fn has_pending_instruction(&self) -> bool {
        self.pending_instruction.is_some()
    }

    pub fn get_pending_instruction(&self) -> Option<&ManagedInstruction> {
        self.pending_instruction.as_ref()
    }

    pub fn get_pending_instruction_mut(&mut self) -> Option<&mut ManagedInstruction> {
        self.pending_instruction.as_mut()
    }

    pub fn has_special_pending_instruction(&self) -> bool {
        self.special_pending_instruction.is_some()
    }

    pub fn get_special_pending_instruction(&self) -> Option<&ManagedInstruction> {
        self.special_pending_instruction.as_ref()
    }

    pub fn get_special_pending_instruction_mut(&mut self) -> Option<&mut ManagedInstruction> {
        self.special_pending_instruction.as_mut()
    }

    pub fn pause(&mut self) -> Result<(), OrderError> {
        self.set_pending_mode(Some(OrderMode::Paused))
    }

    pub fn resume(&mut self) -> Result<(), OrderError> {
        self.set_pending_mode(Some(OrderMode::Normal))
    }

    pub fn set_pending_mode(&mut self, new_mode: Option<OrderMode>) -> Result<(), OrderError> {
        let Some(new_mode) = new_mode else {
            self.pending_mode = None;
            return self.compute();
        };

        // WARNING: This is a special case and should be handled carefully
        // self.cancel_pending_instruction()?;

        /*
        if self.is_pending_open() {
            /*
            if let Some(pending_instruction) = self.pending_instruction.as_ref() {
                if pending_instruction.state == InstructionState::New {
                    // Clearing the pending instruction
                    self.pending_instruction = None;
                }
            }
             */

            // Resetting the order back to Idle state
            self.set_state(OrderState::Idle)?;
        }
         */

        info!("{}: New pending mode: {:?}", self.order_type, new_mode);

        self.pending_mode = Some(new_mode);
        self.compute()
    }

    pub fn set_trade_id(&mut self, trade_id: HandleId) {
        self.trade_id = trade_id;
    }

    pub fn set_local_id(&mut self, id: OrderId) {
        self.local_order_id = id;
    }

    pub fn set_exchange_order_id(&mut self, id: Option<OrderId>) {
        self.exchange_order_id = id;
    }

    pub fn set_initial_quantity(&mut self, quantity: OrderedValueType) -> Result<(), OrderError> {
        self.initial_quantity = quantity;
        self.compute()
    }

    pub fn set_activator(&mut self, activator: Box<dyn Trigger>) -> Result<(), OrderError> {
        activator.validate()?;
        self.activator = activator;
        self.compute()
    }

    pub fn set_deactivator(&mut self, deactivator: Box<dyn Trigger>) -> Result<(), OrderError> {
        deactivator.validate()?;
        self.deactivator = deactivator;
        self.compute()
    }

    pub fn has_filled_entry_quantity(&self) -> bool {
        !self.accumulated_filled_quantity.is_zero()
            && !self
                .appraiser
                .is_dust_quantity(self.accumulated_filled_quantity.0)
    }

    pub fn untraded_quantity_remainder(&self) -> OrderedValueType {
        f!(self
            .appraiser
            .normalize_base(self.initial_quantity.0 - self.accumulated_filled_quantity.0))
    }

    pub fn has_untraded_quantity_remainder(&self) -> bool {
        !self
            .appraiser
            .is_dust_quantity(self.untraded_quantity_remainder().0)
    }

    pub fn terminate(&mut self) -> Result<(), OrderError> {
        self.termination_flag = true;
        self.compute()
    }

    pub fn set_retry_counter(&mut self, counter: Countdown<u8>) {
        self.retry_counter = counter;
    }

    fn set_state(&mut self, new_state: OrderState) -> Result<(), OrderError> {
        use OrderState::*;
        let old_state = self.state;

        // If there is no change in the state of the order due to repeated updates (if ever happens)
        if self.state == new_state {
            return Ok(());
        }

        // Define valid state transitions
        self.state = match (self.state, new_state) {
            // An Idle order can only be moved to PendingOpen or Cancelled
            (Idle, PendingOpen) => PendingOpen, // The order is now waiting for execution
            (Idle, Cancelled) => Cancelled,     // The order is canceled without being executed

            // WARNING: When the order that is pending to be idle is canceled, it moves to Idle state instead of Cancelled
            (PendingCancel, Idle) => Idle, // The order has been canceled and is now idle

            // If the order opening fails, it can be moved back to PendingOpen or can be Cancelled
            (PendingOpen, OpenFailed) => OpenFailed,
            (OpenFailed, PendingOpen) => PendingOpen,
            (OpenFailed, Cancelled) => PendingOpen,
            // An order moves from PendingOpen to Opened when it is being executed
            (PendingOpen, Opened) => Opened,

            // An order can be reset back to Idle during the opening attempts (e.g., when the pending
            // instruction is New and hasn't started processing yet)
            (PendingOpen, Idle) => Idle,

            // An order can be canceled during the opening attempts
            (PendingOpen, Cancelled) => Cancelled,

            // An Opened order can be partially filled, fully filled or move to a PendingCancel state
            (Opened, PartiallyFilled) => PartiallyFilled, // The order is partially filled
            (Opened, Filled) => Filled,                   // The order is fully filled
            (Opened, PendingCancel) => PendingCancel,     // The order is on its way to be canceled

            // If the order is canceled from the outside during the execution
            (Opened, Cancelled) => Cancelled,

            // If cancellation was pending, the order moves to Cancelled state
            (PendingCancel, Cancelled) => Cancelled,
            (PendingCancel, Opened) => Opened, // It is possible to revert the cancellation before it starts
            (Cancelled, Idle) => Idle,
            (Cancelled, Finished) => Finished,

            // WARNING: This is a special case and should be handled carefully
            // This is a special case, where the order is cancelled but the replacement is still pending.
            //
            // CASE IN POINT:
            // The system wants to replace an order and sends a request `cancelReplace` to the exchange.
            // At this point the system knows that the original order needs to be
            // cancelled first and then replaced with a new order. So the system is expects
            // a Cancelled response from the exchange and a follow up with a new order.
            (PendingCancel, CancelledAndPendingReplace) => CancelledAndPendingReplace,
            (Cancelled, CancelledAndPendingReplace) => CancelledAndPendingReplace, // The order is canceled and pending to be replaced
            (CancelledAndPendingReplace, Replaced) => Replaced,

            // A replaced order can only be moved to ReplaceFailed or Replaced states
            // After that, it can be moved to Filled or Opened state
            (Cancelled, ReplaceFailed) => ReplaceFailed,
            (Cancelled, Replaced) => Replaced,

            (Replaced, Filled) => Filled,
            (ReplaceFailed, Opened) => Opened,
            (Replaced, Opened) => Opened,

            // A partially filled order can again be partially filled or canceled
            // Once an order is fully filled, it moves to the Closed state and then to Idle or Finished
            (PartiallyFilled, PartiallyFilled) => PartiallyFilled,
            (PartiallyFilled, PendingCancel) => PendingCancel,
            (PartiallyFilled, Cancelled) => Cancelled,
            (PartiallyFilled, Filled) => Filled,
            (Filled, Closed) => Closed,
            (Closed, Idle) => Idle,
            (Closed, Finished) => Finished,

            // For any other state transition, the order is considered invalid
            _ => {
                error!(
                    "invalid order state transition {:?} -> {:?}",
                    self.state, new_state
                );
                return Err(OrderError::InvalidOrderStateTransition {
                    from: self.state,
                    to: new_state,
                });
            }
        };

        if self.state != old_state {
            info!(
                "{}: Order state transition: {:?} -> {:?}",
                self.order_type, old_state, self.state
            );
            self.updated_at = Some(Utc::now());
        }

        self.compute()
    }

    pub fn confirm_instruction_execution(
        &mut self,
        result: InstructionExecutionResult,
    ) -> Result<(), OrderError> {
        // info!("ORDER: Confirming instruction execution result: {:#?}", result);

        let Some(pending_instruction) = &mut self.pending_instruction else {
            return Err(OrderError::OrderHasNoPendingInstruction);
        };

        // TODO: Add entry to the changelog
        if pending_instruction.id == result.instruction_id {
            match result.outcome {
                Some(outcome) => {
                    match &outcome {
                        ExecutionOutcome::Success => {
                            self.updated_at = Some(Utc::now());
                            pending_instruction.set_state(InstructionState::Executed)?;
                            pending_instruction.executed_at = result.executed_at;

                            // NOTE: Clearing the pending instruction here is vital to avoid double execution
                            self.clear_pending_instruction();
                        }

                        ExecutionOutcome::Failure(desc) => {
                            pending_instruction.set_state(InstructionState::Failed)?;

                            // IMPORTANT: If the instruction failed, we should start terminating procedure
                            // TODO: Decide if I should terminate the order here or not
                            error!("Pending instruction {} failed with status {:?}, terminating order...", result.instruction_id, outcome);
                            self.termination_flag = true;

                            // FIXME: I'm not sure if I should clear the pending instruction here or not
                            // self.pending_instruction = None;
                        }
                    }
                }

                None => {
                    error!("Instruction execution result has no outcome, this should never happen");
                    return Err(OrderError::NoInstructionExecutionOutcome);
                }
            }
        } else {
            error!(
                "Pending instruction {} failed with status {:?}, but the order has a different pending instruction {}",
                result.instruction_id, result.outcome, pending_instruction.id
            );
            return Err(OrderError::WrongInstructionExecutionResult(
                self.local_order_id,
                pending_instruction.clone(),
                result,
            ));
        }

        self.compute()
    }

    // TODO: I'm not sure I need this
    pub fn confirm_instruction_execution_failure(&mut self, reason: &str) {
        if let Some(pending_instruction) = &mut self.pending_instruction {
            if pending_instruction.retry_cooldown.is_some() {
                // Double the retry delay for the next attempt, for example
                pending_instruction.retry_cooldown =
                    Some(pending_instruction.retry_cooldown.unwrap() * 2);
            }

            // Check in with the countdown; if it returns false, then we're out of retries
            if !pending_instruction.retries.checkin() {
                // If no retries left, decide on further actions. This could involve:
                // 1. Setting an internal flag to disable this Stop from attempting further actions.
                // 2. Logging the error.
                // 3. Sending an alert/notification.
                // Or any combination of the above.

                // For the purpose of this example, we will just log the error
                error!(
                    "Failed to process instruction for {}: {}",
                    self.symbol, reason
                );
            }
        }
    }

    pub fn confirm_opened(&mut self, exchange_order_id: Option<OrderId>) -> Result<(), OrderError> {
        if self.state == OrderState::Opened {
            warn!("Order {} is already opened", self.local_order_id);
            return Ok(());
        }

        self.set_state(OrderState::Opened)?;
        self.exchange_order_id = exchange_order_id;
        self.is_created_on_exchange = true;
        self.opened_at = Some(Utc::now());
        self.updated_at = Some(Utc::now());

        info!("{}: Order {} opened", self.order_type, self.local_order_id);

        // FIXME: remove this
        // eprintln!("CONFIRM_OPENED:");
        // eprintln!("self.local_order_id = {:#?}", self.local_order_id);
        // eprintln!("SmartId(self.local_order_id).decode() = {:#?}", SmartId(self.local_order_id).decode());
        // eprintln!("exchange_order_id = {:#?}", exchange_order_id);

        self.compute()
    }

    pub fn confirm_open_failed(&mut self) -> Result<(), OrderError> {
        if self.state == OrderState::OpenFailed {
            warn!("Order {} has already failed to open", self.local_order_id);
            return Ok(());
        }

        self.set_state(OrderState::OpenFailed)?;
        self.compute()
    }

    pub fn confirm_replace_failed(&mut self) -> Result<(), OrderError> {
        if self.state == OrderState::ReplaceFailed {
            warn!(
                "Order {} has already failed to replace",
                self.local_order_id
            );
            return Ok(());
        }

        self.set_state(OrderState::ReplaceFailed)?;
        self.compute()
    }

    pub fn confirm_filled(&mut self, latest_fill: OrderFill) -> Result<(), OrderError> {
        if self.state == OrderState::Filled {
            warn!("Order {} is already filled", self.local_order_id);
            return Ok(());
        }

        latest_fill.validate()?;

        match latest_fill.status {
            OrderFillStatus::PartiallyFilled => {
                self.accumulated_filled_quantity = latest_fill.metadata.accumulated_filled_quantity;
                self.accumulated_filled_quote = latest_fill.metadata.accumulated_filled_quote;
                self.fills.push(latest_fill);
                self.updated_at = Some(Utc::now());
                self.set_state(OrderState::PartiallyFilled)?;
            }
            OrderFillStatus::Filled => {
                self.accumulated_filled_quantity = latest_fill.metadata.accumulated_filled_quantity;
                self.accumulated_filled_quote = latest_fill.metadata.accumulated_filled_quote;
                self.fills.push(latest_fill);
                self.updated_at = Some(Utc::now());
                self.set_state(OrderState::Filled)?;
            }
        }

        info!("{}: Order {} filled", self.order_type, self.local_order_id);

        self.compute()
    }

    pub fn confirm_cancelled(&mut self) -> Result<(), OrderError> {
        if matches!(
            self.state,
            OrderState::Cancelled | OrderState::CancelledAndPendingReplace
        ) {
            warn!(
                "{}: Order {} is already cancelled: {:?}",
                self.order_type, self.local_order_id, self.state
            );
            return Ok(());
        }

        // Handling cancellation based on the flags
        if self.is_flagged_cancel_and_replace() {
            self.cancel_and_replace_flag = false;
            self.set_state(OrderState::CancelledAndPendingReplace)?;
            info!(
                "{}: Order {} cancelled and pending replace",
                self.order_type, self.local_order_id
            );
        } else if self.is_flagged_cancel_and_idle() {
            info!(
                "{}: Order {} cancelled and pending idle",
                self.order_type, self.local_order_id
            );
            self.cancel_and_idle_flag = false;
            self.pause()?;
            self.set_state(OrderState::Idle)?;
        } else {
            self.set_state(OrderState::Cancelled)?;
            info!(
                "{}: Order {} cancelled",
                self.order_type, self.local_order_id
            );
        }

        // Since the order is cancelled, we need to clear the flag that
        // indicates that the order is created on the exchange
        self.is_created_on_exchange = false;

        Ok(())
    }

    pub fn confirm_cancel_failed(&mut self) -> Result<(), OrderError> {
        if self.state == OrderState::CancelFailed {
            warn!("Order {} has already failed to cancel", self.local_order_id);
            return Ok(());
        }

        self.set_state(OrderState::CancelFailed)
    }

    pub fn confirm_rejected(&mut self) -> Result<(), OrderError> {
        if self.state == OrderState::Rejected {
            warn!("Order {} is already rejected", self.local_order_id);
            return Ok(());
        }

        self.set_state(OrderState::Rejected)
    }

    pub fn confirm_expired(&mut self) -> Result<(), OrderError> {
        if self.state == OrderState::Expired {
            warn!("Order {} is already expired", self.local_order_id);
            return Ok(());
        }

        self.set_state(OrderState::Expired)
    }

    pub fn confirm_replaced(&mut self, event: NormalizedOrderEvent) -> Result<(), OrderError> {
        if self.state == OrderState::Replaced {
            warn!("Order {} is already replaced", self.local_order_id);
            return Ok(());
        }

        if !self.is_cancelled_and_pending_replace() {
            return Err(OrderError::InvalidReplaceConfirmationState(
                self.state,
                OrderState::CancelledAndPendingReplace,
            ));
        }

        // eprintln!("REPLACEMENT DATA:");
        // eprintln!("event = {:#?}", event);
        // eprintln!("self = {:#?}", self);

        // TODO: adjust activator and deactivator
        self.is_created_on_exchange = true;
        self.exchange_order_id = Some(event.exchange_order_id);
        self.local_order_id = event.local_order_id;
        self.accumulated_filled_quantity = event.cumulative_filled_quantity;
        self.accumulated_filled_quote = event.cumulative_quote_asset_transacted_quantity;
        self.order_type = event.order_type;
        self.price = event.order_price;
        self.limit_price = None; // FIXME: Adapt limit price based on the original limit price and the price delta
        self.initial_quantity = event.quantity;
        // self.pending_instruction = None; // FIXME: This is not correct, we need to keep the pending instruction
        self.num_updates = SmartId(self.local_order_id).sequence_number() as u64;
        self.updated_at = Some(Utc::now());

        info!(
            "{}: Order {} replaced",
            self.order_type, self.local_order_id
        );

        self.set_state(OrderState::Replaced)
    }

    pub fn compute_next_instruction(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        use OrderState::*;

        // NOTE: handling current order state and producing signals
        match self.state {
            Idle => self.handle_idle(),
            PendingOpen => self.handle_pending_open(),
            Rejected => self.handle_instruction_failed(),
            OpenFailed => self.handle_open_failed(),
            Opened | PartiallyFilled => self.handle_opened_and_partially_filled(),
            PendingCancel => Ok(None), // Nothing to do here since it's waiting for the cancellation
            CancelFailed => self.handle_cancel_failed(),
            CancelledAndPendingReplace => Ok(None), // Nothing to do here since it's waiting for the replacement
            Expired | Cancelled => self.handle_cancelled(), // WARNING: handling Expired as Cancelled (unexpectedly),
            ReplaceFailed => self.handle_replace_failed(),
            Replaced => self.handle_replaced(),
            Filled => self.handle_filled(),
            Closed => self.handle_closed(),
            Finished => self.handle_finished(),
        }
    }

    pub fn compute(&mut self) -> Result<(), OrderError> {
        // If the order is not created on the exchange and the activator says NO, then do nothing
        // IMPORTANT: The activator is used to determine if the order should be created on the exchange
        if !self.is_created_on_exchange
            && !self
                .activator
                .check(f!(self.atomic_market_price.load(Ordering::SeqCst)))
        {
            return Ok(());
        }

        // IMPORTANT: Handling the state of the existing instruction
        if let Some(pending_instruction) = &mut self.pending_instruction {
            match pending_instruction.state {
                InstructionState::New => {
                    // If the existing instruction is still new, we need to check if it's expired
                    // TODO: Implement this
                }

                InstructionState::Running => {
                    // If the instruction is Running, it means it's already underway,
                    // so we better let it run its course
                }

                InstructionState::Executed => {
                    // If the existing instruction is executed, we need to finalize it
                    println!(
                        "CLEARING EXECUTED INSTRUCTION: {:#?}",
                        self.pending_instruction
                    );
                    self.clear_pending_instruction();
                }

                InstructionState::Failed => {
                    // If the existing instruction is failed, we need to cancel it
                    println!(
                        "CLEARING FAILED INSTRUCTION: {:#?}",
                        self.pending_instruction
                    );
                    self.clear_pending_instruction();
                }
            }
        }

        // Computing the next instruction to execute for this order
        let next_instruction = self.compute_next_instruction()?;

        // NOTE: A case where we need to cancel the existing instruction and replace it with a new one
        let mut cancel_and_reset_instruction = None;

        // TODO: check if we need to update the instruction or override, otherwise do nothing
        match (&self.pending_instruction, next_instruction) {
            // If there is no existing instruction and there is a new instruction, then set it
            (None, Some(new_instruction)) => {
                info!("NEW INSTRUCTION: {:#?}", new_instruction);
                // self.pending_instruction = Some(new_instruction);
                self.set_pending_instruction(new_instruction);

                // IMPORTANT: Recomputing the order state after setting the new instruction
                return self.compute();
            }

            // If there is an existing instruction and there is no new instruction, then do nothing
            (Some(existing_instruction), None) => return Ok(()),

            // If there is an existing instruction and there is a new instruction, then check for conflicts
            // and update the existing instruction if possible
            (Some(existing_instruction), Some(new_instruction)) => {
                info!("NEW INSTRUCTION WHILE STILL GOT EXISTING:");
                info!("CONFLICT: EXISTING INSTRUCTION {:#?}", existing_instruction);
                info!("CONFLICT: NEW INSTRUCTION {:#?}", new_instruction);

                // If the existing instruction is still new and the new instruction is
                // of the same kind, then we can just update the existing instruction
                // before it starts processing
                if existing_instruction.state == InstructionState::New {
                    if existing_instruction.is_new()
                        && existing_instruction.kind() == new_instruction.kind()
                    {
                        info!("UPDATING NEW INSTRUCTION: {:#?}", new_instruction);
                        // self.pending_instruction = Some(new_instruction);
                        self.set_pending_instruction(new_instruction);
                        return self.compute();
                    } else if existing_instruction.is_new()
                        && existing_instruction.kind() != new_instruction.kind()
                        && existing_instruction.priority < new_instruction.priority
                    {
                        // If the existing instruction is new but the new instruction is of a different kind
                        // and has a higher priority, then we need to cancel the existing instruction
                        // and set the new instruction
                        info!(
                            "FLAGGING EXISTING INSTRUCTION TO BE CANCELLED: {:#?}",
                            existing_instruction
                        );
                        cancel_and_reset_instruction = Some(new_instruction);
                    }
                }
            }

            (None, None) => {}
        }

        // WARNING:
        // If we got here, it means that there is an existing instruction that needs to be cancelled
        // and replaced with a new instruction. This is a special case and should be handled carefully.
        if let Some(new_instruction) = cancel_and_reset_instruction {
            info!(
                "CANCELING PENDING AND REPLACING WITH NEW INSTRUCTION: {:#?}",
                new_instruction
            );
            self.cancel_pending_instruction()?;
            self.set_pending_instruction(new_instruction);
            self.compute()
        } else {
            Ok(())
        }
    }

    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    // TODO: Find a way to prevent stop from trailing if the next price is too close to the exit price
    pub fn compute_stoploss(
        &mut self,
        current_market_price: OrderedValueType,
    ) -> Result<Option<ManagedInstruction>, OrderError> {
        // IMPORTANT: Need to prevent the stoploss from being updated if the order is on hold
        if self.is_paused() {
            return Ok(None);
        }

        if !self.is_stoploss() {
            return Ok(None);
        }

        let Some(stoploss_metadata) = &self.metadata.stoploss else {
            return Err(OrderError::StoplossMissingMetadata);
        };

        // If activator allows it, continue
        if !self.activator.check(current_market_price) {
            return Ok(None);
        }

        // Paranoid check to ensure that the current market price is valid
        if current_market_price <= f!(0.0) {
            return Err(OrderError::InvalidCurrentMarketPrice(current_market_price));
        }

        if self.pending_instruction.is_some() {
            // FIXME: maybe stoploss shouldn't be updated if there's a pending instruction
            return Ok(None);
        }

        // NOTE: At this point we know that the stop is created on the exchange

        // ----------------------------------------------------------------------------
        // Trailing stoploss logic; checking whether the stoploss needs to be updated
        // ----------------------------------------------------------------------------

        // The amount of price change that is required to activate trailing
        let initial_step_amount =
            stoploss_metadata.reference_price * stoploss_metadata.trailing_step_ratio;

        // The amount of price change that is required to trigger an update
        let trailing_step_amount =
            stoploss_metadata.reference_price * stoploss_metadata.trailing_step_ratio;

        // The amount of price change that has occurred since the trailing stop was activated
        // FIXME: revert back if this doesn't work
        // FIXME: This seems to be that what was causing the issue with trailing stop all along
        // let delta_amount = f!((current_market_price - self.price - initial_step_amount).abs());
        let delta_amount = current_market_price - self.price - initial_step_amount;

        // The ratio of price change that has occurred since the trailing stop was activated
        // NOTE: This is the ratio of the delta amount relative to the reference price (entry price)
        let delta_ratio = delta_amount / stoploss_metadata.reference_price;

        // Calculate how many "steps" the price has jumped
        let num_steps = {
            let num_steps = f!((delta_ratio / stoploss_metadata.trailing_step_ratio).floor());

            if num_steps <= f!(1.0) {
                f!(1.0)
            } else {
                num_steps
            }
        };

        /*
        eprintln!("current_market_price = {:#?}", current_market_price.0);
        // eprintln!("stoploss_metadata.reference_price = {:#?}", stoploss_metadata.reference_price.0);
        // eprintln!("self.initial_trailing_step_ratio = {:#?}", self.initial_trailing_step_ratio.0);
        // eprintln!("stoploss_metadata.trailing_step_ratio = {:#?}", stoploss_metadata.trailing_step_ratio.0);
        // eprintln!("trailing_step_amount = {:#?}", trailing_step_amount.0);
        // eprintln!("self.trailing_activation_price = {:#?}", self.trailing_activation_price.0);
        eprintln!("delta_amount = {:#?}", delta_amount.0);
        eprintln!("delta_ratio = {:#?}", delta_ratio.0);
        eprintln!("self.price = {:#?}", self.price.0);
        eprintln!("self.limit_price = {:#?}", self.limit_price.unwrap_or(f!(0.0)).0);
        eprintln!("initial_step_amount = {:#?}", initial_step_amount);
         */

        if stoploss_metadata.is_trailing_enabled
            && delta_ratio >= stoploss_metadata.trailing_step_ratio
        {
            // Calculate the total adjustment for the stop price based on the number of steps
            let total_trailing_delta = stoploss_metadata.reference_price
                * stoploss_metadata.trailing_step_ratio
                * num_steps;
            let excess_amount = delta_amount - total_trailing_delta;
            let excess_ratio = excess_amount / stoploss_metadata.reference_price;

            // eprintln!("compute_stoploss: market_price = {} self.side = {}", current_market_price.0, self.side);

            let mut new_stop_price = match self.side {
                NormalizedSide::Buy => self.price - total_trailing_delta,
                NormalizedSide::Sell => self.price + total_trailing_delta,
            };

            let new_limit_price = if self.order_type == NormalizedOrderType::StopLossLimit {
                Some(match self.side {
                    NormalizedSide::Buy => {
                        new_stop_price + new_stop_price * self.metadata.limit_price_offset_ratio
                    }
                    NormalizedSide::Sell => {
                        new_stop_price - new_stop_price * self.metadata.limit_price_offset_ratio
                    }
                })
            } else {
                None
            };

            /*
            eprintln!("excess_amount = {:#?}", excess_amount.0);
            eprintln!("excess_ratio = {:#?}", excess_ratio.0);
            eprintln!("num_steps = {:#?}", num_steps);
            eprintln!("total_trailing_delta = {:#?}", total_trailing_delta.0);
            eprintln!("new_stop_price = {:#?}", new_stop_price.0);
            eprintln!("new_limit_price = {:#?}", new_limit_price.unwrap_or(f!(0.0)).0);
             */

            // ----------------------------------------------------------------------------
            // Updating the state to PendingReplace before sending the cancel/replace instruction,
            // so that the order is not handled as just being Cancelled when the order update event
            // is received from the exchange. This way the system will know that cancellation is
            // followed by a replacement.
            //
            // IMPORTANT:
            // This is necessary to avoid the order being cancelled/closed prematurely,
            // potentially causing the Trade to be closed prematurely as well.
            // ----------------------------------------------------------------------------
            warn!(
                "{}: Stoploss {} is pending to trail to new_stop_price={} new_stop_limit_price={}",
                self.order_type,
                self.local_order_id,
                self.appraiser.round_quote(new_stop_price.0),
                self.appraiser
                    .round_quote(new_limit_price.unwrap_or(f!(0.0)).0)
            );

            self.cancel_and_replace_flag = true;
            self.set_state(OrderState::PendingCancel)?;

            // eprintln!("COMPARING UPDATED STOP PRICES");
            // eprintln!("compute_stoploss: new_stop_price = {:#?}, new_limit_price = {:#?}", new_stop_price.0, new_limit_price.unwrap_or_default());
            // let (new_stop_price2, new_limit_price2) = self.next_stop_update()?;
            // eprintln!("next_stop_update: new_stop_price = {:#?}, new_limit_price = {:#?}", new_stop_price2.0, new_limit_price2.0);

            // Set or update (if possible) the pending instruction
            self.set_pending_instruction(
                self.instruction_cancel_replace_stoploss(new_stop_price, new_limit_price)?,
            );
        }

        if self.pending_instruction.is_some() {
            Ok(self.pending_instruction.clone())
        } else {
            Ok(None)
        }
    }

    /// TODO: Verify that this is correct
    /// Returns the next proposed stop price and limit price update.
    pub fn next_stop_update(&self) -> Result<(OrderedValueType, OrderedValueType), OrderError> {
        if !self.is_stoploss() {
            return Err(OrderError::OrderIsNotStoploss);
        }

        let Some(stoploss_metadata) = &self.metadata.stoploss else {
            return Err(OrderError::StoplossMissingMetadata);
        };

        let trailing_step_amount =
            stoploss_metadata.trailing_step_ratio * stoploss_metadata.reference_price;

        let next_stop_price = match self.side {
            NormalizedSide::Buy => self.price - trailing_step_amount,
            NormalizedSide::Sell => self.price + trailing_step_amount,
        };

        // This is the amount that the limit price is offset from the stop price (in the direction of the stop side)
        let limit_price_offset = next_stop_price * self.metadata.limit_price_offset_ratio;

        let next_limit_price = match self.side {
            NormalizedSide::Buy => next_stop_price + limit_price_offset,
            NormalizedSide::Sell => next_stop_price - limit_price_offset,
        };

        let new_limit_price = if self.order_type == NormalizedOrderType::StopLossLimit {
            Some(match self.side {
                NormalizedSide::Buy => {
                    next_stop_price + next_stop_price * self.metadata.limit_price_offset_ratio
                }
                NormalizedSide::Sell => {
                    next_stop_price - next_stop_price * self.metadata.limit_price_offset_ratio
                }
            })
        } else {
            None
        };

        Ok((next_stop_price, next_limit_price))
    }

    pub fn set_pending_instruction(&mut self, new_instruction: ManagedInstruction) {
        if let Some(pending_instruction) = &self.pending_instruction {
            if !pending_instruction.is_new() {
                // If the pending instruction is not new, then it means it is being processed right now, we can't override it
                return;
            }

            if pending_instruction.metadata.kind == new_instruction.metadata.kind {
                // If the pending instruction is the same kind as the new instruction, then we can override it
                self.pending_instruction = Some(new_instruction);
                return;
            } else {
                // Determining which instruction is more important
                if new_instruction.priority > pending_instruction.priority {
                    // If the new instruction has higher priority, then we can override it before it's processed
                    self.pending_instruction = Some(new_instruction);
                    return;
                } else {
                    // If the new instruction is not of higher priority, then we just ignore it (for now)
                    return;
                }
            }
        } else {
            // Regular logic to set a pending instruction
            self.pending_instruction = Some(new_instruction);
        }
    }

    /// An explicit way to clear the pending instruction after it's executed
    pub fn clear_pending_instruction(&mut self) {
        self.pending_instruction = None;
    }

    /// An attempt to cancel the pending instruction before it's executed
    pub fn cancel_pending_instruction(&mut self) -> Result<(), OrderError> {
        // If the order is already flagged for termination, then it's better to just let it be
        if self.is_flagged_for_termination() {
            return Ok(());
        }

        let Some(pending_instruction) = self.pending_instruction.as_ref() else {
            // If no pending instruction exists, there's nothing to cancel.
            return Ok(());
        };

        // The pending instruction represents an 'intention' to perform an action.
        // This could be an action that is about to start or is already in progress.
        // We need to ensure we only interrupt actions that haven't started yet.
        if !pending_instruction.is_new() {
            // If the instruction is already being processed, do not interrupt.
            // This prevents partial execution of actions, ensuring either complete execution or none at all.
            return Err(OrderError::CannotCancelActivePendingInstruction(
                self.trade_id,
                self.local_order_id,
                pending_instruction.clone(),
            ));
        }

        warn!(
            "{}: Order {} is cancelling pending instruction",
            self.order_type, self.local_order_id
        );

        // By clearing the pending instruction here, we are effectively stopping any planned actions
        // before they commence. This is especially relevant for actions like cancel-and-replace,
        // which should either be executed in full or not at all.
        // Clearing it here ensures that if the action hasn't started, it won't start at all.
        self.clear_pending_instruction();

        // Revert the order state based on the nature of the pending instruction:
        // - If the instruction was to open the order, we revert to the Idle state.
        // - If it was to cancel or replace the order, we revert to the Opened state.
        // This state reset is important to reflect the cancellation of the intended action.
        if self.is_pending_open() {
            info!(
                "{}: Order {} is reverting to Idle state after cancelling pending open",
                self.order_type, self.local_order_id
            );
            self.set_state(OrderState::Idle)?;
        } else if self.is_pending_cancel_only() {
            info!(
                "{}: Order {} is reverting to Opened state after cancelling pending cancel",
                self.order_type, self.local_order_id
            );
            self.set_state(OrderState::Opened)?;
        } else if self.is_pending_cancel() && self.cancel_and_replace_flag {
            info!("{}: Order {} is reverting to Opened state after cancelling pending cancel-and-replace", self.order_type, self.local_order_id);
            // For cancel or cancel-and-replace instructions, revert to Opened state.
            self.set_state(OrderState::Opened)?;
            self.cancel_and_replace_flag = false;
        } else if self.is_pending_cancel() && self.cancel_and_idle_flag {
            info!("{}: Order {} is reverting to Opened state after cancelling pending cancel-and-idle", self.order_type, self.local_order_id);
            self.set_state(OrderState::Opened)?;
            self.cancel_and_idle_flag = false;
        }

        // Finally, recompute the order's state. This step reassesses the current conditions
        // and ensures that the order's state accurately reflects its actual status in the system.
        self.compute()
    }

    pub fn instruction_open(&self) -> ManagedInstruction {
        // ----------------------------------------------------------------------------
        // WARNING: Overriding the opening instruction if it's a stoploss
        // ----------------------------------------------------------------------------
        if self.is_stoploss() {
            return self.instruction_stoploss();
        }

        let a = self.appraiser;
        let initial_quantity = f!(a.round_normalized_base(self.initial_quantity.0));
        let price = f!(a.round_normalized_quote(self.price.0));

        // ----------------------------------------------------------------------------
        // Creating the opening instruction
        // ----------------------------------------------------------------------------
        let instruction = match (self.side, self.order_type) {
            (NormalizedSide::Buy, NormalizedOrderType::Limit) => Instruction::LimitBuy {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
                order_type: self.order_type,
                quantity: initial_quantity,
                price: price,
                is_post_only: false,
            },

            (NormalizedSide::Buy, NormalizedOrderType::LimitMaker) => Instruction::LimitBuy {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
                order_type: self.order_type,
                quantity: initial_quantity,
                price: price,
                is_post_only: true,
            },

            (NormalizedSide::Buy, NormalizedOrderType::Market) => Instruction::MarketBuy {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
                quantity: initial_quantity,
            },

            (NormalizedSide::Sell, NormalizedOrderType::Limit) => Instruction::LimitSell {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
                order_type: self.order_type,
                quantity: initial_quantity,
                price: price,
                is_post_only: false,
            },

            (NormalizedSide::Sell, NormalizedOrderType::LimitMaker) => Instruction::LimitSell {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
                order_type: self.order_type,
                quantity: initial_quantity,
                price: price,
                is_post_only: true,
            },

            (NormalizedSide::Sell, NormalizedOrderType::Market) => Instruction::MarketSell {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
                quantity: initial_quantity,
            },

            _ => {
                panic!("UNSUPPORTED REGULAR ORDER TYPE = {:#?}", self);
            }
        };

        // Initializing the opening instruction
        ManagedInstruction::new(
            self.symbol,
            Priority::Normal,
            InstructionMetadata {
                is_special: false,
                kind: InstructionKind::CreateOrder,
                trade_id: Some(self.trade_id),
                local_order_id: Some(self.local_order_id),
                local_stop_order_id: None,
                process_id: None,
            },
            instruction,
            None,
            None,
        )
    }

    fn instruction_stoploss(&self) -> ManagedInstruction {
        let a = self.appraiser;
        let price = f!(a.round_normalized_quote(self.price.0));
        let limit_price = self.limit_price.map(|p| f!(a.round_quote(p.0)));
        let initial_quantity = f!(a.round_normalized_base(self.initial_quantity.0));

        let instruction = match (self.side, self.order_type) {
            (NormalizedSide::Buy, NormalizedOrderType::StopLoss) => Instruction::CreateStop {
                symbol: self.symbol,
                order_type: self.order_type,
                side: self.side,
                local_order_id: self.local_order_id,
                local_stop_order_id: self.local_stop_order_id,
                stop_price: price,
                price: limit_price,
                quantity: initial_quantity,
            },

            (NormalizedSide::Buy, NormalizedOrderType::StopLossLimit) => Instruction::CreateStop {
                symbol: self.symbol,
                order_type: self.order_type,
                side: self.side,
                local_order_id: self.local_order_id,
                local_stop_order_id: self.local_stop_order_id,
                stop_price: price,
                price: limit_price,
                quantity: initial_quantity,
            },

            (NormalizedSide::Sell, NormalizedOrderType::StopLoss) => Instruction::CreateStop {
                symbol: self.symbol,
                order_type: self.order_type,
                side: self.side,
                local_order_id: self.local_order_id,
                local_stop_order_id: self.local_stop_order_id,
                stop_price: price,
                price: limit_price,
                quantity: initial_quantity,
            },

            (NormalizedSide::Sell, NormalizedOrderType::StopLossLimit) => Instruction::CreateStop {
                symbol: self.symbol,
                order_type: self.order_type,
                side: self.side,
                local_order_id: self.local_order_id,
                local_stop_order_id: self.local_stop_order_id,
                stop_price: price,
                price: limit_price,
                quantity: initial_quantity,
            },

            _ => {
                panic!("UNSUPPORTED STOPLOSS ORDER TYPE = {:#?}", self);
            }
        };

        ManagedInstruction::new(
            self.symbol,
            Priority::High,
            InstructionMetadata {
                is_special: false,
                kind: InstructionKind::CreateOrder,
                trade_id: self.trade_id.to_u16(),
                local_order_id: self.local_order_id.to_u64(),
                local_stop_order_id: self.local_stop_order_id,
                process_id: None,
            },
            instruction,
            None,
            None,
        )
    }

    fn instruction_cancel_replace_stoploss(
        &self,
        new_stop_price: OrderedValueType,
        new_limit_price: Option<OrderedValueType>,
    ) -> Result<ManagedInstruction, OrderError> {
        Ok(ManagedInstruction::new(
            self.symbol,
            Priority::Immediate,
            InstructionMetadata {
                is_special: false,
                kind: InstructionKind::CancelReplaceOrder,
                trade_id: self.trade_id.to_u16(),
                local_order_id: Some(self.local_order_id),
                local_stop_order_id: self.local_stop_order_id,
                process_id: None,
            },
            Instruction::CancelReplaceOrder {
                symbol: self.symbol,
                local_original_order_id: self.local_order_id,
                exchange_order_id: None,
                cancel_order_id: None, // I don't know what to do with it yet
                new_client_order_id: SmartId(self.local_order_id).new_incremented_id()?,
                new_order_type: self.order_type,
                new_time_in_force: self.time_in_force,
                new_quantity: self.untraded_quantity_remainder(),
                new_price: f!(self.appraiser.round_normalized_quote(new_stop_price.0)),
                new_limit_price: new_limit_price
                    .map(|p| f!(self.appraiser.round_normalized_quote(p.0))),
            },
            None,
            None,
        ))
    }

    pub fn instruction_cancel(&self) -> ManagedInstruction {
        ManagedInstruction::new(
            self.symbol,
            Priority::Immediate,
            InstructionMetadata {
                is_special: false,
                kind: InstructionKind::CancelOrder,
                trade_id: Some(self.trade_id),
                local_order_id: Some(self.local_order_id),
                local_stop_order_id: self.local_stop_order_id,
                process_id: None,
            },
            Instruction::CancelOrder {
                symbol: self.symbol,
                local_order_id: self.local_order_id,
            },
            None,
            None,
        )
    }

    pub fn update_filled_both_amounts(&mut self, qty: OrderedValueType, quote: OrderedValueType) {
        self.update_filled_qty(qty);
        self.update_filled_quote(quote);
    }

    fn update_filled_quote(&mut self, quote: OrderedValueType) {
        self.accumulated_filled_quote = f!(self.appraiser.normalize_quote(quote.0));
    }

    fn update_filled_qty(&mut self, qty: OrderedValueType) {
        self.accumulated_filled_quantity = f!(self.appraiser.normalize_base(qty.0));
    }

    fn cancel(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        self.set_state(OrderState::PendingCancel)?;
        Ok(Some(self.instruction_cancel()))
    }

    pub fn cancel_and_idle(&mut self) -> Result<(), OrderError> {
        if self.is_flagged_cancel_and_idle() {
            warn!("Attempting to cancel and idle an order that is already flagged to be cancelled and idle");
            return Ok(());
        }

        warn!(
            "{}: Order {} is flagged to be cancelled and idle",
            self.order_type, self.local_order_id
        );
        self.cancel_and_idle_flag = true;
        self.compute()
    }

    fn handle_instruction_failed(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        self.set_state(OrderState::Rejected)?;
        self.close()
    }

    fn handle_idle(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if let Some(pending_mode) = self.pending_mode.take() {
            if self.mode == pending_mode {
                return Ok(None);
            }

            info!(
                "{}: Order {} is switching mode from {:?} to {:?}",
                self.order_type, self.local_order_id, self.mode, pending_mode
            );
            self.mode = pending_mode;
        }

        if self.is_paused() {
            return Ok(None);
        }

        if self.is_flagged_for_termination()
            || self.is_flagged_cancel_and_replace()
            || self.is_flagged_cancel_and_idle()
        {
            return Ok(None);
        }

        self.set_state(OrderState::PendingOpen)?;

        Ok(Some(self.instruction_open()))
    }

    fn handle_pending_open(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        Ok(None)
    }

    fn handle_open_failed(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        warn!(
            "{}: Order {} failed to open",
            self.order_type, self.local_order_id
        );

        if self.is_flagged_for_termination() {
            return Ok(None);
        }

        // Paranoid check
        if !self.is_off_cooldown() {
            error!("CRITICAL: The order has somehow failed to open while still on cooldown, terminating...");
            self.terminate()?;
            return Ok(None);
        }

        // Terminate the order if it has failed to open too many times
        if self.retry_counter.checkin() {
            warn!(
                "{}: Order {} has failed to open too many times, terminating...",
                self.order_type, self.local_order_id
            );
            self.terminate()?;
            return Ok(None);
        }

        // Getting to this point means there are still retries left
        self.set_state(OrderState::PendingOpen)?;

        Ok(Some(self.instruction_open()))
    }

    fn handle_deactivation(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if self.is_created_on_exchange
            && !self
                .activator
                .check(f!(self.atomic_market_price.load(Ordering::SeqCst)))
        {
            return Ok(None);
        }

        let Some(action) = self.lifecycle.on_deactivation else {
            return Ok(None);
        };

        match action {
            OrderLifecycleAction::CancelAndMarketExit => {}
            OrderLifecycleAction::Cancel => {}
            OrderLifecycleAction::CancelAndResumeStoplossOrMarketExit => {}
        }

        Ok(None)
    }

    fn handle_opened_and_partially_filled(
        &mut self,
    ) -> Result<Option<ManagedInstruction>, OrderError> {
        let current_market_price = f!(self.atomic_market_price.load(Ordering::SeqCst));

        // IMPORTANT: Handling the deactivation of the order (if necessary)
        // if let Some(instruction) = self.handle_deactivation()? {
        //     return Ok(Some(instruction));
        // }

        // IMPORTANT: Paranoid check, better safe than sorry
        if current_market_price.is_zero() {
            return Ok(None);
        }

        // NOTE: `cancel_and_replace_flag` is not used here because it has a different instruction
        if self.termination_flag || self.cancel_and_idle_flag {
            return self.cancel();
        }

        // Computing the stoploss update (only happens if this order is a stoploss)
        self.compute_stoploss(current_market_price)
    }

    fn handle_cancel_failed(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        error!("Cancel failed for order {}", self.local_order_id);

        // TODO: implement contingency logic here

        Ok(None)
    }

    /// if the cancel event has arrived from outside the system,
    /// then it's unhandled internally, otherwise can be set straight to Cancelled
    /// WARNING: the system may decide whether to dump quantity (if it has any)
    fn handle_cancelled(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        self.set_state(OrderState::Finished)?;
        self.is_finished_by_cancel = true;
        self.cancelled_at = Some(Utc::now());
        self.compute_next_instruction()
    }

    fn handle_filled(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if self.state != OrderState::Filled {
            panic!("Attempting to handle order without Filled state");
        }

        self.set_state(OrderState::Closed)?;
        self.compute()?;
        self.compute_next_instruction()
    }

    fn handle_replaced(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if self.state != OrderState::Replaced {
            panic!("Attempting to handle order without Replaced state");
        }

        info!(
            "{}: Replace succeeded for order {}",
            self.order_type, self.local_order_id
        );

        self.set_state(OrderState::Opened)?;
        self.compute_next_instruction()
    }

    fn handle_replace_failed(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if self.state != OrderState::ReplaceFailed {
            panic!("Attempting to handle order without ReplaceFailed state");
        }

        error!("Replace failed for order {}", self.local_order_id);

        // TODO: implement contingency logic here

        Ok(None)
    }

    fn handle_closed(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if self.state != OrderState::Closed {
            panic!("Attempting to handle order without Closed state");
        }

        self.is_created_on_exchange = false;
        self.set_state(OrderState::Finished)?;
        self.compute_next_instruction()
    }

    fn handle_finished(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        if self.state != OrderState::Finished {
            panic!("attempting to handle order without Finished state");
        }

        // TODO: some post-finish actions

        self.close()
    }

    pub fn status(&self) -> Status {
        match self.state {
            OrderState::Closed | OrderState::Finished => Status::Ok,
            OrderState::Cancelled => Status::Cancelled,
            OrderState::OpenFailed => Status::Failed,
            OrderState::Rejected => Status::Failed,
            OrderState::Expired => Status::Failed,
            _ => Status::Unknown,
        }
    }

    fn close(&mut self) -> Result<Option<ManagedInstruction>, OrderError> {
        // Preventing repeated finalization
        if self.is_closed() {
            return Ok(None);
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    /*
    #[test]
    fn test_new_stop_no_activator() {
        let mut stop = Stoploss::new(
            ustr("TESTUSDT"),
            new_id_u64(),
            f!(100.0),
            false,
            NormalizedSide::Sell,
            OrderedFloat(11.0),
            OrderedFloat(10.0),
            OrderedFloat(0.1),
            Box::new(NoTrigger::new()),
            OrderedFloat(0.01),
            OrderedFloat(0.02),
        );

        // MUST NOT produce any instruction
        let instruction = stop.compute(OrderedFloat(8.0));
        assert_eq!(stop.activator.metadata().kind(), TriggerKind::None);
        assert_eq!(instruction, Ok(None));
    }

    #[test]
    fn test_new_stop_with_instant_activator() {
        let mut stop = Stoploss::new(
            ustr("TESTUSDT"),
            new_id_u64(),
            f!(100.0),
            false,
            NormalizedSide::Sell,
            OrderedFloat(11.0),
            OrderedFloat(10.0),
            OrderedFloat(0.1),
            Box::new(InstantTrigger::new()),
            OrderedFloat(0.01),
            OrderedFloat(0.02),
        );

        // MUST produce a create stop instruction
        let instruction = stop.compute(OrderedFloat(8.0));
        assert!(instruction.is_ok());

        let instruction = instruction.unwrap();
        assert!(instruction.is_some());

        let instruction = instruction.unwrap();

        assert_eq!(stop.activator.metadata().kind(), TriggerKind::Instant);
        assert_eq!(instruction.priority, Priority::High);
        assert_eq!(instruction.metadata.kind, InstructionKind::CreateStop);
        assert_eq!(instruction.metadata.stop_order_id, Some(stop.id));
        assert_eq!(instruction.metadata.local_stop_order_id, Some(stop.local_stop_order_id));
        assert_eq!(instruction.metadata.local_order_id, None);
        assert_eq!(instruction.metadata.trade_id, None);
        assert_eq!(instruction.state, InstructionState::New);

        if let Instruction::CreateStop {
            symbol,
            local_stop_order_id,
            stop_price,
            limit_price,
            ..
        } = &instruction.instruction
        {
            assert_eq!(symbol, &stop.symbol);
            assert_eq!(local_stop_order_id, &stop.local_stop_order_id);
            assert!(approx_eq(stop_price.0, 10.0, 0.1));
            assert!(approx_eq(limit_price.0, 9.0, 0.1));
        }
    }

    #[test]
    fn test_trailing_stop_flow_for_sell_position() {
        let mut stop = Stoploss::new(
            ustr("TESTUSDT"),
            None,
            f!(100.0),
            false,
            NormalizedSide::Sell,
            OrderedFloat(100.0), // reference price
            OrderedFloat(98.0),  // initial stop price
            OrderedFloat(0.001), // limit offset ratio
            Box::new(InstantTrigger::new()),
            true,
            OrderedFloat(0.01), // initial step ratio
            OrderedFloat(0.02), // trailing step ratio
        );

        with_stop_explanation!(stop, {
            // First call to compute should immediately produce a create stop instruction
            let initial_instruction = stop.compute(add_ratio!(stop.reference_price, 0.005, 2));
            assert!(initial_instruction.is_ok());

            let initial_instruction = initial_instruction.unwrap();
            assert!(initial_instruction.is_some());
        });

        with_stop_explanation!(stop, {
            stop.confirm_stop_created();
        });

        with_stop_explanation!(stop, {
            // Next, let's simulate a scenario where no action should take place.
            // Assuming a slight price movement might not cause a stop update.
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.006, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_stop_explanation!(stop, {
            // Now, simulate a scenario where the price has moved enough to activate the trailing logic.
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.011, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_stop_explanation!(stop, {
            stop.confirm_stop_updated();
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.0195, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.021, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_stop_explanation!(stop, {
            stop.confirm_stop_updated();
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.0195, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.021, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_stop_explanation!(stop, {
            // Confirm the stop update
            stop.confirm_stop_updated();
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.020 * 2.5, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_stop_explanation!(stop, {
            // Confirm the stop update
            stop.confirm_stop_updated();
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.020 * 7.5, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_stop_explanation!(stop, {
            stop.confirm_stop_updated();
        });

        with_stop_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.027 * 1.5, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_stop_explanation!(stop, {
            // Confirm the stop update
            stop.confirm_stop_updated();
        });
    }
     */

    /*
    #[test]
    fn test_trailing_stop_flow_for_sell_position() {
        let mut stop = Stoploss::new(
            ustr("TESTUSDT"),
            new_id_u64(),
            f!(100.0),
            false,
            NormalizedSide::Sell,
            OrderedFloat(100.0), // reference price
            OrderedFloat(98.0),  // initial stop price
            OrderedFloat(0.001), // limit offset ratio
            Box::new(InstantTrigger::new()),
            OrderedFloat(0.01), // initial step ratio
            OrderedFloat(0.02),
        );

        with_order_explanation!(stop, {
            // First call to compute should immediately produce a create stop instruction
            let initial_instruction = stop.compute(add_ratio!(stop.reference_price, 0.005, 2));
            assert!(initial_instruction.is_ok());

            let initial_instruction = initial_instruction.unwrap();
            assert!(initial_instruction.is_some());
        });

        with_order_explanation!(stop, {
            stop.confirm_stop_created();
        });

        with_order_explanation!(stop, {
            // Next, let's simulate a scenario where no action should take place.
            // Assuming a slight price movement might not cause a stop update.
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.006, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_order_explanation!(stop, {
            // Now, simulate a scenario where the price has moved enough to activate the trailing logic.
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.011, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_order_explanation!(stop, {
            stop.confirm_stop_updated();
        });

        with_order_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.0195, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_order_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.reference_price, 0.021, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_order_explanation!(stop, {
            stop.confirm_stop_updated();
        });

        with_order_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.stop_price, 0.0195, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_order_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.stop_price, 0.031, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });

        with_order_explanation!(stop, {
            stop.confirm_stop_updated();
        });

        with_order_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.stop_price, 0.0195, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_none());
        });

        with_order_explanation!(stop, {
            let instruction = stop.compute(add_ratio!(stop.stop_price, 0.031, 2));
            assert!(instruction.is_ok());
            let instruction = instruction.unwrap();
            assert!(instruction.is_some());
        });
    }
     */
}
