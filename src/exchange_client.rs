use binance_spot_connector_rust::{
    http::{request::Request, Credentials},
    trade::{
        cancel_order::CancelOrder,
        new_order::NewOrder,
        order::{NewOrderResponseType, Side, TimeInForce},
    },
    ureq::Response,
};
use chrono::{Duration, Utc};
use crossbeam_channel::Sender;
use itertools::Itertools;
use log::{error, info, warn};
use num_traits::{FromPrimitive, Zero};
use rand::RngCore;
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use std::{cmp::Ordering, collections::BinaryHeap, fmt::Display, time::Instant};
use thiserror::__private::AsDynError;
use yata::core::ValueType;

use crate::{
    coin_worker::CoinWorkerMessage,
    id::OrderId,
    instruction::{ExecutionOutcome, Instruction, InstructionExecutionResult, InstructionMetadata},
    model::AccountInformation,
    order::{OrderError, OrderFill},
    ratelimiter::UnifiedRateLimiter,
    trader::{Trader, TraderError},
    types::{ExchangeApi, ExchangeApiError, NormalizedOrderEvent, NormalizedSide, NormalizedTimeInForce, OrderedValueType, Priority},
    util::{itoa_u64, s2u64},
};
use thiserror::Error;
use threadpool::ThreadPool;
use ustr::{ustr, Ustr, UstrMap};

#[derive(Error, Debug)]
pub enum ExchangeClientError {
    #[error("Operation is ignored due to rate limiter")]
    RateLimited,

    #[error("Order quote and quantity is not specified")]
    QuoteAndQuantityNotSpecified,

    #[error("No credentials provided")]
    NoCredentials,

    #[error("Order error: {0}")]
    OrderError(#[from] OrderError),

    #[error("Trader error: {0}")]
    TraderError(#[from] TraderError),

    #[error("Error: {0}")]
    ErrorDescription(String),

    #[error("Exchange API error: {0}")]
    ExchangeApiError(#[from] ExchangeApiError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ExchangeClientError {
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited)
    }

    pub fn is_quote_and_quantity_not_specified(&self) -> bool {
        matches!(self, Self::QuoteAndQuantityNotSpecified)
    }

    pub fn is_order_error(&self) -> bool {
        matches!(self, Self::OrderError(_))
    }

    pub fn is_trader_error(&self) -> bool {
        matches!(self, Self::TraderError(_))
    }

    pub fn is_error_with_description(&self) -> bool {
        matches!(self, Self::ErrorDescription(_))
    }

    pub fn is_other(&self) -> bool {
        matches!(self, Self::Other(_))
    }
}

pub trait ExchangeClient {
    fn push_instruction(
        &mut self,
        instruction_id: u64,
        metadata: InstructionMetadata,
        instruction: Instruction,
        priority: Priority,
    ) -> Result<(), ExchangeClientError>;

    fn confirm_opened(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn confirm_open_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn confirm_filled(&mut self, symbol: Ustr, local_order_id: OrderId, last_fill: Option<OrderFill>) -> Result<(), ExchangeClientError>;

    fn confirm_cancelled(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn confirm_cancel_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn confirm_rejected(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn notify_expired(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn notify_closed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn confirm_replaced(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn notify_replace_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn update_filled_both_amounts(
        &mut self,
        symbol: Ustr,
        local_order_id: OrderId,
        quantity: OrderedValueType,
        quote: OrderedValueType,
    ) -> Result<(), ExchangeClientError>;

    fn limit_buy(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        price: OrderedValueType,
        quantity: OrderedValueType,
        tif: NormalizedTimeInForce,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError>;

    fn stop_limit_buy(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        quote: OrderedValueType,
        stop_price: OrderedValueType,
        stop_limit_price: OrderedValueType,
        tif: NormalizedTimeInForce,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError>;

    fn market_buy(&mut self, order_id: OrderId, symbol: Ustr, quote: OrderedValueType, recv_window: u64) -> Result<(), ExchangeClientError>;

    fn limit_sell(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        price: OrderedValueType,
        quantity: OrderedValueType,
        tif: NormalizedTimeInForce,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError>;

    fn cancel_replace_sell(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        price: OrderedValueType,
        quantity: OrderedValueType,
        tif: NormalizedTimeInForce,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError>;

    fn market_sell(&mut self, order_id: OrderId, symbol: Ustr, quantity: OrderedValueType, recv_window: u64) -> Result<(), ExchangeClientError>;

    fn cancel_order_by_internal_id(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError>;

    fn cancel_all_orders(&mut self, symbol: Ustr) -> Result<(), ExchangeClientError>;

    fn cancel_all_orders_for_all_symbols(&mut self) -> Result<(), ExchangeClientError>;
}

#[derive(Debug)]
pub enum ExchangeClientMessage {
    NormalizedUserTradeEvent(NormalizedOrderEvent),
    Instruction(u64, InstructionMetadata, Instruction, Priority),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum TradeSignal {
    #[default]
    None,
    Buy,
    BuyQuantity(OrderedValueType),
    Sell,
    SellQuantity(OrderedValueType),
}

#[derive(Debug)]
struct CoinMetadata {
    pub market_price:    OrderedValueType,
    pub price_step:      OrderedValueType,
    pub stop_exit_point: OrderedValueType,
}

#[derive(Debug)]
pub struct ExchangeClientJob {
    pub instruction_id: u64,
    pub metadata:       InstructionMetadata,
    pub instruction:    Instruction,
    pub priority:       Priority,
}

impl Eq for ExchangeClientJob {}

impl Ord for ExchangeClientJob {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

impl PartialEq<Self> for ExchangeClientJob {
    fn eq(&self, other: &Self) -> bool {
        self.instruction == other.instruction
    }
}

impl PartialOrd<Self> for ExchangeClientJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.priority.partial_cmp(&other.priority)
    }
}

pub struct BinanceExchangeClient {
    /// ----------------------------------------------------------------------------
    /// WARNING: THESE FLAGS MUST BE DEALT WITH UTMOST CARE
    is_enabled:         bool,
    is_trading_allowed: bool,
    /// ----------------------------------------------------------------------------

    /// WARNING: this thread pool is meant only for executing calls to the exchange
    threadpool:         ThreadPool,

    /// WARNING: this is a trader that is used to handle all trading
    trader: Trader,

    /// This is a priority queue of instructions that are to be executed
    jobs: BinaryHeap<ExchangeClientJob>,

    /// This is a shared, global rate limiter that is used for everything
    ratelimiter: UnifiedRateLimiter,

    /// This is a struct that combines all Binance-related APIs
    exchange_api: ExchangeApi,
}

impl BinanceExchangeClient {
    pub fn new(exchange_api: ExchangeApi, ratelimiter: UnifiedRateLimiter, trader: Trader) -> Self {
        // WARNING: this thread pool is meant only for executing calls to the exchange
        let threadpool = ThreadPool::new(num_cpus::get());

        trader.validate().expect("trader validation failed");

        Self {
            is_trading_allowed: false,
            exchange_api,
            is_enabled: false,
            trader,
            jobs: BinaryHeap::new(),
            ratelimiter,
            threadpool,
        }
    }

    fn wait_for_ratelimiter(&mut self, order_weight: u32, request_weight: u32) {
        self.ratelimiter.wait_for_order_limit_per_second(order_weight);
        self.ratelimiter.wait_for_order_limit_per_day(order_weight);
        self.ratelimiter.wait_for_request_weight_limit(request_weight);
        self.ratelimiter.wait_for_raw_request_limit(request_weight);
    }

    pub fn compute(&mut self) -> Result<(), ExchangeClientError> {
        let job = match self.jobs.pop() {
            None => return Ok(()),
            Some(job) => job,
        };

        let (symbol, result, execution_time) = match job.instruction {
            Instruction::CreateOrder {
                symbol,
                client_order_id,
                side,
                order_type,
                quantity,
                quantity_quote,
                quantity_iceberg,
                price,
                time_in_force,
                stop_price,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let mut new_order = NewOrder::new(
                    symbol.as_str(),
                    match side {
                        NormalizedSide::Buy => Side::Buy,
                        NormalizedSide::Sell => Side::Sell,
                    },
                    order_type.as_ref(),
                );

                if let Some(client_order_id) = client_order_id {
                    new_order = new_order.new_client_order_id(client_order_id.to_string().as_str());
                }

                if let Some(time_in_force) = time_in_force {
                    let time_in_force = match time_in_force {
                        NormalizedTimeInForce::GTC => TimeInForce::Gtc,
                        NormalizedTimeInForce::IOC => TimeInForce::Ioc,
                        NormalizedTimeInForce::FOK => TimeInForce::Fok,
                        NormalizedTimeInForce::GTX => panic!("GTX is not supported"),
                    };

                    new_order = new_order.time_in_force(time_in_force);
                }

                if let Some(price) = price {
                    new_order = new_order.price(Decimal::from_f64(price.0).expect("price conversion to rust_decimal failed"));
                }

                if let Some(stop_price) = stop_price {
                    new_order = new_order.stop_price(Decimal::from_f64(stop_price.0).expect("stop_price conversion to rust_decimal failed"));
                }

                if let Some(quantity_iceberg) = quantity_iceberg {
                    new_order = new_order.iceberg_qty(Decimal::from_f64(quantity_iceberg.0).expect("quantity_iceberg conversion to rust_decimal failed"));
                }

                if let Some(quote_quantity) = quantity_quote {
                    new_order = new_order.quote_order_qty(Decimal::from_f64(quote_quantity.0).expect("quote_quantity conversion to rust_decimal failed"));
                } else {
                    new_order = new_order.quantity(Decimal::from_f64(quantity.0).expect("quantity conversion to rust_decimal failed"));
                }

                eprintln!("new_order = {:#?}", new_order);

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(new_order.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::CancelOrder { symbol, local_order_id } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let request = CancelOrder::new(symbol.as_str()).new_client_order_id(local_order_id.to_string().as_str());

                eprintln!("cancel request = {:#?}", request);

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(request.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::LimitBuy {
                symbol,
                local_order_id,
                order_type,
                quantity,
                price,
                is_post_only,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let order_type = if is_post_only { "LIMIT_MAKER" } else { order_type.as_ref() };

                let mut new_order = NewOrder::new(symbol.as_str(), Side::Buy, order_type.as_ref())
                    .new_client_order_id(local_order_id.to_string().as_str())
                    .price(Decimal::from_f64(price.0).expect("price conversion to rust_decimal failed"))
                    .quantity(Decimal::from_f64(quantity.0).expect("quantity conversion to rust_decimal failed"));

                eprintln!("new_order = {:#?}", new_order);

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(new_order.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::LimitSell {
                symbol,
                local_order_id,
                order_type,
                quantity,
                price,
                is_post_only,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let order_type = if is_post_only { "LIMIT_MAKER" } else { order_type.as_ref() };

                let mut new_order = NewOrder::new(symbol.as_str(), Side::Sell, order_type.as_ref())
                    .new_client_order_id(local_order_id.to_string().as_str())
                    .price(Decimal::from_f64(price.0).expect("price conversion to rust_decimal failed"))
                    .quantity(Decimal::from_f64(quantity.0).expect("quantity conversion to rust_decimal failed"));

                eprintln!("new_order = {:#?}", new_order);

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(new_order.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::MarketBuy {
                symbol,
                local_order_id,
                quantity,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let request = NewOrder::new(symbol.as_str(), Side::Buy, "MARKET")
                    .new_client_order_id(local_order_id.to_string().as_str())
                    .quantity(Decimal::from_f64(quantity.0).expect("quantity conversion to rust_decimal failed"));

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(request.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::MarketBuyQuoteQuantity {
                symbol,
                local_order_id,
                quote_quantity,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let request = NewOrder::new(symbol.as_str(), Side::Buy, "MARKET")
                    .new_client_order_id(local_order_id.to_string().as_str())
                    .quote_order_qty(Decimal::from_f64(quote_quantity.0).expect("quote_quantity conversion to rust_decimal failed"));

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(request.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::MarketSell {
                symbol,
                local_order_id,
                quantity,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let request = NewOrder::new(symbol.as_str(), Side::Sell, "MARKET")
                    .new_client_order_id(local_order_id.to_string().as_str())
                    .quantity(Decimal::from_f64(quantity.0).expect("quantity conversion to rust_decimal failed"));

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(request.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            Instruction::CreateStop {
                symbol,
                order_type,
                side,
                local_order_id,
                local_stop_order_id,
                price,
                stop_price,
                quantity,
            } => {
                let Some(credentials) = self.exchange_api.credentials.clone() else {
                    return Err(ExchangeClientError::NoCredentials);
                };

                let order_type = order_type.as_ref();
                let side = match side {
                    NormalizedSide::Buy => Side::Buy,
                    NormalizedSide::Sell => Side::Sell,
                };

                let mut new_stop_order = NewOrder::new(symbol.as_str(), side, order_type)
                    .new_client_order_id(local_order_id.to_string().as_str())
                    .stop_price(Decimal::from_f64(stop_price.0).expect("stop_price conversion to rust_decimal failed"))
                    .quantity(Decimal::from_f64(quantity.0).expect("quantity conversion to rust_decimal failed"));

                if let Some(price) = price {
                    new_stop_order = new_stop_order.price(Decimal::from_f64(price.0).expect("price conversion to rust_decimal failed"));
                }

                eprintln!("new_stop_order = {:#?}", new_stop_order);

                let execution_start = Instant::now();
                let result = self
                    .exchange_api
                    .client
                    .send(new_stop_order.credentials(&credentials))
                    .map_err(|err| self.exchange_api.handle_error(*err));
                let execution_time = execution_start.elapsed();

                (symbol, result, execution_time)
            }

            instruction @ _ => {
                warn!("Unsupported instruction: {:#?}", instruction);
                (ustr("UNKNOWN_SYMBOL"), Err(ExchangeApiError::UnknownError), core::time::Duration::default())
            }
        };

        // Confirming the result of the instruction
        self.trader.confirm_instruction_execution(match result {
            Ok(response) => InstructionExecutionResult {
                instruction_id: job.instruction_id,
                metadata:       job.metadata,
                symbol:         Some(symbol),
                outcome:        Some(ExecutionOutcome::Success),
                executed_at:    Some(Utc::now()),
                execution_time: Some(execution_time),
            },
            Err(err) => InstructionExecutionResult {
                instruction_id: job.instruction_id,
                metadata:       job.metadata,
                symbol:         Some(symbol),
                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                executed_at:    Some(Utc::now()),
                execution_time: Some(execution_time),
            },
        })?;

        Ok(())
    }

    pub fn push_instruction(
        &mut self,
        instruction_id: u64,
        metadata: InstructionMetadata,
        instruction: Instruction,
        priority: Priority,
    ) -> Result<(), ExchangeClientError> {
        self.jobs.push(ExchangeClientJob {
            instruction_id,
            metadata,
            instruction,
            priority,
        });

        self.compute()
    }

    pub fn confirm_opened(&mut self, symbol: Ustr, local_order_id: OrderId, exchange_order_id: Option<u64>) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_opened(symbol, local_order_id, exchange_order_id)?;
        Ok(())
    }

    pub fn confirm_open_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_open_failed(symbol, local_order_id)?;
        Ok(())
    }

    pub fn confirm_filled(&mut self, symbol: Ustr, local_order_id: OrderId, last_fill: Option<OrderFill>) -> Result<(), ExchangeClientError> {
        let last_fill = match last_fill {
            None => return Err(ExchangeClientError::OrderError(OrderError::MissingOrderFillMetadata(symbol, local_order_id))),
            Some(fill) => fill,
        };

        // NOTE: Order fill is validated down the line
        match self.trader.confirm_order_filled(symbol, local_order_id, last_fill) {
            Ok(_) => Ok(()),
            Err(err) => Err(ExchangeClientError::TraderError(err)),
        }
    }

    pub fn confirm_cancelled(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_cancelled(symbol, local_order_id)?;
        Ok(())
    }

    pub fn confirm_cancel_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_cancel_failed(symbol, local_order_id)?;
        Ok(())
    }

    pub fn confirm_rejected(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_rejected(symbol, local_order_id)?;
        Ok(())
    }

    pub fn notify_expired(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_expired(symbol, local_order_id)?;
        Ok(())
    }

    pub fn confirm_replaced(
        &mut self,
        symbol: Ustr,
        local_original_order_id: OrderId,
        local_order_id: OrderId,
        event: NormalizedOrderEvent,
    ) -> Result<(), ExchangeClientError> {
        self.trader.confirm_order_replaced(symbol, local_original_order_id, local_order_id, event)?;
        Ok(())
    }

    pub fn notify_replace_failed(&mut self, symbol: Ustr, local_order_id: OrderId) -> Result<(), ExchangeClientError> {
        unimplemented!("notify_replace_failed");
        /*
        match self.orders.write().get_mut(&order_id) {
            None => Err(ExchangeClientError::OrderError(OrderError::OrderNotFound(order_id))),
            Some(order) => match order.confirm_replace_failed() {
                Ok(_) => Ok(()),
                Err(e) => Err(ExchangeClientError::OrderError(e)),
            },
        }
         */

        Ok(())
    }

    pub fn update_filled_both_amounts(
        &mut self,
        symbol: Ustr,
        local_order_id: OrderId,
        quantity: OrderedValueType,
        quote: OrderedValueType,
    ) -> Result<(), ExchangeClientError> {
        /*
        match self.orders.write().get_mut(&order_id) {
            None => Err(ExchangeClientError::OrderError(OrderError::OrderNotFound(order_id))),
            Some(order) => {
                order.update_filled_both_amounts(quantity, quote);
                Ok(())
            }
        }
         */

        Ok(())
    }

    pub fn limit_buy(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        price: ValueType,
        quantity: ValueType,
        time_in_force: NormalizedTimeInForce,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError> {
        // WARNING: quick fail due to rate limiting
        self.wait_for_ratelimiter(1, 1);
        /*

        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || {
            // if let Some(after_delay) = after_delay {
            //     sleep(after_delay);
            // }

            let result = Self::send_order_request(
                &account,
                Spot::Order,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    new_client_order_id: Some(itoa_u64(order_id)),
                    order_side: OrderSide::Buy,
                    order_type: Some(if is_post_only { OrderType::LimitMaker } else { OrderType::Limit }),
                    price: price.into(),
                    qty: Some(quantity),
                    recv_window,
                    time_in_force: (!is_post_only).then(|| tif),
                    ..Default::default()
                },
            );

            match result {
                Ok(Some(transaction)) => {
                    trader
                        .confirm_order_opened(symbol, order_id, Some(transaction.order_id))
                        .expect("confirm_order_opened failed (with transaction)");
                }
                Ok(None) => {
                    trader
                        .confirm_order_opened(symbol, order_id, None)
                        .expect("confirm_order_opened failed (without transaction)");
                }
                Err(err) => {
                    trader.confirm_order_open_failed(symbol, order_id).expect("confirm_order_open_failed failed");
                }
            }
            });
         */

        Ok(())
    }

    /// Creating a stop limit order is a two step process. This is a stop order that
    /// will be triggered when the price reaches the stop price. When the stop order
    /// is triggered, a limit order will be created with the stop limit price.
    /// It's like a stop loss order to protect against losses. Instead it is used
    /// to buy when the price reaches a certain level.
    /// NOTE: This could be used to buy when the price is breaking out in either direction.
    pub fn stop_limit_buy(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        quote: ValueType,
        stop_price: ValueType,
        stop_limit_price: ValueType,
        tif: NormalizedTimeInForce,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || {
            let result = Self::send_order_request(
                &account,
                Spot::Order,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    new_client_order_id: Some(order_id.to_string()),
                    order_side: OrderSide::Buy,
                    order_type: Some(if is_post_only { OrderType::LimitMaker } else { OrderType::Limit }),
                    stop_price: Some(stop_price),
                    stop_limit_price: Some(stop_limit_price),
                    stop_limit_time_in_force: Some(tif),
                    quote_order_qty: Some(quote.into()),
                    recv_window,
                    time_in_force: (!is_post_only).then(|| tif),
                    ..Default::default()
                },
            );

            match result {
                Ok(Some(transaction)) => {
                    println!("{:#?}", transaction);
                    trader
                        .confirm_order_opened(symbol, order_id, Some(transaction.order_id))
                        .expect("confirm_order_opened failed (with transaction)");
                }
                Ok(None) => {
                    trader
                        .confirm_order_opened(symbol, order_id, None)
                        .expect("confirm_order_opened failed (without transaction)");
                }
                Err(err) => {
                    println!("{:#?}", err);
                    trader.confirm_order_open_failed(symbol, order_id).expect("confirm_order_open_failed failed");
                }
            }
        });
         */

        Ok(())
    }

    pub fn market_buy(&mut self, order_id: OrderId, symbol: Ustr, quote: ValueType, recv_window: u64) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || {
            let result = Self::send_order_request(
                &account,
                Spot::Order,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    new_client_order_id: Some(order_id.to_string()),
                    order_side: OrderSide::Buy,
                    order_type: Some(OrderType::Market),
                    quote_order_qty: Some(quote.into()),
                    recv_window,
                    ..Default::default()
                },
            );

            match result {
                Ok(Some(transaction)) => {
                    trader
                        .confirm_order_opened(symbol, order_id, Some(transaction.order_id))
                        .expect("confirm_order_opened failed (with transaction returned)");
                    println!("{:#?}", transaction);
                }
                Ok(None) => {
                    trader
                        .confirm_order_opened(symbol, order_id, None)
                        .expect("confirm_order_opened failed (without transaction returned)");
                }
                Err(err) => {
                    trader.confirm_order_open_failed(symbol, order_id).expect("confirm_order_open_failed failed");
                    println!("{:#?}", err);
                }
            }
            });
             */

        Ok(())
    }

    pub fn limit_sell(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        price: ValueType,
        quantity: ValueType,
        tif: NormalizedTimeInForce,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || {
            // if let Some(after_delay) = after_delay {
            //     sleep(after_delay);
            // }

            let result = Self::send_order_request(
                &account,
                Spot::Order,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    new_client_order_id: Some(itoa_u64(order_id)),
                    order_side: OrderSide::Sell,
                    order_type: Some(if is_post_only { OrderType::LimitMaker } else { OrderType::Limit }),
                    price,
                    qty: Some(quantity),
                    recv_window,
                    time_in_force: (!is_post_only).then(|| tif),
                    ..Default::default()
                },
            );

            match result {
                Ok(Some(transaction)) => {
                    println!("{:#?}", transaction);
                    trader
                        .confirm_order_opened(symbol, order_id, Some(transaction.order_id))
                        .expect("confirm_order_opened failed (with transaction)");
                }
                Ok(None) => {
                    trader
                        .confirm_order_opened(symbol, order_id, None)
                        .expect("confirm_order_opened failed (without transaction)");
                }
                Err(err) => {
                    println!("{:#?}", err);
                    trader.confirm_order_open_failed(symbol, order_id).expect("confirm_order_open_failed failed");
                }
            }
            });
             */

        Ok(())
    }

    pub fn cancel_replace_sell(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        price: ValueType,
        quantity: ValueType,
        tif: NormalizedTimeInForce,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || {
            // if let Some(after_delay) = after_delay {
            //     sleep(after_delay);
            // }

            let result = Self::send_cancel_replace_request(
                &account,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    cancel_replace_mode: Some(OrderCancelReplaceMode::StopOnFailure),
                    cancel_orig_client_order_id: Some(itoa_u64(order_id)),
                    new_client_order_id: Some(itoa_u64(order_id)),
                    order_side: OrderSide::Sell,
                    order_type: Some(if is_post_only { OrderType::LimitMaker } else { OrderType::Limit }),
                    price,
                    qty: Some(quantity),
                    recv_window,
                    time_in_force: (!is_post_only).then(|| tif),
                    ..Default::default()
                },
            );

            match result {
                Ok(response) => {
                    // println!("{:#?}", response);

                    match response {
                        CancelReplaceResponse::Ok {
                            cancel_result,
                            new_order_result,
                            cancel_response,
                            new_order_response,
                        } => {
                            // ----------------------------------------------------------------------------
                            // handling `cancel_response`

                            match cancel_response {
                                None => {
                                    trader
                                        .confirm_order_replace_failed(symbol, order_id)
                                        .expect("confirm_order_replace_failed failed (cancel_response is none)");
                                }

                                Some(cancelled_order) => {
                                    /*
                                    match cancelled_order {
                                        TransactionOrError::Transaction(cancelled_transaction) => {
                                            info!("cancelled {} to replace", cancelled_transaction.client_order_id);

                                            if let Err(err) = tx.send(TradeMessage::NotifyCancelled {
                                                symbol:   symbol.clone(),
                                                order_id: cancelled_transaction.client_order_id,
                                            }) {
                                                error!("failed to notify the processor that cancel replace sell was successful");
                                            }
                                        }
                                        TransactionOrError::TransactionAck(cancelled_transaction_ack) => {
                                            info!("cancelled {} to replace", cancelled_transaction_ack.client_order_id);

                                            if let Err(err) = tx.send(TradeMessage::NotifyCancelled {
                                                symbol:   symbol.clone(),
                                                order_id: cancelled_transaction_ack.client_order_id,
                                            }) {
                                                error!("failed to notify the processor that cancel replace sell was successful");
                                            }
                                        }
                                        TransactionOrError::Error { code, msg } => {
                                            error!("CancelReplace failed with a nested error code (cancelled_transaction): code={} msg={:#?}", code, msg);

                                            if let Err(err) = tx.send(TradeMessage::NotifyCancelFailed {
                                                symbol:   symbol.clone(),
                                                order_id: order_id.clone(),
                                            }) {
                                                error!("failed to notify the processor that cancel replace sell failed");
                                            }
                                        }
                                    }
                                     */
                                }
                            }

                            // ----------------------------------------------------------------------------
                            // handling `new_order_response`

                            match new_order_response {
                                None => {
                                    trader
                                        .confirm_order_replace_failed(symbol, order_id)
                                        .expect("confirm_order_replace_failed failed (new_order_response is none)");
                                }

                                Some(new_order) => match new_order {
                                    TransactionOrError::Transaction(new_order_transaction) => {
                                        let order_id = s2u64(new_order_transaction.client_order_id.as_str()).expect(
                                            format!(
                                                "failed to convert `new_order_transaction.client_order_id` to u64: {}",
                                                new_order_transaction.client_order_id
                                            )
                                            .as_str(),
                                        );

                                        warn!("CANCEL_REPLACE: replaced {} with {}", order_id, new_order_transaction.order_id);

                                        /*
                                        // TODO: Actually, I don't think I want to confirm here, I'll just rely on the following ack event
                                        // WARNING: Trying to convert `new_order_transaction` to `NormalizedOrderEvent` here
                                        let event = NormalizedOrderEvent::from(new_order_transaction);

                                        trader
                                            .confirm_order_replaced(symbol, order_id, event)
                                            .expect("confirm_order_replaced failed (full transaction response)");
                                         */
                                    }
                                    TransactionOrError::TransactionAck(new_order_transaction_ack) => {
                                        let order_id = s2u64(new_order_transaction_ack.client_order_id.as_str()).expect(
                                            format!(
                                                "failed to convert `new_order_transaction_ack.client_order_id` to u64: {}",
                                                new_order_transaction_ack.client_order_id
                                            )
                                            .as_str(),
                                        );

                                        /*
                                        trader
                                            .confirm_order_replaced(symbol, order_id)
                                            .expect("confirm_order_replaced failed (ack; short transaction response)");
                                         */

                                        warn!("CANCEL_REPLACE: replaced {} with {}", order_id, new_order_transaction_ack.order_id);
                                    }
                                    TransactionOrError::Error { code, msg } => {
                                        error!("CancelReplace failed with a nested error code (new_order_transaction): code={} msg={:#?}", code, msg);
                                        trader
                                            .confirm_order_replace_failed(symbol, order_id)
                                            .expect("confirm_order_replace_failed failed (new_order_transaction error)");
                                    }
                                },
                            }
                        }

                        CancelReplaceResponse::Error { code, msg, data } => {
                            error!("CancelReplaceFailed with an error code: code={} msg={:#?} data={:#?}", code, msg, data);

                            trader
                                .confirm_order_replace_failed(symbol, order_id)
                                .expect("confirm_order_replace_failed failed (CancelReplaceResponse::Error)");
                        }
                    }
                }
                Err(err) => {
                    trader
                        .confirm_order_replace_failed(symbol, order_id)
                        .expect("confirm_order_replace_failed failed (send_order_request failed)");
                    println!("{:#?}", err);
                }
            }
        });
         */

        Ok(())
    }

    pub fn market_sell(&mut self, order_id: OrderId, symbol: Ustr, quantity: ValueType, recv_window: u64) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        warn!("EXECUTING MARKET SELL: {}", order_id);

        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || {
            let result = Self::send_order_request(
                &account,
                Spot::Order,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    new_client_order_id: Some(order_id.to_string()),
                    order_side: OrderSide::Sell,
                    order_type: Some(OrderType::Market),
                    qty: Some(quantity),
                    recv_window,
                    ..Default::default()
                },
            );

            match result {
                Ok(Some(transaction)) => {
                    trader
                        .confirm_order_opened(symbol, order_id, Some(transaction.order_id))
                        .expect("failed to confirm order opened (with transaction)");
                    println!("{:#?}", transaction);
                }
                Ok(None) => {
                    trader
                        .confirm_order_opened(symbol, order_id, None)
                        .expect("failed to confirm order opened (without transaction)");
                }
                Err(err) => {
                    println!("{:#?}", err);
                    trader.confirm_order_open_failed(symbol, order_id).expect("failed to confirm order open failed");
                }
            }
        });
         */

        Ok(())
    }

    /*
    // FIXME: fix this after other basic stufs is fixed
    pub fn sell_oco(
        &mut self,
        is_post_only: bool,
        order_id: OrderId,
        symbol: Ustr,
        qty: ValueType,
        price: ValueType,
        stop_price: Option<ValueType>,
        stop_limit_price: Option<ValueType>,
        tif: Option<NormalizedTimeInForce>,
        after_delay: Option<Duration>,
        recv_window: u64,
    ) -> Result<(), ExchangeClientError> {
        let tx = self.trade_event_tx.clone();

        // WARNING: quick fail due to rate limiting
        if let Err(err) = self.check_ratelimits(2, 1) {
            if let Err(err) = tx.send(ExchangeClientMessage::ConfirmOpenFailed {
                symbol:   symbol.clone(),
                order_id: order_id.clone(),
            }) {
                error!("failed to notify the processor that sell oco failed");
            }

            return Err(err);
        }

        let account = self.exchange_api.account.clone();
        let tx = self.trade_event_tx.clone();

        self.threadpool.execute(move || {
            // if let Some(after_delay) = after_delay {
            //     sleep(after_delay);
            // }

            let result = Self::send_oco_order_request(
                &account,
                OrderRequestExtended {
                    symbol: symbol.to_string(),
                    list_client_order_id: Some(format!("list_{}", order_id)),
                    limit_client_order_id: Some(format!("limit_{}", order_id)),
                    stop_client_order_id: Some(format!("stop_{}", order_id)),
                    order_side: OrderSide::Sell,
                    price,
                    stop_price,
                    stop_limit_price,
                    stop_limit_time_in_force: tif,
                    qty: Some(qty),
                    recv_window,
                    ..Default::default()
                },
            );

            match result {
                Ok(Some(transaction)) => {
                    info!("sell_oco is successful (with response)");
                    println!("{:#?}", transaction);
                }
                Ok(None) => {
                    info!("sell_oco is successful");
                }
                Err(err) => {
                    if let Err(err) = tx.send(ExchangeClientMessage::ConfirmOpenFailed { symbol, order_id }) {
                        error!("failed to send trade event to the processor");
                    }

                    println!("{:#?}", err);
                }
            }
        });

        Ok(())
    }
     */

    pub fn cancel_order_by_exchange_order_id(&mut self, account: &AccountInformation, symbol: Ustr, exchange_order_id: u64) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool
            .execute(move || match account.cancel_order_by_external_id(symbol.as_str(), exchange_order_id) {
                Ok(result) => {
                    trader
                        .confirm_order_cancelled(symbol, trader.external_id_to_internal_id(exchange_order_id))
                        .expect("failed to confirm order cancelled (by remote id)");
                }
                Err(err) => match err.0 {
                    BinanceErrorKind::BinanceError(response) => match response.code {
                        -1000_i16 => error!("An unknown error occured while processing the request"),
                        _ => error!("Binance error: code={}: {}", response.code, response.msg),
                    },
                    BinanceErrorKind::Msg(msg) => error!("Binancelib error msg: {}", msg),
                    _ => error!("Other errors: {}.", err.0),
                },
            });
         */

        Ok(())
    }

    pub fn cancel_order_by_local_id(&mut self, symbol: Ustr, local_id: OrderId) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(0, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool
            .execute(move || match account.cancel_order_by_internal_id(symbol.as_str(), itoa_u64(local_id)) {
                Ok(cancelled_order) => {
                    // println!("CANCELED ORDER: {:#?}", cancelled_order);

                    let cancelled_order_id = match cancelled_order.client_order_id {
                        None => {
                            error!("failed to cancel order (regular): no client_order_id sent back for internal_id={}", local_id);
                            trader
                                .confirm_order_cancel_failed(symbol, local_id)
                                .expect("failed to confirm order cancel failed (by local id) (no client_order_id)");
                        }

                        Some(local_id_string) => {
                            // Parse the string order id as a u64
                            let local_id = match s2u64(local_id_string.as_str()) {
                                Ok(local_id) => local_id,
                                Err(e) => {
                                    error!("failed to parse client_order_id={} as u64: {}", local_id_string, e);
                                    trader
                                        .confirm_order_cancel_failed(symbol, local_id)
                                        .expect("failed to confirm order cancel failed (by local id) (parse error)");
                                    return;
                                }
                            };

                            trader
                                .confirm_order_cancelled(symbol, local_id)
                                .expect("failed to confirm order cancelled (by local id)");
                        }
                    };

                    ()
                }
                Err(err) => match err.0 {
                    BinanceErrorKind::BinanceError(response) => match response.code {
                        -1000_i16 => error!("An unknown error occured while processing the request"),
                        _ => error!("Binance error: code={}: {}", response.code, response.msg),
                    },
                    BinanceErrorKind::Msg(msg) => error!("Binancelib error msg: {}", msg),
                    _ => error!("Other errors: {}.", err.0),
                },
            });
         */

        Ok(())
    }

    pub fn cancel_all(&mut self, symbol: Ustr, recv_window: u64) -> Result<(), ExchangeClientError> {
        self.wait_for_ratelimiter(1, 1);

        /*
        let account = self.exchange_api.account.clone();
        let mut trader = self.trader.clone();

        self.threadpool.execute(move || match account.cancel_all_open_orders(symbol.as_str()) {
            Ok(result) => {
                () // TODO: make sure that all affected orders are properly handled locally
            }
            Err(err) => match err.0 {
                BinanceErrorKind::BinanceError(response) => match response.code {
                    -1000_i16 => error!("An unknown error occured while processing the request"),
                    _ => error!("Binance error: code={}: {}", response.code, response.msg),
                },
                BinanceErrorKind::Msg(msg) => error!("Binancelib error msg: {}", msg),
                _ => error!("Other errors: {}.", err.0),
            },
        });
         */

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{exchange_client::ExchangeClientJob, instruction::InstructionMetadata, types::Priority};
    use std::{collections::BinaryHeap, time::Instant};
    use uuid::Uuid;

    #[test]
    fn test_trader() {
        let mut rng = rand::thread_rng();

        let it = Instant::now();
        let x = 9306126341824671624u64;
        let s = x.to_string();
        println!("{:#?}", it.elapsed());

        let it = Instant::now();
        let s = Uuid::new_v4().to_string();
        println!("{:#?}", it.elapsed());
    }

    #[test]
    fn test_job_priority() {
        let jobs = vec![
            ExchangeClientJob {
                instruction_id: 1,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Elevated,
            },
            ExchangeClientJob {
                instruction_id: 2,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Immediate,
            },
            ExchangeClientJob {
                instruction_id: 3,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Relaxed,
            },
            ExchangeClientJob {
                instruction_id: 4,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Normal,
            },
            ExchangeClientJob {
                instruction_id: 5,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Normal,
            },
            ExchangeClientJob {
                instruction_id: 6,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::High,
            },
            ExchangeClientJob {
                instruction_id: 7,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Relaxed,
            },
            ExchangeClientJob {
                instruction_id: 8,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Immediate,
            },
            ExchangeClientJob {
                instruction_id: 9,
                metadata:       InstructionMetadata::default(),
                instruction:    Default::default(),
                priority:       Priority::Urgent,
            },
        ];

        println!("{:#?}", jobs);

        let mut queue = jobs.into_iter().collect::<BinaryHeap<_>>();
        println!("{:#?}", queue);

        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
        println!("{:?}", queue.pop());
    }
}
