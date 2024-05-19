use crossbeam_channel::Sender;
use std::{
    sync::{atomic::Ordering, Arc},
    time::Instant,
};

use indexmap::{
    map::{Entry, MutableKeys},
    IndexMap,
};
use intmap::IntMap;
use itertools::Itertools;
use log::info;
use ordered_float::OrderedFloat;
use parking_lot::{MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use portable_atomic::AtomicF64;
use thiserror::Error;
use ustr::{ustr, Ustr};
use yata::core::{Method, ValueType};

use crate::{
    candle::Candle,
    f,
    id::{OrderId, SmartId},
    order::OrderState,
    simulator_generator::{ComputedGeneratorState, Generator, GeneratorConfig},
    simulator_order::{SimulatorOrder, SimulatorOrderMode},
    timeframe::{Timeframe, TimeframeError},
    timeset::TimeSet,
    trader::{TraderError, TraderSymbolContext},
    types::{NormalizedBalanceEvent, NormalizedOrderEvent, NormalizedOrderType, NormalizedSide, NormalizedTimeInForce, OrderedValueType},
    util::new_id_u64,
};

#[derive(Error, Debug)]
pub enum SimulatorError {
    #[error("Lookback length exceeds available data")]
    LookbackLengthExceedsAvailableData,

    #[error(transparent)]
    TimeframeError(#[from] TimeframeError),

    #[error("Candle not found for timeframe {0:?} at index {1}")]
    CandleNotFound(Timeframe, usize),

    #[error("Order not found: {0:?}")]
    OrderNotFound(OrderId),

    #[error("Unsupported timeframe: {0:?}")]
    UnsupportedTimeframe(Timeframe),

    #[error("Duplicate order: {0:?}")]
    DuplicateOrder(OrderId),

    #[error("Failed to place order: {0:?}")]
    FailedToPlaceOrder(OrderId),

    #[error("Failed to fill order: {0:?}")]
    FailedToFillOrder(OrderId),

    #[error("Failed to close order: {0:?}")]
    FailedToCloseOrder(OrderId),

    #[error("Failed to cancel order: {0:?}")]
    FailedToCancelOrder(OrderId),

    #[error("Invalid price: {0}")]
    InvalidPrice(OrderedValueType),

    #[error("Invalid quantity: {0}")]
    InvalidQuantity(OrderedValueType),

    #[error("Invalid order type: {0:?}")]
    InvalidOrderType(NormalizedOrderType),

    #[error("Invalid stop order type: {0:?}")]
    InvalidStopOrderType(NormalizedOrderType),

    #[error("Invalid stop order: {0}")]
    InvalidStopOrder(OrderId),

    #[error("Regular order must not have a parent id: {0}")]
    RegularOrderWithParentId(OrderId),

    #[error("Stop limit order MUST have a parent id: {0}")]
    MissingParentId(OrderId),

    #[error("Stop order must NOT have a parent id: {0}")]
    StopOrderWithParentId(OrderId),

    #[error("Stop order MUST have a stop price: {0}")]
    MissingStopPrice(OrderId),

    #[error("Stop limit order MUST have a stop limit price: {0}")]
    MissingStopLimitPrice(OrderId),

    #[error("Invalid stop price: {0}")]
    InvalidStopPrice(OrderedValueType),

    #[error("Invalid regular order with stop limit id: {0}")]
    InvalidRegularOrderWithStopLimitId(OrderId),

    #[error("Invalid filled quantity: {0}")]
    InvalidFilledQuantity(OrderedValueType),

    #[error("Invalid price precision: {0}")]
    InvalidPricePrecision(OrderedValueType),

    #[error("Invalid time in force: {0:?}")]
    InvalidTimeInForce(NormalizedTimeInForce),

    #[error("Invalid market price: {0}")]
    InvalidMarketPrice(OrderedValueType),

    #[error("Order is not open: {0:?}")]
    OrderIsNotOpen(OrderId),

    #[error("Duplicate client order id: {0:?}")]
    DuplicateClientOrderId(OrderId),

    #[error("Error({code}): {msg} ({local_description})")]
    CodedError {
        code:              i32,
        msg:               String,
        local_description: String,
    },

    #[error("Error: {0:?}")]
    Error(String),
}

#[derive(Debug, Default, Clone)]
pub struct SimulationStats {
    pub ticks:                  usize,
    pub total_quote_turnover:   ValueType,
    pub total_base_turnover:    ValueType,
    pub total_orders_opened:    u32,
    pub total_orders_replaced:  u32,
    pub total_orders_cancelled: u32,
    pub total_orders_closed:    u32,
    pub total_orders_rejected:  u32,
    pub total_orders_expired:   u32,
}

// ----------------------------------------------------------------------------
// IMPORTANT:
//
// If the limit price is not provided, then assuming it's a market order,
// BUT the order must be VALIDATED by checking the order type and price values
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SimulatorRequestMessage {
    GenerateCandles {
        timeframe:       Timeframe,
        count:           usize,
        response_sender: Sender<SimulatorRequestResponse>,
    },

    CreateOrder {
        symbol:                Ustr,
        client_order_id:       OrderId,
        client_limit_order_id: Option<OrderId>,
        order_type:            NormalizedOrderType,
        side:                  NormalizedSide,
        price:                 OrderedValueType,
        limit_price:           Option<OrderedValueType>,
        quantity:              OrderedValueType,
        response_sender:       Sender<SimulatorRequestResponse>,
    },

    CreateOcoOrder {
        symbol:                   Ustr,
        list_client_order_id:     Option<OrderId>,
        stop_client_order_id:     OrderId,
        limit_client_order_id:    OrderId,
        side:                     NormalizedSide,
        quantity:                 OrderedValueType,
        price:                    OrderedValueType,
        stop_price:               OrderedValueType,
        stop_limit_price:         Option<OrderedValueType>,
        stop_limit_time_in_force: Option<NormalizedTimeInForce>,
        response_sender:          Sender<SimulatorRequestResponse>,
    },

    CancelReplaceOrder {
        symbol:                   Ustr,
        exchange_order_id:        Option<OrderId>,
        cancel_order_id:          Option<OrderId>,
        original_client_order_id: Option<OrderId>, // This is the actual clientOrderId assigned by the system (e.g. local order id)
        new_client_order_id:      OrderId,
        new_order_type:           NormalizedOrderType,
        new_time_in_force:        NormalizedTimeInForce,
        new_quantity:             OrderedValueType,
        new_price:                OrderedValueType,
        new_limit_price:          Option<OrderedValueType>,
        response_sender:          Sender<SimulatorRequestResponse>,
    },

    CancelOrder {
        symbol:          Ustr,
        client_order_id: OrderId,
        response_sender: Sender<SimulatorRequestResponse>,
    },

    CancelAllOrdersForSymbol {
        symbol:          Ustr,
        response_sender: Sender<SimulatorRequestResponse>,
    },

    CancelAllOpenOrders {
        response_sender: Sender<SimulatorRequestResponse>,
    },
}

pub type SimulatorRequestResponse = Result<(), SimulatorError>;

/// Simulator message sent to the client imitating the live feed from the exchange
#[derive(Debug, Clone)]
pub enum SimulatorMessage {
    CandleEvent(Candle),
    OrderEvent(NormalizedOrderEvent),
    BalanceUpdateEvent(NormalizedBalanceEvent),
    Error {
        code:              i32,
        msg:               String,
        local_description: String,
    },
}

/// These events are produced by orders internal to the simulator and serve as
/// a way to notify the client about the order changes
#[derive(Debug, Clone)]
pub enum SimulatorOrderEvent {
    OrderEvent(NormalizedOrderEvent),
    Error {
        code:              i32,
        msg:               String,
        local_description: String,
    },
}

impl SimulatorOrderEvent {
    pub fn new_limit_maker_rejected() -> Self {
        Self::Error {
            code:              -2010,
            msg:               "Order would immediately match and take.".to_string(),
            local_description: "LIMIT_MAKER order type would immediately match and trade, and not be a pure maker order.".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SimulatorConfig {
    //
}

#[derive(Debug, Clone)]
pub struct Simulator(Arc<RwLock<SimulatorInner>>);

// TODO: implement order lifetimes (GTC, GTD, IOC, FOK)
// TODO: implement order stoploss
// TODO: implement order OCO
// TODO: implement fake latency
// TODO: implement rate limit
// TODO: implement user balance management i.e. fake accounts
// TODO: implement occasional order rejections
// TODO: Implement borrowing and repay for simulator; shorting, long
// TODO: maybe make grid runs concurrent
// TODO: maybe simulate network delays by skipping random N price changes
// TODO: look forward to having multiple coin simulations simultaneously and a way to have them interact with each other via correlation or something
// TODO: look forward to having multiple exchanges simulated at once
#[derive(Debug, Clone)]
struct SimulatorInner {
    pub config:                    SimulatorConfig,
    pub generator:                 Generator,
    pub client_to_internal_id_map: IntMap<OrderId>,
    pub orders:                    IndexMap<OrderId, SimulatorOrder>,
    pub trades:                    Vec<(OrderId, NormalizedSide)>,
    pub stats:                     SimulationStats,
    pub market_price:              Arc<AtomicF64>,
    pub previous_market_price:     OrderedValueType,
}

impl Simulator {
    pub fn new(config: SimulatorConfig, generator_config: GeneratorConfig) -> Self {
        Self(Arc::new(RwLock::new(SimulatorInner {
            generator: Generator::new(generator_config),
            config,
            orders: Default::default(),
            trades: vec![],
            stats: Default::default(),
            market_price: Arc::new(AtomicF64::new(0.0)),
            previous_market_price: Default::default(),
            client_to_internal_id_map: Default::default(),
        })))
    }

    pub fn config(&self) -> MappedRwLockReadGuard<SimulatorConfig> {
        RwLockReadGuard::map(self.0.read(), |i| &i.config)
    }

    pub fn config_mut(&self) -> MappedRwLockWriteGuard<SimulatorConfig> {
        RwLockWriteGuard::map(self.0.write(), |i| &mut i.config)
    }

    pub fn generator(&self) -> MappedRwLockReadGuard<Generator> {
        RwLockReadGuard::map(self.0.read(), |i| &i.generator)
    }

    pub fn generator_mut(&self) -> MappedRwLockWriteGuard<Generator> {
        RwLockWriteGuard::map(self.0.write(), |i| &mut i.generator)
    }

    pub fn stats(&self) -> MappedRwLockReadGuard<SimulationStats> {
        RwLockReadGuard::map(self.0.read(), |i| &i.stats)
    }

    pub fn computed_generator_state(&self) -> MappedRwLockReadGuard<TimeSet<ComputedGeneratorState>> {
        RwLockReadGuard::map(self.0.read(), |i| &i.generator.computed_generator_state)
    }

    pub fn orders(&self) -> MappedRwLockReadGuard<IndexMap<OrderId, SimulatorOrder>> {
        RwLockReadGuard::map(self.0.read(), |i| &i.orders)
    }

    pub fn trades(&self) -> MappedRwLockReadGuard<Vec<(OrderId, NormalizedSide)>> {
        RwLockReadGuard::map(self.0.read(), |i| &i.trades)
    }

    pub fn market_price_snapshot(&self) -> OrderedValueType {
        f!(self.0.read().market_price.load(Ordering::SeqCst))
    }

    pub fn atomic_market_price(&self) -> Arc<AtomicF64> {
        self.0.read().market_price.clone()
    }

    pub fn inc_ticks(&mut self) {
        self.0.write().stats.ticks += 1;
    }

    pub fn inc_quote_turnover(&mut self, value: ValueType) {
        self.0.write().stats.total_quote_turnover += value;
    }

    pub fn inc_base_turnover(&mut self, value: ValueType) {
        self.0.write().stats.total_base_turnover += value;
    }

    pub fn inc_orders_opened(&mut self) {
        self.0.write().stats.total_orders_opened += 1;
    }

    pub fn inc_orders_cancelled(&mut self) {
        self.0.write().stats.total_orders_cancelled += 1;
    }

    pub fn inc_orders_closed(&mut self) {
        self.0.write().stats.total_orders_closed += 1;
    }

    pub fn inc_orders_rejected(&mut self) {
        self.0.write().stats.total_orders_rejected += 1;
    }

    pub fn generate_candles(&mut self, timeframe: Timeframe, count: usize) -> Result<(), SimulatorError> {
        self.0.write().generator.generate_candles(timeframe, count)?;

        Ok(())
    }

    pub fn create_order(&mut self, mut order: SimulatorOrder) -> Result<(), SimulatorError> {
        // Validating the order before anything else
        order.validate()?;

        let market_price = self.market_price_snapshot();

        // ----------------------------------------------------------------------------
        // IMPORTANT:
        // The simulated order is initialized and added to the simulator, then handled
        // normally, meaning that events added here will still be sent to the client
        // ----------------------------------------------------------------------------

        // WARNING: Simple matching check for LIMIT_MAKER orders
        if order.is_limit_maker() && (order.is_buy() && market_price <= order.price || order.is_sell() && market_price >= order.price) {
            self.0.write().stats.total_orders_rejected += 1;
            return Err(SimulatorError::CodedError {
                code:              -2010,
                msg:               "Order would immediately match and take.".to_string(),
                local_description: "LIMIT_MAKER order type would immediately match and trade, and not be a pure maker order.".to_string(),
            });
        }

        let mut inner = self.0.write();

        // Need to map client order ids to internal ids to be able to
        // find the order by either of them
        let mut new_order_id_mapping = None;

        match inner.orders.entry(order.id) {
            Entry::Occupied(_) => return Err(SimulatorError::DuplicateOrder(order.id)),
            Entry::Vacant(entry) => {
                order.state = OrderState::Opened;
                order.events.push(SimulatorOrderEvent::OrderEvent(order.generate_event()));

                if order.client_order_id == 0 {
                    // NOTE: Just making it act like a real exchange e.g. Binance says it's random generated if not provided
                    order.client_order_id = new_id_u64();
                }

                // Remembering the new mapped client order id to map
                // outside this scope
                new_order_id_mapping = Some((order.client_order_id, order.id));

                entry.insert(order);
            }
        }

        // Incrementing the total orders opened
        inner.stats.total_orders_opened += 1;

        // Mapping the new client order id to the internal order id
        if let Some((client_order_id, internal_order_id)) = new_order_id_mapping {
            inner.client_to_internal_id_map.insert(client_order_id, internal_order_id);
        }

        Ok(())
    }

    pub fn create_oco_order(
        &mut self,
        symbol: Ustr,
        list_client_order_id: Option<OrderId>,
        stop_client_order_id: OrderId,
        limit_client_order_id: OrderId,
        side: NormalizedSide,
        quantity: OrderedValueType,
        price: OrderedValueType,
        stop_price: OrderedValueType,
        stop_limit_price: Option<OrderedValueType>,
        stop_limit_time_in_force: Option<NormalizedTimeInForce>,
    ) -> Result<(), SimulatorError> {
        let market_price = self.market_price_snapshot();
        let limit_order_id = new_id_u64();
        let stop_order_id = new_id_u64();
        let time_in_force = stop_limit_time_in_force.unwrap_or(NormalizedTimeInForce::GTC);

        let limit_order = SimulatorOrder {
            id: limit_order_id,
            other_order_id: Some(stop_order_id),
            is_cancelled_by_other_order: false,
            mode: SimulatorOrderMode::Regular,
            client_order_id: limit_client_order_id,
            client_stop_order_id: None,
            client_limit_order_id: None,
            client_list_order_id: list_client_order_id,
            client_cancel_order_id: None,
            client_original_order_id: None,
            symbol,
            side,
            order_type: NormalizedOrderType::Limit,
            time_in_force,
            price,
            limit_price: None,
            initial_quantity: quantity,
            filled_quantity: f!(0.0),
            fills: vec![],
            state: OrderState::Idle,
            atomic_market_price: self.atomic_market_price().clone(),
            replace_with_on_cancel: None,
            events: vec![],
        };

        let stop_order = SimulatorOrder {
            id: stop_order_id,
            other_order_id: Some(limit_order_id),
            is_cancelled_by_other_order: false,
            mode: SimulatorOrderMode::Regular,
            client_order_id: stop_client_order_id,
            client_stop_order_id: None,
            client_limit_order_id: None,
            client_list_order_id: list_client_order_id,
            client_cancel_order_id: None,
            client_original_order_id: None,
            symbol,
            side,
            order_type: if stop_limit_price.is_some() { NormalizedOrderType::StopLossLimit } else { NormalizedOrderType::StopLoss },
            time_in_force,
            price: stop_price,
            limit_price: stop_limit_price,
            initial_quantity: quantity,
            filled_quantity: f!(0.0),
            fills: vec![],
            state: OrderState::Idle,
            atomic_market_price: self.atomic_market_price().clone(),
            replace_with_on_cancel: None,
            events: vec![],
        };

        limit_order.validate()?;
        stop_order.validate()?;

        self.create_order(limit_order)?;
        self.create_order(stop_order)?;

        Ok(())
    }

    pub fn cancel_replace_order(
        &mut self,
        symbol: Ustr,
        exchange_order_id: Option<OrderId>,
        cancel_order_id: Option<OrderId>,
        original_client_order_id: Option<OrderId>, // This is the actual clientOrderId assigned by the system (e.g. local order id)
        new_client_order_id: OrderId,
        new_order_type: NormalizedOrderType,
        new_time_in_force: NormalizedTimeInForce,
        new_quantity: OrderedValueType,
        new_price: OrderedValueType,
        new_limit_price: Option<OrderedValueType>,
    ) -> Result<(), SimulatorError> {
        let market_price = self.market_price_snapshot();
        let mut inner = self.0.write();

        if let Some(exchange_order_id) = exchange_order_id {
            if !inner.orders.contains_key(&exchange_order_id) {
                return Err(SimulatorError::OrderNotFound(exchange_order_id));
            }
        }

        if let Some(original_client_order_id) = original_client_order_id {
            // Checking by the new client order id (external id) whether the order is already in use
            if inner.client_to_internal_id_map.contains_key(new_client_order_id) {
                return Err(SimulatorError::DuplicateClientOrderId(new_client_order_id));
            }
        }

        let mut replacement: SimulatorOrder;
        let mut original_order_internal_id;
        let new_internal_order_id = new_id_u64();

        // Attempting to find the original order by the client_original_order_id or id
        {
            let original_order = if let Some(exchange_order_id) = exchange_order_id {
                inner
                    .orders
                    .get_mut(&exchange_order_id)
                    .ok_or(SimulatorError::OrderNotFound(exchange_order_id))?
            } else {
                let Some(original_client_order_id) = original_client_order_id else {
                    return Err(SimulatorError::OrderNotFound(0));
                };

                let mapped_internal_id = inner
                    .client_to_internal_id_map
                    .get(original_client_order_id)
                    .cloned()
                    .ok_or(SimulatorError::OrderNotFound(original_client_order_id))?;

                inner
                    .orders
                    .get_mut(&mapped_internal_id)
                    .ok_or(SimulatorError::OrderNotFound(mapped_internal_id))?
            };

            original_order_internal_id = original_order.id;

            if let Some(original_client_order_id) = original_client_order_id {
                if original_client_order_id != original_order.client_order_id {
                    return Err(SimulatorError::Error("Original client order id does not match the existing order".to_string()));
                }
            }

            if original_order.state != OrderState::Opened {
                return Err(SimulatorError::OrderIsNotOpen(original_order.id));
            }

            // Constructing the new order from the original order
            replacement = SimulatorOrder {
                symbol,
                id: new_internal_order_id,
                other_order_id: None,
                is_cancelled_by_other_order: false,
                mode: SimulatorOrderMode::Regular,
                client_order_id: new_client_order_id,
                client_stop_order_id: None,
                client_limit_order_id: None,
                client_list_order_id: None,
                client_cancel_order_id: cancel_order_id,
                client_original_order_id: original_client_order_id,
                side: original_order.side,
                order_type: new_order_type,
                time_in_force: NormalizedTimeInForce::GTC,
                price: new_price,
                limit_price: new_limit_price,
                initial_quantity: original_order.initial_quantity - original_order.filled_quantity,
                filled_quantity: f!(0.0),
                fills: vec![],
                state: OrderState::Idle,
                atomic_market_price: original_order.atomic_market_price.clone(),
                replace_with_on_cancel: None,
                events: vec![],
            };

            if original_order.is_stoploss() && !replacement.is_stoploss() {
                return Err(SimulatorError::Error("Cannot convert a stop order to a regular order".to_string()));
            }

            if !original_order.is_stoploss() && replacement.is_stoploss() {
                return Err(SimulatorError::Error("Cannot convert a regular order to a stop order".to_string()));
            }

            if original_order.symbol != replacement.symbol {
                return Err(SimulatorError::Error("Symbol must not be changed".to_string()));
            }

            if original_order.side != replacement.side {
                return Err(SimulatorError::Error("Side must not be changed".to_string()));
            }

            // ----------------------------------------------------------------------------
            // IMPORTANT:
            // The simulated order is initialized and embedded into the original order,
            // then when the original order is cancelled, the embedded order is added to
            // the simulator, then handled normally, meaning that events added here will
            // still be sent to the client
            // ----------------------------------------------------------------------------
            original_order.replace_with_on_cancel = Some(Box::new(replacement));
        }

        // Dropping inner to release the lock
        drop(inner);

        // Cancel the original order
        self.cancel_order(symbol, original_order_internal_id, false)?;

        Ok(())
    }

    pub fn cancel_order(&mut self, symbol: Ustr, order_id: OrderId, is_cancelled_by_other_order: bool) -> Result<(), SimulatorError> {
        // TODO: first attempt to get the order from the orders map by the order id
        // TODO: then if not found, attempt to get the order id from the client_to_internal_id_map
        // TODO: then attempt to get the order from the orders map by the internal id returned from the client_to_internal_id_map

        let mut inner = self.0.write();

        let mapped_internal_id = inner.client_to_internal_id_map.get(order_id).cloned();

        match inner.orders.get_mut(&order_id) {
            Some(order) => {
                info!("Cancelling order {} by internal id", order_id);

                // The order has been found by the primary (internal) id
                order.cancel(is_cancelled_by_other_order)?;
            }
            None => {
                // The order has not been found by the primary (internal) id,
                // so trying to find it by the mapped client id

                let Some(mapped_internal_id) = mapped_internal_id else {
                    return Err(SimulatorError::OrderNotFound(order_id));
                };

                info!("Cancelling order {} by client order id {}", mapped_internal_id, order_id);

                let order = inner.orders.get_mut(&mapped_internal_id).ok_or(SimulatorError::OrderNotFound(order_id))?;

                order.cancel(is_cancelled_by_other_order)?;
            }
        }

        inner.stats.total_orders_cancelled += 1;

        Ok(())
    }

    pub fn get_order(&self, order_id: OrderId) -> Result<MappedRwLockReadGuard<'_, SimulatorOrder>, SimulatorError> {
        RwLockReadGuard::try_map(self.0.read(), |inner| match inner.orders.get(&order_id) {
            Some(order) => Some(order),
            None => match inner.client_to_internal_id_map.get(order_id) {
                Some(internal_order_id) => inner.orders.get(internal_order_id),
                None => None,
            },
        })
        .map_err(|_| SimulatorError::OrderNotFound(order_id))
    }

    pub fn get_order_mut(&mut self, order_id: OrderId) -> Result<MappedRwLockWriteGuard<'_, SimulatorOrder>, SimulatorError> {
        let internal_order_id = {
            let inner = self.0.read();
            match inner.orders.get(&order_id) {
                Some(_) => Some(order_id),
                None => inner.client_to_internal_id_map.get(order_id).cloned(),
            }
        };

        RwLockWriteGuard::try_map(self.0.write(), |inner| internal_order_id.and_then(|id| inner.orders.get_mut(&id)))
            .map_err(|_| SimulatorError::OrderNotFound(order_id))
    }

    pub fn compute(&mut self) -> Result<(Vec<SimulatorOrderEvent>, Vec<Candle>), SimulatorError> {
        // The previous market price is used to compute the order state
        let previous_market_price = self.0.read().previous_market_price;

        // Generating a new tick
        let (market_price, volume, candles) = {
            let (price, volume, candles) = self.0.write().generator.compute()?;
            (f!(price), f!(volume), candles)
        };

        // The last price is stored in the atomic variable for the UI to access
        self.0.write().market_price.store(market_price.0, Ordering::SeqCst);

        // Events to be returned (produced by the orders)
        let mut order_events: Vec<SimulatorOrderEvent> = vec![];

        // Unretained client order ids for further removal
        let mut unretained_client_order_ids = vec![];
        let mut replaced_orders = vec![];
        let mut cancelled_other_order_ids = vec![];

        self.0.write().orders.retain(|id, order| {
            order.compute(previous_market_price, market_price).expect("Failed to compute order");

            // Determining whether the order needs to be removed
            let is_retained = match order.state {
                OrderState::OpenFailed => false,
                OrderState::Opened => true,
                OrderState::PartiallyFilled => true,
                OrderState::Filled => false,
                OrderState::Cancelled => false,
                OrderState::CancelFailed => true,
                OrderState::Rejected => false,
                OrderState::Expired => false,
                _ => false,
            };

            // Taking out order events to notify the client about updates
            order_events.extend(order.events.drain(..));

            // Checking whether the `other` order needs to be cancelled
            if !order.is_cancelled_by_other_order {
                // IMPORTANT: OCO's other order is cancelled once the first order is partially or fully filled, or cancelled
                if matches!(order.state, OrderState::PartiallyFilled | OrderState::Filled | OrderState::Cancelled | OrderState::Finished) {
                    if let Some(other_order_id) = order.other_order_id {
                        cancelled_other_order_ids.push((order.symbol, other_order_id));
                    }
                }
            }

            // Remembering the order id if it's not retained
            if !is_retained {
                // If the order is cancelled, then need to take care of the possible replacement
                if order.state == OrderState::Cancelled {
                    // Extracting the replacement order if it's there
                    if let Some(replacement) = order.replace_with_on_cancel.take() {
                        replaced_orders.push(*replacement);
                    }
                }

                unretained_client_order_ids.push(order.client_order_id);
            }

            is_retained
        });

        // Replacing the cancelled orders with the replacement orders
        for mut replacement in replaced_orders {
            replacement.state = OrderState::Opened;
            replacement.events.push(SimulatorOrderEvent::OrderEvent(replacement.generate_event()));
            let mut lock = self.0.write();
            lock.client_to_internal_id_map.insert(replacement.client_order_id, replacement.id);
            lock.orders.insert(replacement.id, replacement);
            lock.stats.total_orders_opened += 1;
            lock.stats.total_orders_replaced += 1;
        }

        // Cancelling the other orders
        for (symbol, other_order_id) in cancelled_other_order_ids {
            self.cancel_order(symbol, other_order_id, true)?;
        }

        // Removing the client order id mapping
        for client_order_id in unretained_client_order_ids {
            self.0.write().client_to_internal_id_map.remove(client_order_id);
        }

        // Updating the previous market price
        self.0.write().previous_market_price = market_price;

        // println!("SIMULATOR ORDERS = {:#?}", self.orders());

        Ok((order_events, candles))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        order::OrderState,
        simulator::Simulator,
        simulator_generator::GeneratorConfig,
        types::{NormalizedOrderType, NormalizedSide, NormalizedTimeInForce},
    };
    use rand::Rng;
    use std::time::Instant;

    #[test]
    fn test_simulator() {
        let mut rng = rand::thread_rng();
        let mut simulator = Simulator::new(SimulatorConfig {}, GeneratorConfig::default());
        // simulator.generator.generate_candles(Timeframe::M1, 10000).unwrap();

        let mut order = SimulatorOrder {
            id:                          0,
            other_order_id:              None,
            is_cancelled_by_other_order: false,
            mode:                        Default::default(),
            client_order_id:             0,
            client_limit_order_id:       None,
            client_list_order_id:        None,
            client_stop_order_id:        None,
            client_cancel_order_id:      None,
            symbol:                      ustr("TESTUSDT"),
            side:                        Default::default(),
            order_type:                  Default::default(),
            time_in_force:               NormalizedTimeInForce::GTC,
            price:                       Default::default(),
            limit_price:                 None,
            initial_quantity:            f!(100.0),
            filled_quantity:             f!(0.0),
            fills:                       vec![],
            state:                       OrderState::Idle,
            events:                      vec![],
            client_original_order_id:    None,
            atomic_market_price:         Arc::new(Default::default()),
            replace_with_on_cancel:      None,
        };

        order.id = 1;
        order.side = NormalizedSide::Sell;
        order.price = f!(rng.gen_range(95.0..=105.0));
        order.order_type = NormalizedOrderType::Limit;
        simulator.create_order(order.clone()).unwrap();

        order.id = 2;
        order.side = NormalizedSide::Buy;
        order.order_type = NormalizedOrderType::Market;
        order.price = f!(rng.gen_range(95.0..=105.0));
        simulator.create_order(order.clone()).unwrap();

        order.id = 3;
        order.side = NormalizedSide::Sell;
        order.order_type = NormalizedOrderType::Limit;
        order.price = f!(rng.gen_range(95.0..=105.0));
        simulator.create_order(order.clone()).unwrap();

        // println!("{:#?}", simulator.orders);
        // return;

        /*
        for _ in 0..1000 {
            let processed_orders = simulator.compute().unwrap();

            if processed_orders.len() > 0 {
                println!("{:#?}", processed_orders);
            }
        }
         */

        let it = Instant::now();
        while !simulator.orders().is_empty() {
            let processed_orders = simulator.compute().unwrap();
        }
        println!("{:#?}", it.elapsed());

        println!("{:#?}", simulator.orders());
    }
}
