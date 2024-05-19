/*
use crate::{
    appraiser::Appraiser,
    coin::CoinContext,
    coin_worker::CoinWorkerEvent,
    order::{Order, OrderError, OrderState},
    order_grid::Lifetime,
    roundtrip::RoundtripState::Cancelled,
    roundtrip_metadata::RoundtripMetadata,
    trader::TradeError,
    trader_directive::{Directive, Instruction},
    trigger::Trigger,
    types::{
        NormalizedOrderType::{self, *},
        NormalizedSide::{self, *},
        OrderedValueType, Quantity, Status,
    },
    util::CoinId,
};
use crossbeam::channel::Sender;
use log::{debug, error, info, warn};
use num_traits::{zero, Zero};
use ordered_float::OrderedFloat;
use parking_lot::RwLock;
use pausable_clock::PausableClock;
use std::{
    fmt,
    fmt::{Debug, Display, Formatter},
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use ustr::Ustr;
use yata::core::ValueType;

#[derive(Error, Debug)]
pub enum RoundtripError {
    #[error("Unsupported: {0}")]
    Unsupported(String),

    #[error("No entry price")]
    NoEntryPrice,

    #[error("No exit price")]
    NoExitPrice,

    #[error("No stop trigger price (stop is set but has no trigger price)")]
    NoStopTriggerPrice,

    #[error(transparent)]
    OrderError(#[from] OrderError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum RoundtripMode {
    #[default]
    Once,
    Perpetual,
}

impl Display for RoundtripMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RoundtripMode::Once => write!(f, "Once"),
            RoundtripMode::Perpetual => write!(f, "Perpetual"),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum TradingPhase {
    #[default]
    Pending,
    Entering,
    Exiting,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum RoundtripState {
    #[default]
    Pending,
    EntryPending,
    EntryFailed,
    EntryOpened,
    EntryClosed,
    ExitPending,
    ExitFailed,
    ExitOpened,
    ExitClosed,
    PendingCancel,
    Cancelled,
    Interrupted,
    Failed,
    Finished,
}

pub struct Roundtrip {
    /// Unique identifier for the roundtrip
    pub id: String,

    /// A reference to the coin context.
    pub coin_context: Arc<RwLock<CoinContext>>,

    /// Indicates whether this roundtrip is completed or not
    pub is_finalized: bool,

    /// Symbol for the pair of assets involved in the trade
    pub symbol: Ustr,

    /// The latest known market price of the asset being traded
    pub latest_known_market_price: OrderedValueType,

    /// Responsible for determining the price of the asset
    pub appraiser: Appraiser,

    /// The side (Buy or Sell) the roundtrip initially enters the trade
    pub initial_trade_side: NormalizedSide,

    /// The side (Buy or Sell) on which the roundtrip is currently active
    pub current_trade_side: NormalizedSide,

    /// Metadata for the order that starts the trade
    pub entry: Order,

    /// Metadata for the order that ends the trade
    pub exit: Order,

    /// The `exit_quantity_ratio` field represents the portion of the total entry quantity
    /// that should be exited when a sell or short order is executed.
    /// It is expressed as a relative value where 1.0 represents 100% of the entered quantity.
    ///
    /// For example, if `exit_quantity_ratio` is set to 0.5 and the entry quantity was 100 units,
    /// the exit order would aim to sell or short 50 units.
    ///
    /// Note that due to market conditions and trading rules, the actual executed quantity
    /// may not be exactly as specified by this ratio. Therefore, it is recommended
    /// to handle the potential discrepancies in the trading logic.
    pub exit_quantity_ratio: OrderedValueType,

    /// Price adjustment to be used in case of entry or exit failure.
    /// WARNING: Must be handled carefully to prevent balance depletion. This amount should be significantly small.
    pub price_adjustment: OrderedValueType,

    /// The current state of the roundtrip
    pub roundtrip_state: RoundtripState,

    /// The current state of the order within the roundtrip
    pub order_state: OrderState,

    /// The time at which the order was initialized or reset after a successful entry
    pub lifetime: Instant,

    /// Duration to wait before processing a queued order
    pub queue_waiting_duration: Duration,

    /// Duration to wait before canceling an active order
    pub active_order_duration: Duration,

    /// Indicates whether to execute a market sell if all retries fail
    pub market_exit_on_retry_fail: bool,

    /// Indicates if the roundtrip should cancel instead of replace/move an order
    pub cancel_over_replace: bool,

    /// The operational mode of the roundtrip
    pub mode: RoundtripMode,

    /// Indicates if the roundtrip is marked for termination by the user
    pub user_termination_mark: bool,

    /// This flag is only useful when the roundtrip is being
    /// cancelled, to know whether it's a CancelReplace as a single
    /// operation or a 2-step cancel then exit.
    pub is_cancel_exit_in_progress: bool,

    /// A flag indicating whether an unexpected order cancellation has occurred.
    /// This flag is set when an order is cancelled from an external source.
    pub is_unexpectedly_cancelled: bool,

    /// A flag indicating whether an emergency entry (e.g., market entry) had to be made.
    pub had_emergency_entry: bool,

    /// A flag indicating whether an emergency exit (e.g., market exit) had to be made.
    pub had_emergency_exit: bool,

    /// A flag indicating whether to execute a contingency exit when an unexpected cancel event is received.
    /// NOTE: This could be triggered externally, e.g. user or exchange cancels the order.
    pub execute_contingency_exit_on_cancel: bool,

    /// The number of successful roundtrips made
    pub successful_roundtrip_count: usize,

    /// Triggers an immediate and unconditional emergency exit on the next cycle,
    /// disrupting the normal processing flow. This should only be enabled in critical
    /// situations where an immediate exit is absolutely necessary.
    pub trigger_emergency_exit_on_next_compute: bool,

    /// Indicates if the process of an emergency exit has already been initiated.
    /// This can be used to prevent duplicate exit actions or handle state changes
    /// that are dependent on the commencement of an emergency exit.
    pub emergency_exit_in_progress: bool,

    /// Channel for sending feedback to the coin worker
    pub coin_worker_feedback_sender: Option<Sender<CoinWorkerEvent>>,

    /// Name of the support/resistance used as a reference point to avoid multiple orders on the same level
    pub support_resistance_reference_tag: Option<&'static str>,
}

impl Roundtrip {
    pub const ID_RANGE: Range<CoinId> = 1024000000..2048000000;

    pub fn new(
        id: String,
        symbol: Ustr,
        coin_context: Arc<RwLock<CoinContext>>,
        appraiser: Appraiser,
        mode: RoundtripMode,
        entry: Order,
        exit: Order,
        price_step: OrderedValueType,
        cancel_over_replace: bool,
        feedback_sender: Option<Sender<CoinWorkerEvent>>,
    ) -> Result<Self, RoundtripError> {
        Ok(Self {
            id,
            coin_context,
            is_finalized: false,
            symbol,
            latest_known_market_price: Default::default(),
            appraiser,
            initial_trade_side: entry.side,
            current_trade_side: entry.side,
            entry,
            exit,
            exit_quantity_ratio: OrderedFloat(1.0),
            price_adjustment: price_step,
            roundtrip_state: RoundtripState::Pending,
            order_state: OrderState::Inactive,
            lifetime: Instant::now(),
            queue_waiting_duration: Duration::from_secs(1),
            active_order_duration: Duration::from_secs(10),
            market_exit_on_retry_fail: true,
            user_termination_mark: false,
            is_cancel_exit_in_progress: false,
            is_unexpectedly_cancelled: false,
            had_emergency_entry: false,
            had_emergency_exit: false,
            execute_contingency_exit_on_cancel: true,
            mode,
            successful_roundtrip_count: 0,
            trigger_emergency_exit_on_next_compute: false,
            emergency_exit_in_progress: false,
            coin_worker_feedback_sender: feedback_sender,
            support_resistance_reference_tag: None,
            cancel_over_replace,
        })
    }

    pub fn validate(&self) -> Result<(), TradeError> {
        if self.id.is_empty() {
            return Err(TradeError::EmptyRoundtripId(self.id.clone()));
        }

        self.entry.validate()?;

        if self.entry.price.is_zero() {
            return Err(TradeError::ZeroEntryPrice);
        }

        if self.exit.price.is_zero() {
            return Err(TradeError::ZeroExitPrice);
        }

        Ok(())
    }

    pub fn order_state(&self) -> OrderState {
        match self.phase() {
            TradingPhase::Pending | TradingPhase::Entering => self.entry.state,
            TradingPhase::Exiting => self.exit.state,
        }
    }

    pub fn metadata(&self) -> RoundtripMetadata {
        RoundtripMetadata {
            tag:                        self.support_resistance_reference_tag,
            entry:                      &self.entry,
            exit:                       &self.exit,
            exit_ratio:                 0.0,
            has_been_moved_or_replaced: self.has_been_moved,
        }
    }

    pub fn pin_tag(&mut self, name: &'static str) {
        self.support_resistance_reference_tag = Some(name);
    }

    pub fn clear_tag(&mut self) {
        self.support_resistance_reference_tag = None;
    }

    fn is_perpetual(&self) -> bool {
        self.mode == RoundtripMode::Perpetual
    }

    fn has_filled_entry_quantity(&self) -> bool {
        !self.filled_entry_quantity.is_zero() && !self.appraiser.is_dust_quantity(self.filled_entry_quantity.0)
    }

    fn has_filled_exit_quantity(&self) -> bool {
        !self.filled_exit_quantity.is_zero() && !self.appraiser.is_dust_quantity(self.filled_exit_quantity.0)
    }

    fn untraded_quantity_remainder(&self) -> OrderedValueType {
        OrderedFloat(self.appraiser.normalize_quantity(self.filled_entry_qty.0 - self.filled_exit_qty.0))
    }

    fn has_untraded_qty_remainder(&self) -> bool {
        !self.appraiser.is_dust_qty(self.untraded_qty_remainder().0)
    }

    fn terminate(&mut self) {
        self.user_termination_mark = true;
    }

    fn reset(&mut self) -> Result<(), TradeError> {
        // incrementing cycle count
        self.successful_roundtrip_count += 1;

        // now using the filled quote as an entry amount
        self.entry_qty = OrderedFloat(self.appraiser.normalize_qty((self.filled_quote / self.entry_price).0));

        // resetting filled values
        self.filled_entry_qty = zero();
        self.filled_exit_qty = zero();
        self.filled_quote = zero();

        // resetting retries
        self.entry_retries_left.0 = self.entry_retries_left.1;
        self.exit_retries_left.0 = self.exit_retries_left.1;

        // resetting states
        self.set_roundtrip_state(RoundtripState::Pending);
        self.set_order_state(OrderState::Inactive);

        // resetting back to the entry side
        self.current_trade_side = self.initial_trade_side;

        // resetting delay
        self.next_normal_delay = Some(Duration::from_millis(300));

        Ok(())
    }

    /// if the current order side is the same as an entry side,
    /// then it's in the `Entering` phase, otherwise the phase is `Exiting`
    /// NOTE: the phase is `Inactive` if the roundtrip itself is `Pending`
    pub fn phase(&self) -> TradingPhase {
        if self.current_trade_side == self.initial_trade_side {
            // if the roundtrip is pending to be processed,
            // then the phase is considered to be inactive
            if self.roundtrip_state == RoundtripState::Pending {
                TradingPhase::Pending
            } else {
                TradingPhase::Entering
            }
        } else {
            TradingPhase::Exiting
        }
    }

    fn switch_side(&mut self) -> Result<(), TradeError> {
        self.current_trade_side = match (self.initial_trade_side, self.current_trade_side) {
            (Buy, Buy) => Sell,
            (Sell, Sell) => Buy,
            _ => self.current_trade_side,
        };

        // resetting possible delay
        self.next_normal_delay = Some(Duration::from_millis(300));

        Ok(())
    }

    fn set_roundtrip_state(&mut self, new_state: RoundtripState) -> Result<RoundtripState, TradeError> {
        use RoundtripState::*;
        let old_state = self.roundtrip_state;

        // If there is no change in the state of the order (possibly due to repeated updates from the exchange)
        if self.state == new_state {
            return Ok(());
        }

        // listing only valid state transitions
        self.roundtrip_state = match (self.roundtrip_state, new_state) {
            // initial unprocessed state
            (Pending, EntryPending) => EntryPending,
            (EntryPending, Pending) => Pending,

            // roundtrip may be cancelled even before it is processed
            (Pending, Cancelled) => Cancelled,

            // roundtrip may also be cancelled during retries
            (EntryPending, Cancelled) => Cancelled,

            // entry attempt with a retry loop
            (EntryPending, EntryFailed) => EntryFailed,
            (EntryFailed, EntryPending) => EntryPending,

            // entry trade is successfully opened
            (EntryPending, EntryOpened) => EntryOpened,
            (EntryPending, EntryClosed) => EntryClosed,
            (EntryOpened, EntryClosed) => EntryClosed,
            (EntryOpened, Cancelled) => Cancelled,

            // entry position is successfully closed, switching sides
            (EntryClosed, ExitPending) => ExitPending,

            // this is to retry a failed exit
            (ExitPending, EntryClosed) => EntryClosed,

            // roundtrip may be cancelled when trying to exit normally
            (ExitPending, Cancelled) => Cancelled,

            // exit attempt with a retry loop
            (ExitPending, ExitFailed) => ExitFailed,
            (ExitFailed, ExitPending) => ExitPending,
            (ExitPending, ExitOpened) => ExitOpened,
            (ExitPending, ExitClosed) => ExitClosed,
            (ExitOpened, ExitClosed) => ExitClosed,
            (ExitOpened, Cancelled) => Cancelled,

            // allow reset if perpetual or finish upon closure
            (ExitClosed, Pending) => Pending,
            (ExitClosed, Finished) => Finished,

            (_, Failed) => Failed,
            (_, PendingCancel) => PendingCancel,
            (PendingCancel, Cancelled) => Cancelled,
            (_, Interrupted) => Interrupted,

            _ => {
                panic!("invalid roundtrip state transition {:?} -> {:?}", self.roundtrip_state, new_state);
                /*
                return Err(TraderError::InvalidRoundtripStateTransition {
                    from: self.roundtrip_state,
                    to: new_state,
                });
                 */
            }
        };

        // println!("Roundtrip: {:?} -> {:?}", old_state, new_state);

        Ok(self.roundtrip_state)
    }

    fn set_order_state(&mut self, new_state: OrderState) -> Result<OrderState, TradeError> {
        use OrderState::*;
        let old_state = self.order_state;


        // If there is no change in the state of the order (possibly due to repeated updates from the exchange)
        if self.state == new_state {
            return Ok(());
        }

        // listing only valid state transitions
        self.order_state = match (self.order_state, new_state) {
            (Inactive, Pending) => Pending,

            // cancel may be invoked even before it attempts to open
            (Inactive, Cancelled) => Cancelled,

            (Pending, OpenFailed) => OpenFailed,
            (OpenFailed, Pending) => Pending,
            (OpenFailed, Cancelled) => Pending,
            (Pending, Opened) => Opened,

            // cancel may be invoked during opening attempts too
            (Pending, Cancelled) => Cancelled,

            (Opened, PartiallyFilled) => PartiallyFilled,
            (Opened, Filled) => Filled,
            (Opened, PendingCancel) => PendingCancel,

            // support for cancels invoked from outside
            (Opened, Cancelled) => Cancelled,

            (PendingCancel, Cancelled) => Cancelled,
            // (PendingCancel, Replaced) => Replaced,
            (Cancelled, Inactive) => Inactive,
            (Cancelled, PendingReplace) => PendingReplace,
            (Cancelled, Finished) => Finished,

            // WARNING: replaced order is simply moved, so it has to go its normal way again
            // NOTE: Replaced order can only be Filled at the moment
            (PendingReplace, ReplaceFailed) => ReplaceFailed,
            (PendingReplace, Replaced) => Replaced,
            (Replaced, Filled) => Filled,
            (ReplaceFailed, Opened) => Opened,
            (Replaced, Opened) => Opened,

            (PartiallyFilled, PartiallyFilled) => PartiallyFilled,
            (PartiallyFilled, PendingCancel) => PendingCancel,

            // support for cancels invoked from outside
            (PartiallyFilled, Cancelled) => Cancelled,

            (PartiallyFilled, Filled) => Filled,
            (Filled, Closed) => Closed,
            (Closed, Inactive) => Inactive,
            (Closed, Finished) => Finished,

            _ => {
                panic!("invalid order state transition {:?} -> {:?}", self.order_state, new_state);
                /*
                return Err(TraderError::InvalidOrderStateTransition {
                    from: self.order_state,
                    to: new_state,
                });
                 */
            }
        };

        // println!("Order: {:?} -> {:?}", old_state, new_state);

        Ok(self.order_state)
    }

    /// WARNING: this is vital to safe stopping, to minimize losses on fees (at least during zero maker fee programs)
    pub fn set_market_price(&mut self, market_price: OrderedValueType) {
        if market_price.is_zero() {
            panic!("attempted to set market price to zero, has the apocalypse begun?");
        }

        self.latest_known_market_price = self.appraiser.normalize_quote(market_price).into();
    }

    /// WARNING: this is vital to safe (in terms of fee losses) stopping, to minimize losses on fees (at least during zero maker fee programs)
    /// NOTE: `stop exit price` is used to determine when to exit the position, it is not the same as `stop loss price`
    ///  which is used to move order closer to the market price instead of immediate exit (to avoid fees).
    ///  This is useful mostly if not only when there is a zero maker fee program and the move is made
    ///  using LimitMaker order type. In this case the order is "moved" closer to the market price.
    pub fn set_stop_exit_price_for_move(&mut self, stop_exit_price: OrderedValueType) {
        if stop_exit_price.is_zero() {
            panic!("attempted to set stop exit price for replace to zero, this could end very badly");
        }

        self.stop_exit_price_for_move = self.appraiser.normalize_quote(stop_exit_price).into();
    }

    pub fn adjust_prices(&mut self) {
        match self.phase() {
            TradingPhase::Pending | TradingPhase::Entering => match self.initial_trade_side {
                Buy =>
                    self.entry_price = self
                        .appraiser
                        .normalize_quote(self.entry_price.min(self.latest_known_market_price) - (self.price_adjustment * 2.0))
                        .into(),
                Sell =>
                    self.entry_price = self
                        .appraiser
                        .normalize_quote(self.entry_price.max(self.latest_known_market_price) + (self.price_adjustment * 1.5))
                        .into(),
            },
            TradingPhase::Exiting => match self.initial_trade_side {
                Buy =>
                    self.exit_price = self
                        .appraiser
                        .normalize_quote(self.exit_price.min(self.latest_known_market_price) + (self.price_adjustment * 2.0))
                        .into(),
                Sell =>
                    self.exit_price = self
                        .appraiser
                        .normalize_quote(self.exit_price.max(self.latest_known_market_price) - (self.price_adjustment * 1.5))
                        .into(),
            },
        }
    }

    pub fn update_filled_amount(&mut self, amount: Quantity) {
        match amount {
            Quantity::Quote(quote) => self.update_filled_quote(quote),
            Quantity::Base(qty) => self.update_filled_qty(qty),
        }
    }

    pub fn update_filled_both_amounts(&mut self, qty: OrderedValueType, quote: OrderedValueType) {
        self.update_filled_qty(qty);
        self.update_filled_quote(quote);
    }

    fn update_filled_quote(&mut self, quote: OrderedValueType) {
        self.filled_quote = OrderedFloat(self.appraiser.normalize_quote(quote.0));
    }

    fn update_filled_qty(&mut self, qty: OrderedValueType) {
        match self.phase() {
            TradingPhase::Pending | TradingPhase::Entering => self.filled_entry_qty = OrderedFloat(self.appraiser.normalize_qty(qty.0)),
            TradingPhase::Exiting => self.filled_exit_qty = OrderedFloat(self.appraiser.normalize_qty(qty.0)),
        }
    }

    pub fn notify_opened(&mut self) -> Result<(), TradeError> {
        if self.order_state == OrderState::Opened {
            return Ok(());
        }

        self.set_order_state(OrderState::Opened)?;

        match self.phase() {
            TradingPhase::Pending => unimplemented!("notify_opened(): unhandled pending roundtrip state"),
            TradingPhase::Entering => {
                self.set_roundtrip_state(RoundtripState::EntryOpened)?;
            }
            TradingPhase::Exiting => {
                self.set_roundtrip_state(RoundtripState::ExitOpened)?;
            }
        }

        Ok(())
    }

    pub fn notify_open_failed(&mut self) -> Result<(), TradeError> {
        self.set_order_state(OrderState::OpenFailed)?;
        Ok(())
    }

    pub fn notify_replace_failed(&mut self) -> Result<(), TradeError> {
        self.set_order_state(OrderState::ReplaceFailed)?;
        Ok(())
    }

    pub fn notify_partially_filled(&mut self, quote: Option<OrderedValueType>, qty: Option<OrderedValueType>) -> Result<(), TradeError> {
        if quote.is_none() && qty.is_none() {
            return Err(TradeError::InvalidOrderFill);
        }

        self.set_order_state(OrderState::PartiallyFilled)?;

        if let Some(quote) = quote {
            self.update_filled_quote(quote);
        }

        if let Some(qty) = qty {
            self.update_filled_qty(qty);
        }

        Ok(())
    }

    pub fn notify_filled(&mut self, quote: Option<OrderedValueType>, qty: Option<OrderedValueType>) -> Result<(), TradeError> {
        if self.order_state == OrderState::Filled {
            return Ok(());
        }

        if quote.is_none() && qty.is_none() {
            return Err(TradeError::InvalidOrderFill);
        }

        if let Some(quote) = quote {
            self.update_filled_quote(quote);
        }

        if let Some(qty) = qty {
            self.update_filled_qty(qty);
        }

        self.set_order_state(OrderState::Filled)?;

        Ok(())
    }

    pub fn notify_cancelled(&mut self) -> Result<(), TradeError> {
        // NOTE: do nothing if CancelReplace is pending
        if self.order_state == OrderState::PendingReplace && !self.cancel_over_replace {
            dbg!("ATTEMPTING TO CANCEL ROUNDTRIP WHICH IS PENDING FOR REPLACE");
            println!("{:#?}", self);
            return Ok(());
        }

        if self.order_state == OrderState::Cancelled {
            return Ok(());
        }

        // NOTE: if there is no pending for cancel, then it means
        //  it was triggered externally
        if self.order_state != OrderState::PendingCancel {
            self.is_unexpectedly_cancelled = true;
            return Ok(());
        }

        // WARNING: an edge case that sets state according to whether
        //  this roundtrip has any quantity filled during entry phase.
        if self.has_untraded_qty_remainder() {
            self.is_cancel_exit_in_progress = true;
        }

        self.set_roundtrip_state(RoundtripState::PendingCancel)?;
        self.set_order_state(OrderState::Cancelled)?;

        Ok(())
    }

    pub fn notify_cancel_failed(&mut self) -> Result<(), TradeError> {
        self.set_order_state(OrderState::CancelFailed)?;
        Ok(())
    }

    pub fn notify_cancelled_and_pending_replace(&mut self) -> Result<(), TradeError> {
        if self.order_state == OrderState::PendingReplace {
            return Ok(());
        }

        self.set_order_state(OrderState::PendingReplace)?;
        Ok(())
    }

    pub fn notify_rejected(&mut self) -> Result<(), TradeError> {
        if self.order_state == OrderState::Rejected {
            return Ok(());
        }

        self.set_order_state(OrderState::Rejected)?;

        Ok(())
    }

    pub fn notify_expired(&mut self) -> Result<(), TradeError> {
        if self.order_state == OrderState::Expired {
            return Ok(());
        }

        self.set_order_state(OrderState::Expired)?;

        Ok(())
    }

    pub fn notify_closed(&mut self) -> Result<(), TradeError> {
        if self.order_state == OrderState::Closed {
            return Ok(());
        }

        self.set_order_state(OrderState::Closed)?;

        Ok(())
    }

    pub fn notify_replaced(&mut self) -> Result<(), TradeError> {
        if self.order_state == OrderState::Replaced {
            return Ok(());
        }

        self.set_order_state(OrderState::Replaced)?;

        Ok(())
    }

    pub fn compute(&mut self) -> Result<Directive, TradeError> {
        use OrderState::*;

        // WARNING: handling the emergency exit case
        if self.trigger_emergency_exit_on_next_compute && !self.emergency_exit_in_progress {
            return self.emergency_exit_trade();
        }

        // NOTE: handling current order state and producing signals
        Ok(match self.order_state {
            Inactive => self.handle_inactive()?,
            Pending => self.handle_pending()?,
            Rejected | OpenFailed => self.handle_open_failed()?,
            Opened | PartiallyFilled => self.handle_opened_and_partially_filled()?,
            PendingCancel => Directive::empty(),
            CancelFailed => self.handle_cancel_failed()?,

            // WARNING: handling Expired as Cancelled (unexpectedly),
            Expired | Cancelled => self.handle_cancelled()?,

            Filled => self.handle_filled()?,
            Closed => self.handle_closed()?,
            Finished => self.handle_finished()?,

            PendingReplace => Directive::empty(),
            ReplaceFailed => self.handle_replace_failed()?,
            Replaced => self.handle_replaced()?,
        })
    }

    pub fn instruction_normal_entry(&self) -> Instruction {
        match self.initial_trade_side {
            Buy => Instruction::LimitBuy {
                symbol:      self.symbol(),
                order_id:    self.id.clone(),
                order_type:  self.entry_order_type,
                qty:         self.entry_qty.into(),
                price:       self.entry_price.into(),
                after_delay: self.next_normal_delay,
            },

            Sell => {
                /*
                Instruction::SellOCO {
                    symbol:     self.symbol(),
                    order_id: self.id,
                    order_type: self.entry_order_type,
                    qty: self.entry_qty.into(),
                    price: self.entry_price.into(),
                    stop_price: self.stop_price.into(),
                    stop_limit_price: None,
                }
                 */

                Instruction::LimitSell {
                    symbol:      self.symbol(),
                    order_id:    self.id.clone(),
                    order_type:  self.entry_order_type,
                    qty:         self.entry_qty.into(),
                    price:       self.entry_price.into(),
                    after_delay: self.next_normal_delay,
                }
            }
        }
    }

    pub fn instruction_contingency_entry(&self) -> Instruction {
        match self.initial_trade_side {
            Buy => Instruction::MarketBuy {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
                qty:      self.entry_qty.into(),
            },

            Sell => Instruction::MarketSell {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
                qty:      self.entry_qty.into(),
            },
        }
    }

    pub fn instruction_normal_exit(&self) -> Instruction {
        match self.initial_trade_side {
            Buy => match (self.stop_price, self.stop_limit_price) {
                (Some(stop_price), Some(stop_limit_price)) => Instruction::SellOCO {
                    symbol:           self.symbol(),
                    order_id:         self.id.clone(),
                    order_type:       self.exit_order_type,
                    qty:              self.untraded_qty_remainder(),
                    price:            self.exit_price,
                    stop_price:       Some(stop_price),
                    stop_limit_price: Some(stop_limit_price),
                    after_delay:      self.next_normal_delay,
                },
                _ => Instruction::LimitSell {
                    symbol:      self.symbol(),
                    order_id:    self.id.clone(),
                    order_type:  self.exit_order_type,
                    qty:         self.untraded_qty_remainder(),
                    price:       self.exit_price.into(),
                    after_delay: self.next_normal_delay,
                },
            },

            // FIXME: change to OCO buy because it's supposed to be used for shorting and must be extremely safe
            Sell => {
                Instruction::LimitBuy {
                    symbol:      self.symbol(),
                    order_id:    self.id.clone(),
                    order_type:  self.exit_order_type,
                    // WARNING: I'm totally unsure about the following line
                    qty:         self.untraded_qty_remainder(),
                    price:       self.entry_price.into(),
                    after_delay: self.next_normal_delay,
                }
            }
        }
    }

    pub fn instruction_cancel(&self) -> Instruction {
        Instruction::CancelOrder {
            symbol:   self.symbol(),
            order_id: self.id.clone(),
        }
    }

    pub fn instruction_cancel_replace_move(&self, new_price: OrderedValueType) -> Instruction {
        Instruction::CancelReplace {
            symbol: self.symbol(),
            cancel_order_id: self.id.clone(),
            new_side: self.current_trade_side,
            new_order_type: self.stop_order_type,
            qty: self.untraded_qty_remainder(),
            new_price,
            after_delay: self.next_normal_delay,
        }
    }

    pub fn instruction_cancel_replace_exit(&self, new_price: OrderedValueType) -> Instruction {
        if self.has_filled_entry_qty() && !self.cancel_over_replace {
            Instruction::CancelReplace {
                symbol:          self.symbol(),
                cancel_order_id: self.id.clone(),
                new_side:        self.initial_trade_side.opposite(),
                new_order_type:  self.stop_order_type,
                qty:             self.untraded_qty_remainder(),
                new_price:       self.exit_price,
                after_delay:     self.next_normal_delay,
            }
        } else {
            Instruction::CancelOrder {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
            }
        }
    }

    ///
    /// TODO: make this more custamizable
    pub fn instruction_contingency_exit(&self) -> Instruction {
        match self.initial_trade_side {
            Buy => Instruction::MarketSell {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
                qty:      self.untraded_qty_remainder(),
            },

            Sell => Instruction::MarketBuy {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
                qty:      self.untraded_qty_remainder(),
            },
        }
    }

    /// Returns the exiting instruction for emergency cases, when the
    /// roundtrip has to be closed with maximum possible certainty.
    ///
    /// This instruction has to be perceived as the last resort to exit the trade,
    /// for example, the order is in the entering phase and is partially filled,
    /// but then the order is cancelled (for whatever reason) by the signal received
    /// from the exchange. The amount possibly filled before it's cancelled needs to
    /// be dumped back, because the normal flow of this trade has been broken.
    ///
    /// NOTE: Same logic applies to the exiting phase.
    /// TODO: make this more custamizable
    pub fn instruction_emergency_exit(&self) -> Instruction {
        match self.initial_trade_side {
            Buy => Instruction::MarketSell {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
                qty:      self.untraded_qty_remainder(),
            },

            Sell => Instruction::MarketBuy {
                symbol:   self.symbol(),
                order_id: self.id.clone(),
                qty:      self.untraded_qty_remainder(),
            },
        }
    }

    fn enter_trade(&mut self, is_contingency_entry: bool, after_delay: Option<Duration>) -> Result<Directive, TradeError> {
        // obtaining instruction before switching states,
        // just to be safer (yes, paranoid, it's fine)
        let instruction = if is_contingency_entry {
            self.had_emergency_entry = true;
            self.instruction_contingency_entry()
        } else {
            self.instruction_normal_entry()
        };

        self.set_roundtrip_state(RoundtripState::EntryPending)?;
        self.set_order_state(OrderState::Pending)?;

        Ok(Directive::from(instruction))
    }

    /// Exit on the opposite side of Entry
    fn exit_trade(&mut self, is_contingency_exit: bool, after_delay: Option<Duration>) -> Result<Directive, TradeError> {
        // WARNING: EXIT IS A REVERSAL OF AN ENTRY SIDE

        // if the filled quantity is zero or is less than the minimum allowed by the rules,
        // then consider it Closed, although it's technically not, but let's keep it for now
        if self.has_filled_entry_qty() && self.appraiser.is_valid_qty(self.filled_entry_qty.0) {
            self.set_roundtrip_state(RoundtripState::ExitClosed)?;
            self.set_order_state(OrderState::Closed)?;
            return self.compute();
        }

        let instruction = if is_contingency_exit {
            // self.had_contingency_exit = true;
            // self.instruction_contingency_exit()
            if self.cancel_over_replace {
                self.instruction_cancel()
            } else {
                self.instruction_cancel_replace_move(self.stop_exit_price_for_move)
            }
        } else {
            self.instruction_normal_exit()
        };

        self.set_roundtrip_state(RoundtripState::ExitPending)?;
        self.set_order_state(OrderState::Pending)?;

        Ok(Directive::from(instruction))
    }

    fn emergency_exit_trade(&mut self) -> Result<Directive, TradeError> {
        // flagging that emergency exit has just started
        self.emergency_exit_in_progress = true;

        // WARNING: same as contingency exit except it ignores normal state transition,
        //  to avoid ANY possible error that would stop this exit from happening
        if self.has_untraded_qty_remainder() {
            // switching to the opposite because this section
            // is ignoring normal state transitioning rules and flow
            self.switch_side()?;

            self.roundtrip_state = RoundtripState::ExitPending;
            self.order_state = OrderState::Pending;
            self.had_emergency_exit = true;

            return Ok(Directive::from(self.instruction_contingency_exit()));
        }

        // overriding states
        self.roundtrip_state = RoundtripState::Cancelled;
        self.order_state = OrderState::Finished;

        self.compute()
    }

    /*
    /// Exit on the opposite side of Entry
    fn exit_trade_cancel_replace(&mut self, after_delay: Option<Duration>) -> Result<Directive, TradeError> {
        // if the filled quantity is zero or is less than the minimum allowed by the rules,
        // then consider it Closed, although it's technically not, but let's keep it for now
        if self.has_filled_entry_qty() && self.appraiser.is_valid_qty(self.filled_entry_qty.0) {
            self.set_roundtrip_state(RoundtripState::ExitClosed)?;
            self.set_order_state(OrderState::Closed)?;
            return self.compute();
        }

        let instruction = if is_contingency_exit {
            self.had_contingency_exit = true;
            self.instruction_contingency_exit()
        } else {
            self.instruction_normal_exit()
        };

        self.set_roundtrip_state(RoundtripState::ExitPending)?;
        self.set_order_state(OrderState::PendingReplace)?;

        Ok(Directive::from(instruction))
    }
     */

    fn cancel_and_exit(&mut self, after_delay: Option<Duration>) -> Result<Directive, TradeError> {
        // obtaining instruction before switching states,
        // just to be safer (yes, paranoid, it's fine)

        let instruction = if self.has_filled_entry_qty() {
            // FIXME: use replace to exit with unfilled qty
            self.set_order_state(OrderState::PendingReplace)?;

            // FIXME: the comment code below is ... strange and probably not working
            /*
            match self.phase() {
                TradingPhase::Pending | TradingPhase::Entering => {
                    warn!("{}: canceling roundtrip with filled entry qty", self.symbol);

                    // flagging that cancel is triggered internally and
                    // needs to be handled properly down the line
                    // FIXME: maybe this should be a config setting and not used like this
                    self.contingency_exit_on_cancel = true;

                    // setting a flag that cancel and exit is in progress,
                    // and Cancelled must be finished with Replaced
                    self.is_cancel_exit_in_progress = true;

                    self.set_roundtrip_state(RoundtripState::PendingCancel)?;
                    self.set_order_state(OrderState::PendingCancel)?;
                }
                TradingPhase::Exiting => {
                    // WARNING: instead of market exit, trying to move order closer to market price
                    self.set_order_state(OrderState::PendingReplace)?;
                }
            }
             */

            self.instruction_cancel_replace_exit(self.stop_exit_price_for_move)
        } else {
            // flagging that cancel is triggered internally and
            // needs to be handled properly down the line
            // FIXME: maybe this should be a config setting and not used like this
            self.execute_contingency_exit_on_cancel = true;

            // setting a flag that cancel and exit is in progress,
            // and Cancelled must be finished with Replaced
            self.is_cancel_exit_in_progress = true;

            self.set_roundtrip_state(RoundtripState::PendingCancel)?;
            self.set_order_state(OrderState::PendingCancel)?;
            self.instruction_cancel()
        };

        Ok(Directive::from(instruction))
    }

    fn cancel(&mut self) -> Result<Directive, TradeError> {
        self.set_roundtrip_state(RoundtripState::PendingCancel)?;
        self.set_order_state(OrderState::PendingCancel)?;
        Ok(Directive::from(self.instruction_cancel()))
    }

    fn move_to_new_price_on_same_side(&mut self, new_price: OrderedValueType, after_delay: Option<Duration>) -> Result<Directive, TradeError> {
        // obtaining instruction before switching states,
        // just to be safer (yes, paranoid, it's fine)
        let instruction = self.instruction_cancel_replace_move(new_price);

        // self.set_roundtrip_state(RoundtripState::PendingCancel)?;
        self.set_order_state(OrderState::PendingReplace);

        Ok(Directive::from(instruction))
    }

    fn handle_inactive(&mut self) -> Result<Directive, TradeError> {
        match self.phase() {
            TradingPhase::Pending | TradingPhase::Entering => {
                if self.user_termination_mark {
                    self.set_roundtrip_state(RoundtripState::Cancelled)?;
                    self.set_order_state(OrderState::Cancelled)?;
                    return self.compute();
                }

                self.enter_trade(false, self.next_normal_delay)
            }

            // WARNING: seeking for an expedient exit if marked for termination
            // TODO: decide whether the order should close normally if the entry order has been Opened
            // TODO: implement timeout mechanism for cases when exit might take too long
            // FIXME: perhaps contingency exit is an overkill and should let it exit normally
            TradingPhase::Exiting => self.exit_trade(self.user_termination_mark, self.next_normal_delay),
        }
    }

    fn handle_pending(&mut self) -> Result<Directive, TradeError> {
        Ok(Directive::empty())
    }

    fn handle_open_failed(&mut self) -> Result<Directive, TradeError> {
        match self.phase() {
            TradingPhase::Pending | TradingPhase::Entering => {
                if self.user_termination_mark {
                    // NOTE: making sure no quantity has been partially filled IOC for example
                    // return if self.has_filled_entry_qty() && self.appraiser.is_valid_qty(self.filled_entry_qty.0) {
                    return if self.has_filled_entry_qty() {
                        // NOTE: if something has been filled after all, then selling it off
                        self.exit_trade(true, None)
                    } else {
                        // nothing has been filled, setting it directly to Cancelled and Finished
                        self.set_roundtrip_state(RoundtripState::Cancelled)?;

                        // WARNING: circumventing state transition, forcing Finished state
                        // self.set_order_state(OrderState::Cancelled)?;
                        self.order_state = OrderState::Finished;

                        self.compute()
                    };
                }

                // checking whether any retries left or just finalizing
                if self.entry_retries_left.0 > 0 {
                    self.entry_retries_left.0 -= 1;
                    self.next_normal_delay = Some(Duration::from_millis(300));
                    self.set_roundtrip_state(RoundtripState::Pending)?;
                    self.adjust_prices();
                    self.enter_trade(false, self.next_normal_delay)
                } else {
                    self.set_roundtrip_state(RoundtripState::Failed)?;
                    self.finalize()
                }
            }

            TradingPhase::Exiting => {
                // FIXME: is this too lazy?
                if self.user_termination_mark {
                    self.set_roundtrip_state(RoundtripState::EntryClosed)?;
                    return self.exit_trade(true, None);
                }

                if self.exit_retries_left.0 > 0 {
                    self.exit_retries_left.0 -= 1;
                    self.next_normal_delay = Some(Duration::from_millis(300));
                    self.set_roundtrip_state(RoundtripState::EntryClosed)?;
                    self.adjust_prices();
                    self.exit_trade(false, self.next_normal_delay)
                } else {
                    self.set_roundtrip_state(RoundtripState::EntryClosed)?;
                    self.exit_trade(true, None)
                }
            }
        }
    }

    fn handle_opened_and_partially_filled(&mut self) -> Result<Directive, TradeError> {
        self.send_opened_ack();

        // ----------------------------------------------------------------------------
        // handling stoploss

        if self.latest_known_market_price.is_zero() {
            println!("current market price is zero");
            return Ok(Directive::empty());
        }

        if let Some(max_distance_from_market_price) = self.max_distance_from_market_price {
            /*
            // FIXME: review this later
            let price_to_measure_distance_from = match self.phase() {
                TradingPhase::Pending | TradingPhase::Entering => self.entry_price,
                TradingPhase::Exiting => self.exit_price,
            };
             */

            // let price_to_measure_distance_from = self.entry_price;
            let price_to_measure_distance_from = match self.phase() {
                TradingPhase::Pending | TradingPhase::Entering => self.entry_price,
                TradingPhase::Exiting => self.exit_price,
            };

            let max_lifetime_outside_of_distance = match self.phase() {
                TradingPhase::Pending | TradingPhase::Entering => self.max_entry_lifetime_outside_of_distance,
                TradingPhase::Exiting => self.max_exit_lifetime_outside_of_distance,
            };

            let max_distance_from_market_price = match self.phase() {
                TradingPhase::Pending | TradingPhase::Entering => max_distance_from_market_price * 0.5,
                // TradingPhase::Exiting => max_distance_from_market_price * 2.0,
                TradingPhase::Exiting => max_distance_from_market_price,
            };

            /*
            if OrderedFloat((self.current_market_price - price_to_measure_distance_from).abs()) > max_distance_from_market_price {
                self.set_order_state(OrderState::PendingReplace)?;

                return match self.current_side {
                    Buy => Ok(Directive::from(self.instruction_cancel_replace_exit())),
                    Sell => Ok(Directive::from(self.instruction_cancel_replace_move(self.stop_exit_price_for_move))),
                };
            }
             */

            if OrderedFloat((self.latest_known_market_price - price_to_measure_distance_from).abs()) > max_distance_from_market_price {
                match self.lifetime_outside_of_distance {
                    None => {
                        self.lifetime_outside_of_distance = Some(PausableClock::default());
                    }
                    Some(ref clock) =>
                        if clock.is_paused() {
                            clock.resume();
                        },
                };

                // WARNING if the market price has stayed beyond the permitted threshold from the
                //  measuring point, then this roundtrip is marked for termination
                if let Some(ref mut lifetime) = self.lifetime_outside_of_distance {
                    if Duration::from_millis(lifetime.now().elapsed_millis()) > max_lifetime_outside_of_distance {
                        // NOTE: removing clock
                        self.lifetime_outside_of_distance = None;

                        // WARNING: short-circuiting
                        if self.cancel_over_replace {
                            return self.cancel();
                        }

                        return match self.current_trade_side {
                            Buy => {
                                let cancel_or_replace_instruction = match self.instruction_cancel_replace_exit(self.stop_exit_price_for_move) {
                                    cancel_replace @ Instruction::CancelReplace { .. } => {
                                        self.set_order_state(OrderState::PendingReplace)?;
                                        cancel_replace
                                    }
                                    cancel_order @ Instruction::CancelOrder { .. } => {
                                        self.set_order_state(OrderState::PendingCancel)?;
                                        cancel_order
                                    }
                                    _ => panic!("returned instruction can only be CancelReplace or CancelOrder"),
                                };

                                Ok(Directive::from(cancel_or_replace_instruction))
                            }
                            Sell => {
                                self.set_order_state(OrderState::PendingReplace)?;
                                Ok(Directive::from(self.instruction_cancel_replace_move(self.stop_exit_price_for_move)))
                            }
                        };
                    }
                }
            } else if let Some(ref lifetime) = self.lifetime_outside_of_distance {
                // creating new clock and pausing it immediatelly, then replacing the main timer
                // let clock = PausableClock::default();
                // clock.pause();

                // FIXME: resetting clock when price returns inside the distance; review this after testing
                // self.lifetime_outside_of_distance = Some(clock);
                self.lifetime_outside_of_distance = None;
            }
        }

        // ----------------------------------------------------------------------------
        // normal operation

        if self.user_termination_mark {
            return self.cancel_and_exit(self.next_normal_delay);
        }

        let is_emergency_exit_in_progress = self.trigger_emergency_exit_on_next_compute && self.emergency_exit_in_progress;

        if self.is_unexpectedly_cancelled && !is_emergency_exit_in_progress {
            self.trigger_emergency_exit_on_next_compute = true;
            return self.compute();
        }

        Ok(Directive::empty())
    }

    /// if the cancel event has arrived from outside the system,
    /// then it's unhandled internally, otherwise can be set straight to Cancelled
    /// WARNING: the system may decide whether to dump quantity (if it has any)
    fn handle_cancelled(&mut self) -> Result<Directive, TradeError> {
        // NOTE: do nothing if CancelReplace is pending
        if self.order_state == OrderState::PendingReplace && !self.cancel_over_replace {
            return Ok(Directive::empty());
        }

        // if the roundtrip is cancelled, then system must handle
        // the contingency to dump any (possibly) partially or fully filled
        // quantity that was filled during entry phase
        // WARNING: this is a very sensitive area
        if self.has_filled_entry_qty() && self.execute_contingency_exit_on_cancel {
            // NOTE: resetting state to trigger contingency exit
            // self.set_roundtrip_state(RoundtripState::ExitPending)?;

            // if the `is_cancel_exit_in_progress` is true,
            // then setting state to Pending and expecting the
            // next state, if successful, to be `Replaced`, which
            // can be `Finished` afterwards
            if self.is_cancel_exit_in_progress {
                // WARNING: switching side because, obviously, exit is on the opposite side
                self.switch_side()?;
                self.set_order_state(OrderState::PendingReplace)?;
            } else {
                // otherwise, resetting state to `Inactive` to
                // trigger the contingency exit
                self.set_order_state(OrderState::Inactive)?;
            }
        } else {
            self.set_roundtrip_state(RoundtripState::Cancelled)?;
            self.set_order_state(OrderState::Finished)?;
        }

        self.compute()
    }

    fn handle_cancel_failed(&mut self) -> Result<Directive, TradeError> {
        // checking whether any retries left or just finalizing
        if self.entry_retries_left.0 > 0 {
            self.entry_retries_left.0 -= 1;
            self.next_normal_delay = Some(Duration::from_millis(300));
            self.set_roundtrip_state(RoundtripState::Pending)?;
            Ok(Directive::from(self.instruction_cancel_replace_exit(self.stop_exit_price_for_move)))
        } else {
            panic!("{}: {} cancel has failed", self.symbol, self.id);
        }
    }

    /*
    fn handle_rejected(&mut self) -> Result<Directive, TraderError> {
        use RoundtripState::*;

        if self.order_state != OrderState::Rejected {
            panic!("attempting to handle order without Rejected state");
        }

        Ok(Directive::empty())
    }

    fn handle_expired(&mut self) -> Result<Directive, TraderError> {
        use RoundtripState::*;

        if self.order_state != OrderState::Expired {
            panic!("attempting to handle order without Expired state");
        }

        // WARNING: processing as if it was unexpectedly cancelled
        Ok(Directive::empty())
    }
     */

    fn handle_filled(&mut self) -> Result<Directive, TradeError> {
        self.set_order_state(OrderState::Closed)?;
        self.compute()
    }

    fn handle_closed(&mut self) -> Result<Directive, TradeError> {
        // NOTE: resetting in case it was an entry
        self.lifetime_outside_of_distance = None;

        // WARNING: edge case for cancel and replace
        if self.is_cancel_exit_in_progress && self.user_termination_mark || self.is_unexpectedly_cancelled {
            self.set_roundtrip_state(RoundtripState::Cancelled)?;
            self.set_order_state(OrderState::Finished)?;
            return self.compute();
        }

        match self.phase() {
            TradingPhase::Pending => panic!("cannot to have an order CLOSED while Roundtrip is Inactive"),
            TradingPhase::Entering => {
                // switching to the opposite side
                self.switch_side()?;

                // entry phase is over
                self.set_roundtrip_state(RoundtripState::EntryClosed)?;

                // resetting order state for the exit phase
                self.set_order_state(OrderState::Inactive)?;
            }
            TradingPhase::Exiting => {
                // the full cycle is closed, the roundtrip is over
                self.set_roundtrip_state(RoundtripState::ExitClosed)?;

                if self.is_perpetual() && (!self.user_termination_mark && !self.had_emergency_exit) {
                    // restarting via reset
                    self.reset()?;
                } else {
                    // the full cycle is closed, the roundtrip is over
                    self.set_roundtrip_state(RoundtripState::Finished)?;

                    // finalizing order
                    self.set_order_state(OrderState::Finished)?;
                }
            }
        }

        self.compute()
    }

    fn handle_replace_failed(&mut self) -> Result<Directive, TradeError> {
        // WARNING: only BUY into SELL is supported at the moment

        self.set_roundtrip_state(RoundtripState::Cancelled);

        // WARNING: since cancel replace has failed, it is safe to finalize,
        //  because the other was cancelled but the new one didn't get created
        // self.set_order_state(OrderState::Cancelled);
        self.set_order_state(OrderState::Finished);

        Ok(Directive::empty())
    }

    fn handle_replaced(&mut self) -> Result<Directive, TradeError> {
        // TODO: add explanation and decide whether current phase matters

        // WARNING: having state `Replaced` means that previous order
        //  is cancelled and new is opened, it must be either Closed or Filled

        // WARNING: LOOK THROUGH AND THINK FIVE TIMES BEFORE CHANGING ANYTHING OR "OPTIMIZING" THIS BLOCK
        // NOTE: need to realign/reset entry and exit prices since it was moved

        // new entry price is made up, because original was created elsewhere
        let new_entry_price = self.stop_exit_price_for_move - (self.exit_price - self.entry_price);

        // exit price is where it was actually moved (because for now only BUY into SELL is supported)
        let new_exit_price = self.stop_exit_price_for_move;

        let (new_stop_price, new_stop_limit_price) = if let Some(old_stop_price) = self.stop_price {
            let new_stop_price = new_entry_price - (self.entry_price - old_stop_price);

            let new_stop_limit_price = self
                .stop_limit_price
                .map(|old_stop_limit_price| new_entry_price - (old_stop_price - old_stop_limit_price));

            (Some(new_stop_price), new_stop_limit_price)
        } else {
            (None, None)
        };

        // NOTE: sending back confirmation before overwriting old values
        self.send_replaced_ack(self.entry_price, self.exit_price, new_entry_price, new_exit_price);

        self.entry_price = new_entry_price;
        self.exit_price = new_exit_price;
        self.stop_price = new_stop_price;
        self.stop_limit_price = new_stop_limit_price;

        self.set_order_state(OrderState::Opened)?;
        self.compute()
    }

    fn handle_finished(&mut self) -> Result<Directive, TradeError> {
        use RoundtripState::*;

        if self.order_state != OrderState::Finished {
            panic!("attempting to handle order without Finished state");
        }

        // TODO: some post-finish hooks

        self.finalize()
    }

    pub fn status(&self) -> Status {
        match self.roundtrip_state {
            RoundtripState::Cancelled => Status::Cancelled,
            RoundtripState::Interrupted => Status::Interrupted,
            RoundtripState::Failed => Status::Failed,
            RoundtripState::Finished => Status::Success,
            _ => Status::Pending,
        }
    }

    pub fn send_open_failed_ack(&self) {
        if let Some(ref feedback_sender) = self.coin_worker_feedback_sender {
            feedback_sender
                .send(CoinWorkerEvent::ConfirmRoundtripOpenFailed { id: self.id.clone() })
                .expect(format!("{}: failed to send roundtrip open failed acknowledgement: {:?}", self.symbol(), self.status()).as_str());
        }
    }

    pub fn send_opened_ack(&self) {
        if let Some(ref feedback_sender) = self.coin_worker_feedback_sender {
            feedback_sender
                .send(CoinWorkerEvent::ConfirmRoundtripOpened {
                    id:          self.id.clone(),
                    tag:         self.support_resistance_reference_tag,
                    entry_price: self.entry_price,
                    exit_price:  self.entry_price,
                })
                .expect(format!("{}: failed to send roundtrip open acknowledgement: {:?}", self.symbol(), self.status()).as_str());
        }
    }

    pub fn send_replaced_ack(
        &self,
        old_entry_price: OrderedValueType,
        old_exit_price: OrderedValueType,
        new_entry_price: OrderedValueType,
        new_exit_price: OrderedValueType,
    ) {
        if let Some(ref feedback_sender) = self.coin_worker_feedback_sender {
            feedback_sender
                .send(CoinWorkerEvent::ConfirmRoundtripReplaced {
                    id: self.id.clone(),
                    old_entry_price,
                    old_exit_price,
                    new_entry_price,
                    new_exit_price,
                })
                .expect(format!("{}: failed to send roundtrip open acknowledgement: {:?}", self.symbol(), self.status()).as_str());
        }
    }

    pub fn send_finalized_feedback(&self) {
        if let Some(ref feedback_sender) = self.coin_worker_feedback_sender {
            feedback_sender
                .send(CoinWorkerEvent::RoundtripFinalized {
                    id:     self.id.clone(),
                    status: self.status(),
                })
                .expect(format!("{}: failed to send trade feedback with status: {:?}", self.symbol(), self.status()).as_str());
        }
    }

    fn finalize(&mut self) -> Result<Directive, TradeError> {
        if self.is_finalized {
            return Err(TradeError::RoundtripIsAlreadyFinalized(self.id.clone()));
        }

        // NOTE: sending feedback to the coin manager which is tracking active roundtrips
        self.send_finalized_feedback();

        // preventing from repeated finalization
        self.is_finalized = true;

        Ok(Directive::from(Instruction::FinalizeRoundtrip {
            symbol:                            self.symbol(),
            order_id:                          self.id.clone(),
            had_contingency_entry:             self.had_emergency_entry,
            had_contingency_exit:              self.had_emergency_exit,
            entry_retries_left:                self.entry_retries_left.0,
            exit_retries_left:                 self.exit_retries_left.0,
            is_cancelled_unexpectedly:         self.is_unexpectedly_cancelled,
            do_emergency_exit_on_next_compute: self.trigger_emergency_exit_on_next_compute,
            status:                            self.status(),
        }))
    }
}
 */

#[cfg(test)]
mod tests {
    use crate::{
        check_feedback,
        coin_worker::CoinEvent,
        order::{NormalizedOrderType, NormalizedSide},
        orderbook::Suggestion,
        price::{Appraiser, Currency, Rules},
        roundtrip::{OrderState, Roundtrip, RoundtripMode, RoundtripState, TradingPhase},
        roundtrip_assert, roundtrip_compute, roundtrip_compute_assert_entry_cancelled_and_pending_exit_replace,
        roundtrip_compute_assert_entry_cancelled_but_pending_replace, roundtrip_compute_assert_entry_closed, roundtrip_compute_assert_entry_opened,
        roundtrip_compute_assert_entry_partially_filled, roundtrip_compute_assert_exit_cancelled_but_pending_replace, roundtrip_compute_assert_exit_closed,
        roundtrip_compute_assert_exit_opened, roundtrip_compute_assert_exit_partially_filled, roundtrip_compute_assert_nothing_changed,
        roundtrip_compute_expecting_cancel_and_exit, roundtrip_compute_expecting_contingency_exit, roundtrip_compute_expecting_entry_cancel_order,
        roundtrip_compute_expecting_entry_cancel_replace_order, roundtrip_compute_expecting_entry_finalized_cancelled,
        roundtrip_compute_expecting_entry_finalized_cancelled_unexpectedly, roundtrip_compute_expecting_entry_finalized_with_failure,
        roundtrip_compute_expecting_exit_cancel_order, roundtrip_compute_expecting_exit_cancel_replace_order,
        roundtrip_compute_expecting_exit_finalized_cancelled, roundtrip_compute_expecting_exit_finalized_cancelled_unexpectedly,
        roundtrip_compute_expecting_exit_finalized_with_failure, roundtrip_compute_expecting_exit_successfully_finalized,
        roundtrip_compute_expecting_internally_cancelled_entry, roundtrip_compute_expecting_normal_entry, roundtrip_compute_expecting_normal_exit,
        roundtrip_compute_expecting_retry_entry, roundtrip_compute_expecting_retry_exit, roundtrip_init_test, roundtrip_notify_assert_entry_cancelled,
        roundtrip_notify_assert_entry_cancelled_unexpectedly, roundtrip_notify_assert_entry_filled, roundtrip_notify_assert_entry_opened,
        roundtrip_notify_assert_entry_partially_filled, roundtrip_notify_assert_exit_cancelled, roundtrip_notify_assert_exit_cancelled_unexpectedly,
        roundtrip_notify_assert_exit_filled, roundtrip_notify_assert_exit_opened, roundtrip_notify_assert_exit_partially_filled,
        roundtrip_notify_assert_exit_replaced, roundtrip_notify_assert_exit_replaced_filled, roundtrip_notify_entry_open_failed,
        roundtrip_notify_exit_open_failed,
        test_helpers::rules_for_testing,
        trader::Trader,
        trader_directive::{Directive, Instruction},
        util::Status,
    };
    use crossbeam::channel;
    use ordered_float::OrderedFloat;

    #[test]
    fn non_perpetual_with_partial_fills() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn non_perpetual_without_partial_fills() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn perpetual_with_partial_fills() {
        let (mut r, rx) = roundtrip_init_test!(Perpetual, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        for i in 0..=5 {
            // entry
            roundtrip_compute_expecting_normal_entry!(r);
            roundtrip_notify_assert_entry_opened!(r);
            roundtrip_compute_assert_entry_opened!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_entry_partially_filled!(r);
            roundtrip_compute_assert_entry_partially_filled!(r);
            roundtrip_notify_assert_entry_filled!(r);

            // exit
            roundtrip_compute_expecting_normal_exit!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_exit_opened!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_exit_partially_filled!(r);
            roundtrip_compute_assert_exit_partially_filled!(r);
            roundtrip_notify_assert_exit_filled!(r);

            if i == 5 {
                r.terminate();
            }
        }

        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn perpetual_without_partial_fills() {
        let (mut r, rx) = roundtrip_init_test!(Perpetual, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        for i in 0..=5 {
            // entry
            roundtrip_compute_expecting_normal_entry!(r);
            roundtrip_notify_assert_entry_opened!(r);
            roundtrip_compute_assert_entry_opened!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_entry_filled!(r);

            // exit
            roundtrip_compute_expecting_normal_exit!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_exit_opened!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_exit_filled!(r);

            if i == 5 {
                r.terminate();
            }
        }

        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn perpetual_failed_opening_with_all_entry_retries_exhausted() {
        let (mut r, rx) = roundtrip_init_test!(Perpetual, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        roundtrip_compute_expecting_normal_entry!(r);

        // failing all normal entry attempts
        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);

        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);

        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_entry_finalized_with_failure!(r);

        check_feedback!(r, rx, Failed);
    }

    #[test]
    fn perpetual_failed_opening_with_all_exit_retries_exhausted() {
        let (mut r, rx) = roundtrip_init_test!(Perpetual, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // failing all normal exit attempts
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_contingency_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_notify_assert_exit_filled!(r);

        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn perpetual_with_some_entries_and_exits_failed() {
        let (mut r, rx) = roundtrip_init_test!(Perpetual, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        for i in 0..=5 {
            // entry
            roundtrip_compute_expecting_normal_entry!(r);
            roundtrip_notify_assert_entry_opened!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_entry_partially_filled!(r);
            roundtrip_compute_assert_nothing_changed!(r);
            roundtrip_notify_assert_entry_filled!(r);

            // exit
            match i {
                1 => {
                    // introducing 1 hiccup before normal exit
                    roundtrip_compute_expecting_normal_exit!(r);
                    roundtrip_compute_assert_nothing_changed!(r);

                    roundtrip_notify_exit_open_failed!(r);
                    roundtrip_compute_expecting_retry_exit!(r);

                    roundtrip_notify_assert_exit_opened!(r);
                    roundtrip_compute_assert_nothing_changed!(r);
                    roundtrip_notify_assert_exit_partially_filled!(r);
                    roundtrip_compute_assert_exit_partially_filled!(r);
                    roundtrip_notify_assert_exit_filled!(r);
                }

                2 => {
                    // introducing 2 hiccups before normal exit
                    roundtrip_compute_expecting_normal_exit!(r);
                    roundtrip_compute_assert_nothing_changed!(r);

                    roundtrip_notify_exit_open_failed!(r);
                    roundtrip_compute_expecting_retry_exit!(r);

                    roundtrip_notify_exit_open_failed!(r);
                    roundtrip_compute_expecting_retry_exit!(r);

                    roundtrip_notify_assert_exit_opened!(r);
                    roundtrip_compute_assert_nothing_changed!(r);
                    roundtrip_notify_assert_exit_partially_filled!(r);
                    roundtrip_compute_assert_exit_partially_filled!(r);
                    roundtrip_notify_assert_exit_filled!(r);
                }

                5 => {
                    r.terminate();
                    roundtrip_compute_expecting_contingency_exit!(r);
                    roundtrip_notify_assert_exit_opened!(r);
                    roundtrip_notify_assert_exit_filled!(r);
                    roundtrip_compute_expecting_exit_successfully_finalized!(r);
                }

                _ => {
                    // normal exits
                    roundtrip_compute_expecting_normal_exit!(r);
                    roundtrip_compute_assert_nothing_changed!(r);
                    roundtrip_notify_assert_exit_opened!(r);
                    roundtrip_compute_assert_nothing_changed!(r);
                    roundtrip_notify_assert_exit_partially_filled!(r);
                    roundtrip_compute_assert_exit_partially_filled!(r);
                    roundtrip_notify_assert_exit_filled!(r);
                }
            }
        }

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn non_perpetual_failed_opening_with_all_entry_retries_exhausted() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        roundtrip_compute_expecting_normal_entry!(r);

        // failing all normal entry attempts
        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);

        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);

        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_entry_finalized_with_failure!(r);

        check_feedback!(r, rx, Failed);
    }

    #[test]
    fn non_perpetual_failed_opening_with_all_exit_retries_exhausted() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        roundtrip_compute_expecting_normal_entry!(r);

        // entry
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // failing all exit attempts, expecting a contingency exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_contingency_exit!(r);

        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn non_perpetual_roundtrip_with_few_failed_openings() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry, filing few entry attempts before successful open
        roundtrip_compute_expecting_normal_entry!(r);

        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);

        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);

        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // exit, same as entry, filing few exit attempts before successful open
        roundtrip_compute_expecting_normal_exit!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);

        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn terminate_entry_phase_failed_to_open() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);

        // stuffing terminate between failed openings
        roundtrip_notify_entry_open_failed!(r);
        roundtrip_compute_expecting_retry_entry!(r);
        roundtrip_notify_entry_open_failed!(r);

        r.terminate();

        roundtrip_compute_expecting_entry_finalized_cancelled!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn terminate_exit_phase_failed_to_open() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_exit_open_failed!(r);

        r.terminate();

        roundtrip_compute_expecting_contingency_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn terminate_entry_phase_opened() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);

        r.terminate();

        roundtrip_compute_expecting_entry_cancel_order!(r);
        roundtrip_notify_assert_entry_cancelled!(r);
        roundtrip_compute_expecting_entry_finalized_cancelled!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn terminate_exit_phase_opened() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);

        r.terminate();

        roundtrip_compute!(r, r.instruction_cancel_replace_move(r.entry_price), Exiting, PendingCancel, PendingCancel, r.entry_side.opposite());
        // roundtrip_compute_expecting_exit_cancel_replace_order!(r);
        roundtrip_notify_assert_exit_cancelled!(r);
        roundtrip_compute_assert_exit_cancelled_but_pending_replace!(r);
        roundtrip_notify_assert_exit_replaced!(r);
        roundtrip_compute!(r, Exiting, PendingCancel, Replaced, r.entry_side.opposite());
        roundtrip_notify_assert_exit_replaced_filled!(r);
        roundtrip_compute_expecting_exit_finalized_cancelled!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn terminate_entry_phase_partially_filled() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        r.terminate();

        // forced exit
        roundtrip_compute_expecting_entry_cancel_replace_order!(r);
        roundtrip_notify_assert_entry_cancelled!(r);
        roundtrip_compute_assert_entry_cancelled_and_pending_exit_replace!(r);
        roundtrip_notify_assert_exit_replaced!(r);
        roundtrip_compute!(r, Exiting, PendingCancel, Replaced, r.entry_side.opposite());
        roundtrip_notify_assert_exit_replaced_filled!(r);
        roundtrip_compute_expecting_exit_finalized_cancelled!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn terminate_exit_phase_partially_filled() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // normal exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);

        // filling partially before termination signal
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);

        r.terminate();

        // contingency exit
        roundtrip_compute_expecting_exit_cancel_replace_order!(r);
        roundtrip_notify_assert_exit_cancelled!(r);
        roundtrip_compute_assert_exit_cancelled_but_pending_replace!(r);
        roundtrip_notify_assert_exit_replaced!(r);
        roundtrip_compute!(r, Exiting, PendingCancel, Replaced, r.entry_side.opposite());
        roundtrip_notify_assert_exit_replaced_filled!(r);
        roundtrip_compute_expecting_exit_finalized_cancelled!(r);

        check_feedback!(r, rx, Cancelled);
    }

    // NOTE TO FUTURE SELF: don't think again about testing `terminate_exit_phase_filled`,
    //  if it's Filled it's Closed, nothing left to terminate.
    // WARNING: this test is redundant, because if entry phase is Filled,
    //  then why bother about trying to catch a termination while it's
    //  already going towards the exit, unless..... HMMMMMM, MAYBE it's useful
    //  in case if in the future, postponed exits will be implemented, so that
    //  Filled/Closed entry but pending for the exit to begin, can terminate
    // TODO: do something about the mess I wrote above
    #[test]
    fn terminate_entry_phase_filled() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        r.terminate();
        roundtrip_compute_expecting_contingency_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    // NOTE: this test must make sure that Filled exit trade won't be
    //  interrupted and will be finalized normally
    #[test]
    fn terminate_exit_phase_filled() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // normal exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);

        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);

        // WARNING: this termination must not affect anything and let roundtrip finish its normal course
        r.terminate();

        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    // NOTE: same as above, this test must make sure that Closed exit trade
    //  won't be interrupted and will be finalized normally (paranoia, yes)
    #[test]
    fn terminate_exit_phase_closed() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // normal exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_exit_open_failed!(r);
        roundtrip_compute_expecting_retry_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);

        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);

        // WARNING: this termination must not affect anything and let roundtrip finish its normal course
        r.terminate();

        roundtrip_compute_expecting_exit_successfully_finalized!(r);

        check_feedback!(r, rx, Success);
    }

    #[test]
    fn cancelled_entry_phase_opened() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry and abrupt cancel
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);

        // unexpected cancel from outside
        r.notify_cancelled();

        roundtrip_compute_expecting_entry_finalized_cancelled_unexpectedly!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn cancelled_exit_phase_opened() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // normal entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // normal exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        // open and abrupt cancel
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);

        // unexpected cancel from outside
        r.notify_cancelled();

        roundtrip_compute_expecting_contingency_exit!(r);
        roundtrip_notify_assert_exit_opened!(r);

        // filling some then fully
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);

        // NOTE: well... this success is questionable, but at least it's finalized
        roundtrip_compute_expecting_exit_finalized_cancelled_unexpectedly!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn cancelled_entry_phase_partially_filled() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // partial fill and abrupt cancel
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);

        // unexpected cancel from outside
        r.notify_cancelled();

        roundtrip_compute!(r, r.instruction_emergency_exit(), Exiting, ExitPending, Pending, r.entry_side.opposite());
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_finalized_cancelled_unexpectedly!(r);

        check_feedback!(r, rx, Cancelled);
    }

    #[test]
    fn cancelled_exit_phase_partially_filled() {
        let (mut r, rx) = roundtrip_init_test!(Once, OrderedFloat(500.0), Buy, 86.71, 86.92, 86.32);

        // entry
        roundtrip_compute_expecting_normal_entry!(r);
        roundtrip_notify_assert_entry_opened!(r);
        roundtrip_compute_assert_entry_opened!(r);
        roundtrip_notify_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_entry_partially_filled!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_entry_filled!(r);

        // exit
        roundtrip_compute_expecting_normal_exit!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);

        // unexpected cancel from outside
        r.notify_cancelled();

        roundtrip_compute!(r, r.instruction_emergency_exit(), Exiting, ExitPending, Pending, r.entry_side.opposite());
        roundtrip_notify_assert_exit_opened!(r);
        roundtrip_compute_assert_exit_opened!(r);
        roundtrip_compute_assert_nothing_changed!(r);
        roundtrip_notify_assert_exit_partially_filled!(r);
        roundtrip_compute_assert_exit_partially_filled!(r);
        roundtrip_notify_assert_exit_filled!(r);
        roundtrip_compute_expecting_exit_finalized_cancelled_unexpectedly!(r);

        check_feedback!(r, rx, Cancelled);
    }
}
