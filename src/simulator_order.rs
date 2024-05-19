use crate::{
    f,
    id::OrderId,
    order::{OrderFill, OrderFillMetadata, OrderFillStatus, OrderState},
    simulator::{SimulatorError, SimulatorOrderEvent},
    types::{
        NormalizedExecutionType, NormalizedOrderEvent, NormalizedOrderEventSource, NormalizedOrderStatus, NormalizedOrderType, NormalizedSide,
        NormalizedTimeInForce, OrderedValueType,
    },
    util::new_id_u64,
};
use chrono::Utc;
use log::info;
use ordered_float::OrderedFloat;
use portable_atomic::AtomicF64;
use std::sync::{atomic::Ordering, Arc};
use ustr::Ustr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SimulatorOrderMode {
    #[default]
    /// This is the default mode for all orders
    Regular,

    /// This is the mode for orders that have been triggered by their stop price
    StopTriggered,
}

#[derive(Debug, Clone)]
pub struct SimulatorOrder {
    /// This is the ID that the exchange (simulator) will return back to the client
    pub id: OrderId,

    /// This is the ID internal to the simulator (used for cancelling related orders in case of OCO)
    pub other_order_id: Option<OrderId>,

    /// This is an auxiliary flag that indicates whether the other order has been cancelled
    /// NOTE: The purpose is avoid complex logic in the simulator, deciding whether to cancel the other order or not
    pub is_cancelled_by_other_order: bool,

    /// This is the mode of the order (regular or stop-triggered)
    pub mode: SimulatorOrderMode,

    /// This is the ID that the client will send to us
    /// IMPORTANT: This serves as both regular client order ID and a stop order ID, depending on the case
    pub client_order_id: OrderId,

    /// The client-provided ID for the stop order part of a complex order like OCO.
    /// TODO: Do I need this?
    pub client_stop_order_id: Option<OrderId>,

    /// The client-provided ID for the limit order part of a complex order like OCO.
    pub client_limit_order_id: Option<OrderId>,

    /// The client-provided ID for tracking the entire OCO order or a similar complex order.
    pub client_list_order_id: Option<OrderId>,

    /// The client-provided ID for tracking the cancel order.
    pub client_cancel_order_id: Option<OrderId>,

    /// The original client order ID, typically used for referencing the original order in modifications or cancellations.
    pub client_original_order_id: Option<OrderId>,

    /// The symbol of the order
    pub symbol: Ustr,

    /// The side of the order
    pub side: NormalizedSide,

    /// The type of the order
    pub order_type: NormalizedOrderType,

    /// The time in force of the order
    pub time_in_force: NormalizedTimeInForce,

    /// The price of the order (for market orders, this is the market price)
    /// NOTE: Depending on the order type this can be either the stop price or the limit price
    pub price: OrderedValueType,

    /// The limit price of the order
    /// NOTE: This is only used for stoploss limit orders
    pub limit_price: Option<OrderedValueType>,

    /// The initial quantity of the order
    pub initial_quantity: OrderedValueType,

    /// The filled quantity of the order
    pub filled_quantity: OrderedValueType,

    /// The fills of the order (if any)
    pub fills: Vec<OrderFillMetadata>,

    /// The state of the order
    pub state: OrderState,

    /// The shared atomic market price
    pub atomic_market_price: Arc<AtomicF64>,

    /// The order that will replace this one if it gets cancelled
    pub replace_with_on_cancel: Option<Box<SimulatorOrder>>,

    /// Accumulated events
    pub events: Vec<SimulatorOrderEvent>,
}

impl AsRef<SimulatorOrder> for SimulatorOrder {
    fn as_ref(&self) -> &SimulatorOrder {
        self
    }
}

impl SimulatorOrder {
    pub fn market_price(&self) -> OrderedValueType {
        f!(self.atomic_market_price.load(Ordering::SeqCst))
    }

    pub fn validate(&self) -> Result<(), SimulatorError> {
        let current_market_price = self.market_price();

        if matches!(self.order_type, NormalizedOrderType::Limit) && self.price <= f!(0.0) {
            return Err(SimulatorError::InvalidPrice(self.price));
        }

        if self.initial_quantity <= f!(0.0) {
            return Err(SimulatorError::InvalidQuantity(self.initial_quantity));
        }

        if matches!(self.order_type, NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit) {
            if self.price <= f!(0.0) {
                return Err(SimulatorError::InvalidStopPrice(self.price));
            }

            if self.order_type == NormalizedOrderType::StopLossLimit {
                if self.limit_price.is_none() || self.limit_price.unwrap() <= f!(0.0) {
                    return Err(SimulatorError::MissingStopLimitPrice(self.client_order_id));
                }
            }
        }

        if !self.is_oco() && !self.is_stoploss() {
            // Regular orders should not have stop or limit order IDs
            if self.client_stop_order_id.is_some() || self.client_limit_order_id.is_some() {
                return Err(SimulatorError::InvalidRegularOrderWithStopLimitId(self.client_order_id));
            }
        }

        if self.filled_quantity > self.initial_quantity {
            return Err(SimulatorError::InvalidFilledQuantity(self.filled_quantity));
        }

        if !self.is_price_valid(self.symbol, self.price) {
            return Err(SimulatorError::InvalidPricePrecision(self.price));
        }

        if !self.is_time_in_force_valid(self.order_type, self.time_in_force) {
            return Err(SimulatorError::InvalidTimeInForce(self.time_in_force));
        }

        if self.order_type == NormalizedOrderType::Market && self.price != current_market_price {
            return Err(SimulatorError::InvalidMarketPrice(self.price));
        }

        if self.filled_quantity < f!(0.0) || self.filled_quantity > self.initial_quantity {
            return Err(SimulatorError::InvalidFilledQuantity(self.filled_quantity));
        }

        Ok(())
    }

    // Check for price precision and limits based on the trading pair
    fn is_price_valid(&self, symbol: Ustr, price: OrderedValueType) -> bool {
        // TODO: Implement this
        true
    }

    fn is_time_in_force_valid(&self, order_type: NormalizedOrderType, time_in_force: NormalizedTimeInForce) -> bool {
        use NormalizedOrderType::*;
        use NormalizedTimeInForce::*;

        match (order_type, time_in_force) {
            (Market, GTC) => true,
            (Market, IOC) => true,
            (Market, FOK) => true,
            (Limit, GTC) => true,
            (Limit, IOC) => true,
            (Limit, FOK) => true,
            (LimitMaker, GTC) => true,
            (StopLoss, GTC) => true,
            (StopLossLimit, GTC) => true,
            _ => false,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, OrderState::Opened | OrderState::PartiallyFilled)
    }

    pub fn is_filled(&self) -> bool {
        self.state == OrderState::Filled
    }

    pub fn is_rejected(&self) -> bool {
        matches!(self.state, OrderState::OpenFailed | OrderState::Rejected)
    }

    pub fn is_cancelled(&self) -> bool {
        self.state == OrderState::Cancelled
    }

    pub fn is_inactive(&self) -> bool {
        self.state == OrderState::Idle
    }

    pub fn is_market(&self) -> bool {
        self.order_type == NormalizedOrderType::Market
    }

    pub fn is_limit(&self) -> bool {
        self.order_type == NormalizedOrderType::Limit
    }

    pub fn is_limit_maker(&self) -> bool {
        self.order_type == NormalizedOrderType::LimitMaker
    }

    pub fn is_stoploss(&self) -> bool {
        matches!(self.order_type, NormalizedOrderType::StopLoss | NormalizedOrderType::StopLossLimit)
    }

    pub fn is_oco(&self) -> bool {
        self.other_order_id.is_some()
    }

    pub fn is_buy(&self) -> bool {
        self.side == NormalizedSide::Buy
    }

    pub fn is_sell(&self) -> bool {
        self.side == NormalizedSide::Sell
    }

    /// NOTE: This is Status not State
    pub fn status(&self) -> NormalizedOrderStatus {
        if self.is_filled() {
            NormalizedOrderStatus::Filled
        } else if self.is_rejected() {
            NormalizedOrderStatus::Rejected
        } else if self.is_cancelled() {
            NormalizedOrderStatus::Cancelled
        } else {
            if self.fills.is_empty() {
                NormalizedOrderStatus::New
            } else {
                NormalizedOrderStatus::PartiallyFilled
            }
        }
    }

    // TODO: Lifetime-related functionality should probably be delegated to the TimeInForce of the exchange
    pub(crate) fn generate_event(&self) -> NormalizedOrderEvent {
        // Calculating the average filled price
        let total_weighted_price = self.fills.iter().fold(f!(0.0), |acc, fill| acc + fill.quantity * fill.price);
        let current_total_quantity = self.fills.iter().fold(f!(0.0), |acc, fill| acc + fill.quantity);
        let avg_filled_price = if current_total_quantity > f!(0.0) { total_weighted_price / current_total_quantity } else { f!(0.0) };

        let execution_type = match (self.status(), self.state) {
            (NormalizedOrderStatus::New, OrderState::OpenFailed) => NormalizedExecutionType::New,
            (NormalizedOrderStatus::New, OrderState::Opened) => NormalizedExecutionType::New,
            (NormalizedOrderStatus::PartiallyFilled, OrderState::PartiallyFilled) => NormalizedExecutionType::Trade,
            (NormalizedOrderStatus::Filled, OrderState::Filled) => NormalizedExecutionType::Trade,
            (NormalizedOrderStatus::Cancelled, OrderState::Cancelled) => NormalizedExecutionType::Cancelled,
            (NormalizedOrderStatus::Rejected, OrderState::Rejected) => NormalizedExecutionType::Rejected,
            (NormalizedOrderStatus::New, OrderState::Expired) => NormalizedExecutionType::Expired,

            (status, state) => {
                panic!("Unhandled status/state combination: {:?}/{:?}", status, state);
            }
        };

        // Obtaining the latest fill (the one that just happened)
        match self.fills.last() {
            None => NormalizedOrderEvent {
                symbol:                                     self.symbol,
                exchange_order_id:                          self.id,
                local_order_id:                             self.client_order_id,
                trade_id:                                   new_id_u64() as i64,
                time_in_force:                              self.time_in_force,
                side:                                       self.side,
                order_type:                                 self.order_type,
                current_execution_type:                     execution_type,
                current_order_status:                       self.status(),
                order_price:                                self.price,
                quantity:                                   self.initial_quantity,
                avg_filled_price:                           f!(0.0),
                last_filled_price:                          f!(0.0),
                last_executed_quantity:                     f!(0.0),
                last_quote_asset_transacted_quantity:       f!(0.0),
                cumulative_filled_quantity:                 f!(0.0),
                cumulative_quote_asset_transacted_quantity: f!(0.0),
                quote_order_quantity:                       f!(0.0),
                commission:                                 f!(0.0),
                commission_asset:                           Some(self.symbol),
                latest_fill:                                None,
                is_trade_the_maker_side:                    false,
                order_reject_reason:                        "".to_string(),
                source:                                     NormalizedOrderEventSource::UserTradeEvent,
                transaction_time:                           Utc::now(),
                event_time:                                 Utc::now(),
            },

            Some(latest_fill_metadata) => {
                let latest_fill = if latest_fill_metadata.accumulated_filled_quantity >= self.initial_quantity {
                    OrderFill::new(
                        OrderFillStatus::Filled,
                        latest_fill_metadata.price,
                        latest_fill_metadata.quantity,
                        latest_fill_metadata.accumulated_filled_quantity,
                        latest_fill_metadata.accumulated_filled_quote,
                        Utc::now(),
                    )
                } else {
                    OrderFill::new(
                        OrderFillStatus::PartiallyFilled,
                        latest_fill_metadata.price,
                        latest_fill_metadata.quantity,
                        latest_fill_metadata.accumulated_filled_quantity,
                        latest_fill_metadata.accumulated_filled_quote,
                        Utc::now(),
                    )
                };

                NormalizedOrderEvent {
                    symbol:                                     self.symbol,
                    exchange_order_id:                          self.id,
                    local_order_id:                             self.client_order_id,
                    trade_id:                                   new_id_u64() as i64,
                    time_in_force:                              NormalizedTimeInForce::GTC,
                    side:                                       self.side,
                    order_type:                                 self.order_type,
                    current_execution_type:                     NormalizedExecutionType::Trade,
                    current_order_status:                       self.status(),
                    order_price:                                latest_fill.metadata.price,
                    quantity:                                   latest_fill.metadata.quantity,
                    avg_filled_price:                           avg_filled_price,
                    last_filled_price:                          latest_fill.metadata.price,
                    last_executed_quantity:                     latest_fill.metadata.quantity,
                    last_quote_asset_transacted_quantity:       latest_fill.metadata.quote_equivalent,
                    cumulative_filled_quantity:                 latest_fill.metadata.accumulated_filled_quantity,
                    cumulative_quote_asset_transacted_quantity: latest_fill.metadata.accumulated_filled_quote,
                    quote_order_quantity:                       f!(0.0),
                    commission:                                 f!(0.0),
                    commission_asset:                           Some(self.symbol),
                    latest_fill:                                Some(latest_fill),
                    is_trade_the_maker_side:                    false,
                    order_reject_reason:                        "".to_string(),
                    source:                                     NormalizedOrderEventSource::UserTradeEvent,
                    transaction_time:                           Utc::now(),
                    event_time:                                 Utc::now(),
                }
            }
        }
    }

    pub fn compute_market(&mut self, current_market_price: OrderedValueType) -> Result<(), SimulatorError> {
        self.fills.push(OrderFillMetadata {
            price:                       current_market_price,
            quantity:                    self.initial_quantity,
            quote_equivalent:            self.initial_quantity * current_market_price,
            accumulated_filled_quantity: self.initial_quantity,
            accumulated_filled_quote:    self.initial_quantity * current_market_price,
            timestamp:                   Utc::now(),
        });

        self.filled_quantity = self.initial_quantity;
        self.state = OrderState::Filled;
        self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));

        Ok(())
    }

    pub fn compute_limit(&mut self, current_market_price: OrderedValueType) -> Result<(), SimulatorError> {
        let is_matched = match self.side {
            NormalizedSide::Buy => {
                if current_market_price > self.price {
                    return Ok(());
                }

                true
            }

            NormalizedSide::Sell => {
                if current_market_price < self.price {
                    return Ok(());
                }

                true
            }
        };

        if is_matched {
            self.fills.push(OrderFillMetadata {
                price:                       self.price,
                quantity:                    self.initial_quantity,
                quote_equivalent:            self.initial_quantity * self.price,
                accumulated_filled_quantity: self.initial_quantity,
                accumulated_filled_quote:    self.initial_quantity * self.price,
                timestamp:                   Utc::now(),
            });

            self.filled_quantity = self.initial_quantity;
            self.state = OrderState::Filled;
            self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));
        }

        Ok(())
    }

    pub fn compute_limit_maker(&mut self, current_market_price: OrderedValueType) -> Result<(), SimulatorError> {
        let is_matched = match self.side {
            NormalizedSide::Buy => {
                if current_market_price > self.price {
                    return Ok(());
                }

                true
            }

            NormalizedSide::Sell => {
                if current_market_price < self.price {
                    return Ok(());
                }

                true
            }
        };

        if is_matched {
            self.fills.push(OrderFillMetadata {
                price:                       self.price,
                quantity:                    self.initial_quantity,
                quote_equivalent:            self.initial_quantity * self.price,
                accumulated_filled_quantity: self.initial_quantity,
                accumulated_filled_quote:    self.initial_quantity * self.price,
                timestamp:                   Utc::now(),
            });

            self.filled_quantity = self.initial_quantity;
            self.state = OrderState::Filled;
            self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));
        }

        Ok(())
    }

    pub fn compute_stoploss(&mut self, current_market_price: OrderedValueType) -> Result<(), SimulatorError> {
        info!("Computing stoploss order: {:?}", self);

        self.fills.push(OrderFillMetadata {
            price:                       current_market_price,
            quantity:                    self.initial_quantity,
            quote_equivalent:            self.initial_quantity * current_market_price,
            accumulated_filled_quantity: self.initial_quantity,
            accumulated_filled_quote:    self.initial_quantity * current_market_price,
            timestamp:                   Utc::now(),
        });

        self.filled_quantity = self.initial_quantity;
        self.state = OrderState::Filled;
        self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));

        Ok(())
    }

    pub fn compute_stoploss_limit(&mut self, previous_market_price: OrderedValueType, current_market_price: OrderedValueType) -> Result<(), SimulatorError> {
        let Some(limit_price) = self.limit_price else {
            return Err(SimulatorError::MissingStopLimitPrice(self.client_order_id));
        };

        let is_matched = match self.side {
            NormalizedSide::Buy => {
                // For shorting, the stop is triggered when the price rises to or above the stop price
                let stop_triggered = self.mode == SimulatorOrderMode::Regular && previous_market_price < self.price && current_market_price >= self.price;

                if stop_triggered {
                    info!("BUY Stop triggered for order {} at price {}, limit price = {}:", self.id, self.price.0, limit_price.0);
                    self.mode = SimulatorOrderMode::StopTriggered;
                }

                self.mode == SimulatorOrderMode::StopTriggered && current_market_price >= limit_price
            }

            NormalizedSide::Sell => {
                let stop_triggered = self.mode == SimulatorOrderMode::Regular && previous_market_price > self.price && current_market_price <= self.price;

                if stop_triggered {
                    info!("SELL Stop triggered for order {} at price {}:", self.id, self.price.0);
                    self.mode = SimulatorOrderMode::StopTriggered;
                }

                // For a sell stoploss limit order, the order is matched if the current price is equal to or less than the limit price
                // after the stop price has been triggered.
                self.mode == SimulatorOrderMode::StopTriggered && current_market_price <= limit_price
            }
        };

        if is_matched {
            info!("Stoploss limit order matched: id={} price={} limit_price={}", self.id, self.price, limit_price.0);

            self.fills.push(OrderFillMetadata {
                price:                       self.price,
                quantity:                    self.initial_quantity,
                quote_equivalent:            self.initial_quantity * limit_price,
                accumulated_filled_quantity: self.initial_quantity,
                accumulated_filled_quote:    self.initial_quantity * limit_price,
                timestamp:                   Utc::now(),
            });

            self.filled_quantity = self.initial_quantity;
            self.state = OrderState::Filled;
            self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));
        }

        Ok(())
    }

    pub fn compute_oco(&mut self, price: OrderedValueType) -> Result<(), SimulatorError> {
        todo!("Implement OCO");
    }

    pub fn compute(&mut self, previous_market_price: OrderedValueType, current_market_price: OrderedValueType) -> Result<(), SimulatorError> {
        if !self.is_open() {
            return Ok(());
        }

        match self.order_type {
            NormalizedOrderType::Limit => self.compute_limit(current_market_price),
            NormalizedOrderType::LimitMaker => self.compute_limit_maker(current_market_price),
            NormalizedOrderType::Market => self.compute_market(current_market_price),
            NormalizedOrderType::StopLoss => self.compute_stoploss(current_market_price),
            NormalizedOrderType::StopLossLimit => self.compute_stoploss_limit(previous_market_price, current_market_price),
            _ => {
                return Err(SimulatorError::Error(format!("Unhandled order type: {:?}", self.order_type)));
            }
        }
    }

    pub fn close(&mut self) -> Result<(), SimulatorError> {
        if !self.is_open() {
            return Ok(());
        }

        self.state = OrderState::Filled;
        self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));

        Ok(())
    }

    pub fn cancel(&mut self, is_cancelled_by_other_order: bool) -> Result<(), SimulatorError> {
        if !self.is_open() {
            return Ok(());
        }

        self.is_cancelled_by_other_order = is_cancelled_by_other_order;
        self.state = OrderState::Cancelled;
        self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));

        Ok(())
    }

    pub fn expire(&mut self) -> Result<(), SimulatorError> {
        if !self.is_open() {
            return Ok(());
        }

        self.state = OrderState::Expired;
        self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));

        Ok(())
    }

    pub fn reject(&mut self) -> Result<(), SimulatorError> {
        if !self.is_open() {
            return Ok(());
        }

        self.state = OrderState::Rejected;
        self.events.push(SimulatorOrderEvent::OrderEvent(self.generate_event()));

        Ok(())
    }
}
