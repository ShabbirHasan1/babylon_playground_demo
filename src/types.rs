use binance_spot_connector_rust::{http::Credentials, market::klines::Klines};
use std::{
    borrow::Cow,
    fmt,
    fmt::{Debug, Display, Formatter},
    ops::{Deref, DerefMut},
};

use chrono::{DateTime, NaiveDateTime, Utc};
use ordered_float::OrderedFloat;
use thiserror::Error;
use ustr::{ustr, Ustr};
use yata::core::ValueType;

use crate::{
    candle::Candle,
    candle_manager::CandleManagerError,
    f,
    id::{CoinId, CoinPairId, OrderId},
    model,
    order::{Order, OrderFill, OrderFillMetadata, OrderFillStatus},
    timeframe::Timeframe,
    trade::{Trade, TradeType},
    util::{hash_id_u32, s2u64},
};

use crate::{
    model::{ExchangeTradeEvent, ListenKey},
    timeframe::TimeframeError,
    util::parse_candle_data_array,
};
use binance_spot_connector_rust::{
    http::{
        error::{ClientError, HttpError as BinanceHttpError},
        request::Request,
    },
    hyper::Error as HyperError,
};
use http::{uri::InvalidUri, Error as HttpError};
use itertools::Itertools;
use log::{error, info};
use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Error)]
pub enum ExchangeApiError {
    #[error("Client error: {0:?}")]
    ClientError(ClientError),

    #[error("Server error: {0:?}")]
    ServerError(BinanceHttpError<String>),

    #[error("Invalid API secret")]
    InvalidApiSecret,

    #[error("Parse error: {0:?}")]
    ParseError(HttpError),

    #[error("Send error: {0:?}")]
    SendError(HyperError),

    #[error("Unknown error")]
    UnknownError,

    #[error("Timeframe error: {0:?}")]
    TimeframeError(TimeframeError),

    #[error("Other error: {0:#?}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

pub type OrderedValueType = OrderedFloat<ValueType>;

/// Represents a pair of coins, each coin is identified by its id (u32)
/// derived from its symbol (e.g. BTC, ETH, etc.)
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct CoinPair {
    pub pair_id:  CoinPairId,
    pub base_id:  CoinId,
    pub quote_id: CoinId,
}

impl CoinPair {
    pub fn from_str(base_symbol: &str, quote_symbol: &str) -> Self {
        let base_id = hash_id_u32(base_symbol);
        let quote_id = hash_id_u32(quote_symbol);
        CoinPair {
            pair_id: ((base_id as CoinPairId) << 32) | (quote_id as CoinPairId),
            base_id,
            quote_id,
        }
    }

    pub fn from_ids(base_id: CoinId, quote_id: CoinId) -> Self {
        CoinPair {
            pair_id: ((base_id as CoinPairId) << 32) | (quote_id as CoinPairId),
            base_id,
            quote_id,
        }
    }
}

#[derive(Clone)]
pub struct ExchangeApi {
    pub client:             binance_spot_connector_rust::ureq::BinanceHttpClient,
    pub(crate) credentials: Option<Credentials>,
    listen_key:             Option<String>,
}

impl ExchangeApi {
    pub fn new(key: Option<String>, secret: Option<String>) -> Self {
        let mut client = binance_spot_connector_rust::ureq::BinanceHttpClient::default();
        let mut credentials = None;

        if let (Some(key), Some(secret)) = (key.clone(), secret.clone()) {
            credentials = Some(Credentials::from_hmac(key, secret));
        }

        if let Some(credentials) = &credentials {
            client = client.credentials(credentials.clone());
        }

        Self {
            client,
            credentials,
            listen_key: None,
        }
    }

    pub fn start_user_stream(&mut self) -> Result<String, ExchangeApiError> {
        let response = self
            .client
            .send(binance_spot_connector_rust::stream::new_listen_key())
            .map_err(|e| self.handle_error(*e))?;

        let listen_key: ListenKey =
            serde_json::from_str(response.into_body_str().expect("into_body_str() failed").as_str()).expect("Failed to parse listen key");

        self.listen_key = Some(listen_key.listen_key.clone());

        info!("Started user stream: {}", listen_key.listen_key);

        Ok(listen_key.listen_key)
    }

    pub fn keepalive_user_stream(&self) -> Result<(), ExchangeApiError> {
        if let Some(listen_key) = &self.listen_key {
            self.client
                .send(binance_spot_connector_rust::stream::renew_listen_key(listen_key))
                .map_err(|e| self.handle_error(*e))?;
        }

        info!("Kept user stream alive");

        Ok(())
    }

    pub fn handle_error(&self, error: binance_spot_connector_rust::ureq::Error) -> ExchangeApiError {
        eprintln!("error = {:#?}", error);

        match error {
            binance_spot_connector_rust::ureq::Error::Client(client_err) => {
                eprintln!("client_err = {:#?}", client_err);
                ExchangeApiError::ClientError(client_err)
            }
            binance_spot_connector_rust::ureq::Error::Server(server_err) => {
                eprintln!("server_err = {:#?}", server_err);
                ExchangeApiError::ServerError(server_err)
            }
            binance_spot_connector_rust::ureq::Error::InvalidApiSecret => {
                eprintln!("InvalidApiSecret");
                ExchangeApiError::InvalidApiSecret
            }

            err => {
                error!("Unhandled error: {:#?}", err);
                ExchangeApiError::UnknownError
            }
        }
    }

    pub fn fetch_account(&self) -> Result<model::AccountInformation, ExchangeApiError> {
        let response = self
            .client
            .send(binance_spot_connector_rust::trade::account())
            .map_err(|e| self.handle_error(*e))?;

        let account: model::AccountInformation =
            serde_json::from_str(response.into_body_str().expect("into_body_str() failed").as_str()).expect("Failed to parse account");

        Ok(account)
    }

    pub fn fetch_exchange_information(&self) -> Result<model::ExchangeInformation, ExchangeApiError> {
        let response = self
            .client
            .send(binance_spot_connector_rust::market::exchange_info())
            .map_err(|e| self.handle_error(*e))?;

        let exchange_info: model::ExchangeInformation =
            serde_json::from_str(response.into_body_str().expect("into_body_str() failed").as_str()).expect("Failed to parse exchange info");

        Ok(exchange_info)
    }

    pub fn fetch_klines(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
        limit: Option<usize>,
    ) -> Result<Vec<Candle>, ExchangeApiError> {
        let start_timestamp = start_time.map(|t| t.timestamp_millis() as u64);
        let end_timestamp = end_time.map(|t| t.timestamp_millis() as u64).unwrap_or(Utc::now().timestamp_millis() as u64);
        let limit = limit.unwrap_or(1000);
        let kline_interval = timeframe.try_into().expect("Invalid timeframe");
        let response = self
            .client
            .send(
                Klines::new(symbol, kline_interval)
                    .limit(limit as u32)
                    .start_time(start_timestamp.unwrap())
                    .end_time(end_timestamp),
            )
            .map_err(|e| self.handle_error(*e))?;
        let response = response.into_body_str().expect("into_body_str() failed");
        let candles = parse_candle_data_array(ustr(symbol), timeframe, response.as_str())
            .into_iter()
            .sorted_by(|a, b| a.open_time.cmp(&b.open_time))
            .collect_vec();

        Ok(candles)
    }

    pub fn fetch_depth(&self, symbol: &str, limit: Option<u32>) -> Result<model::Depth, ExchangeApiError> {
        let response = self
            .client
            .send(binance_spot_connector_rust::market::depth(symbol).limit(limit.unwrap_or(100)))
            .expect("send() failed");
        let depth: model::Depth = serde_json::from_str(response.into_body_str().expect("into_body_str() failed").as_str()).expect("Failed to parse depth");
        Ok(depth)
    }
}

impl Default for ExchangeApi {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl Deref for ExchangeApi {
    type Target = binance_spot_connector_rust::ureq::BinanceHttpClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl DerefMut for ExchangeApi {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

#[derive(Debug)]
pub enum DataPoint {
    Candle(Candle),
    ExchangeTrade(ExchangeTradeEvent),
    OrderClosed(Order),
    TradeClosed(Trade),
}

#[derive(Copy, Clone, Debug, Default)]
pub enum Amount {
    #[default]
    Zero,
    Exact(OrderedValueType),
    RatioOfAvailable(OrderedValueType),
    RatioOfTotal(OrderedValueType),
    AllAvailable,
}

#[derive(Copy, Clone, Debug)]
pub enum Quantity {
    Quote(Amount),
    Base(Amount),
}

impl Quantity {
    pub fn is_zero(&self) -> bool {
        match self {
            Quantity::Quote(Amount::Zero) => true,
            Quantity::Base(Amount::Zero) => true,
            _ => false,
        }
    }

    pub fn is_all_available(&self) -> bool {
        match self {
            Quantity::Quote(Amount::AllAvailable) => true,
            Quantity::Base(Amount::AllAvailable) => true,
            _ => false,
        }
    }

    pub fn is_exact(&self) -> bool {
        match self {
            Quantity::Quote(Amount::Exact(_)) => true,
            Quantity::Base(Amount::Exact(_)) => true,
            _ => false,
        }
    }

    pub fn is_ratio_of_available(&self) -> bool {
        match self {
            Quantity::Quote(Amount::RatioOfAvailable(_)) => true,
            Quantity::Base(Amount::RatioOfAvailable(_)) => true,
            _ => false,
        }
    }

    pub fn is_ratio_of_total(&self) -> bool {
        match self {
            Quantity::Quote(Amount::RatioOfTotal(_)) => true,
            Quantity::Base(Amount::RatioOfTotal(_)) => true,
            _ => false,
        }
    }

    pub fn is_quote(&self) -> bool {
        matches!(self, Quantity::Quote(_))
    }

    pub fn is_base(&self) -> bool {
        matches!(self, Quantity::Base(_))
    }
}

impl Default for Quantity {
    fn default() -> Self {
        Self::Base(Amount::Zero)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub enum OrderLifecycleAction {
    #[default]
    CancelAndMarketExit,
    Cancel,
    CancelAndResumeStoplossOrMarketExit,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OrderLifecycle {
    /// This is an action that is executed when the order is activated.
    pub on_activation: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is deactivated.
    pub on_deactivation: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is opened.
    pub on_open: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is closed.
    pub on_close: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is filled partially or fully.
    pub on_partial_fill: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is filled fully.
    pub on_fill: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is canceled.
    pub on_cancel: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is rejected upon opening.
    pub on_reject: Option<OrderLifecycleAction>,

    /// This is an action that is executed when the order is timed out.
    pub on_timeout: Option<OrderLifecycleAction>,
}

#[derive(Debug, Copy, Clone)]
pub enum UiConfirmationResponse {
    NoResponse,
    Confirmed,
    Cancelled,
}

#[derive(Debug, Default, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, strum_macros::EnumString)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(u8)]
pub enum Status {
    #[default]
    NotInitialized,
    Initialized,
    Interrupted,
    Failed,
    Rejected,
    #[serde(alias = "CANCELED")]
    Cancelled,
    Ok,
    NotAttempted,
    Abandoned,
    Unknown,
}

impl Display for Status {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Status::NotInitialized => "NOT_INITIALIZED",
                Status::Initialized => "INITIALIZED",
                Status::Interrupted => "INTERRUPTED",
                Status::Failed => "FAILED",
                Status::Cancelled => "CANCELED",
                Status::Ok => "SUCCESS",
                Status::NotAttempted => "NOT_ATTEMPTED",
                Status::Unknown => "UNKNOWN",
                Status::Abandoned => "ABANDONED",
                Status::Rejected => "REJECTED",
            }
        )
    }
}

#[derive(Debug, Copy, Clone, PartialOrd, PartialEq, Default)]
#[repr(i8)]
pub enum IndicationStatus {
    VeryOverbought = 3,
    Overbought = 2,
    AlmostOverbought = 1,
    #[default]
    Parity = 0,
    AlmostOversold = -1,
    Oversold = -2,
    VeryOversold = -3,
}

#[derive(Debug, Copy, Clone, PartialOrd, PartialEq, Default)]
#[repr(i8)]
pub enum Direction {
    Rising = 2,
    MostlyRising = 1,
    #[default]
    Choppy = 0,
    MostlyFalling = -1,
    Falling = -2,
}

#[derive(Debug, Copy, Clone, PartialOrd, PartialEq, Default)]
#[repr(i8)]
pub enum Motion {
    Accelerating = 1,
    #[default]
    Normal = 0,
    Decelerating = -1,
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Default)]
#[repr(u8)]
pub enum Priority {
    Relaxed = 0,
    #[default]
    Normal = 1,
    Elevated = 2,
    High = 3,
    Urgent = 4,
    Immediate = 5,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub enum PriceDistribution {
    Arithmetic,
    #[default]
    Geometric,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum QuantityDistribution {
    Equal,
    Progressive,
    Degressive,
}

#[derive(Copy, Clone, Debug, PartialOrd, PartialEq)]
#[repr(i8)]
pub enum Gauge {
    BreakoutUp = 5,
    Crest = 4,
    VeryHigh = 3,
    High = 2,
    AbovePar = 1,
    Parity = 0,
    BelowPar = -1,
    Low = -2,
    VeryLow = -3,
    Trough = -4,
    BreakoutDown = -5,
}

impl Display for Gauge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Gauge::BreakoutUp => "BREAKOUT_UP",
            Gauge::Crest => "CREST",
            Gauge::VeryHigh => "VERY_HIGH",
            Gauge::High => "HIGH",
            Gauge::AbovePar => "ABOVE_PAR",
            Gauge::Parity => "PARITY",
            Gauge::BelowPar => "BELOW_PAR",
            Gauge::Low => "LOW",
            Gauge::VeryLow => "VERY_LOW",
            Gauge::Trough => "TROUGH",
            Gauge::BreakoutDown => "BREAKOUT_DOWN",
        };
        write!(f, "{}", name)
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::Parity
    }
}

#[derive(Debug, Copy, Clone, PartialOrd, PartialEq, Default)]
pub struct NormalizedCommission {
    pub maker:  Option<OrderedValueType>,
    pub taker:  Option<OrderedValueType>,
    pub buyer:  Option<OrderedValueType>,
    pub seller: Option<OrderedValueType>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, Deserialize, Serialize, strum_macros::EnumString)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NormalizedOrderType {
    #[strum(serialize = "LIMIT")]
    Limit,
    #[default]
    #[strum(serialize = "LIMIT_MAKER")]
    LimitMaker,
    #[strum(serialize = "MARKET")]
    Market,
    #[strum(serialize = "TAKE_PROFIT")]
    TakeProfit,
    #[strum(serialize = "TAKE_PROFIT_LIMIT")]
    TakeProfitLimit,
    #[strum(serialize = "STOP_LOSS")]
    StopLoss,
    #[strum(serialize = "STOP_LOSS_LIMIT")]
    StopLossLimit,
}

impl NormalizedOrderType {
    pub fn is_limit(&self) -> bool {
        matches!(self, Self::Limit | Self::LimitMaker)
    }

    pub fn is_limit_or_stop_limit(&self) -> bool {
        matches!(self, Self::Limit | Self::LimitMaker | Self::StopLossLimit)
    }

    pub fn is_limit_maker(&self) -> bool {
        matches!(self, Self::LimitMaker)
    }

    pub fn is_market(&self) -> bool {
        matches!(self, Self::Market)
    }

    pub fn is_stoploss(&self) -> bool {
        matches!(self, Self::StopLoss)
    }

    pub fn is_stoploss_limit(&self) -> bool {
        matches!(self, Self::StopLossLimit)
    }

    pub fn is_taker(&self) -> bool {
        matches!(self, Self::Market | Self::StopLoss | Self::StopLossLimit)
    }
}

impl AsRef<str> for NormalizedOrderType {
    fn as_ref(&self) -> &str {
        match self {
            NormalizedOrderType::Limit => "LIMIT",
            NormalizedOrderType::LimitMaker => "LIMIT_MAKER",
            NormalizedOrderType::Market => "MARKET",
            NormalizedOrderType::TakeProfit => "TAKE_PROFIT",
            NormalizedOrderType::TakeProfitLimit => "TAKE_PROFIT_LIMIT",
            NormalizedOrderType::StopLoss => "STOP_LOSS",
            NormalizedOrderType::StopLossLimit => "STOP_LOSS_LIMIT",
        }
    }
}

impl Display for NormalizedOrderType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NormalizedOrderType::Limit => write!(f, "LIMIT"),
            NormalizedOrderType::LimitMaker => write!(f, "LIMIT_MAKER"),
            NormalizedOrderType::Market => write!(f, "MARKET"),
            NormalizedOrderType::TakeProfit => write!(f, "TAKE_PROFIT"),
            NormalizedOrderType::TakeProfitLimit => write!(f, "TAKE_PROFIT_LIMIT"),
            NormalizedOrderType::StopLoss => write!(f, "STOP_LOSS"),
            NormalizedOrderType::StopLossLimit => write!(f, "STOP_LOSS_LIMIT"),
        }
    }
}

impl From<String> for NormalizedOrderType {
    fn from(item: String) -> Self {
        match item.as_str() {
            "LIMIT" => NormalizedOrderType::Limit,
            "LIMIT_MAKER" => NormalizedOrderType::LimitMaker,
            "MARKET" => NormalizedOrderType::Market,
            "TAKE_PROFIT" => NormalizedOrderType::TakeProfit,
            "TAKE_PROFIT_LIMIT" => NormalizedOrderType::TakeProfitLimit,
            "STOP_LOSS" => NormalizedOrderType::StopLoss,
            "STOP_LOSS_LIMIT" => NormalizedOrderType::StopLossLimit,
            _ => panic!("unknown order type: {}", item),
        }
    }
}

impl From<NormalizedOrderType> for String {
    fn from(item: NormalizedOrderType) -> Self {
        match item {
            NormalizedOrderType::Limit => String::from("LIMIT"),
            NormalizedOrderType::LimitMaker => String::from("LIMIT_MAKER"),
            NormalizedOrderType::Market => String::from("MARKET"),
            NormalizedOrderType::TakeProfit => String::from("TAKE_PROFIT"),
            NormalizedOrderType::TakeProfitLimit => String::from("TAKE_PROFIT_LIMIT"),
            NormalizedOrderType::StopLoss => String::from("STOP_MARKET"),
            NormalizedOrderType::StopLossLimit => String::from("STOP_LIMIT"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, strum_macros::EnumString)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NormalizedSide {
    Buy,
    Sell,
}

impl Default for NormalizedSide {
    fn default() -> Self {
        Self::Buy
    }
}

impl Display for NormalizedSide {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            NormalizedSide::Buy => write!(f, "BUY"),
            NormalizedSide::Sell => write!(f, "SELL"),
        }
    }
}

impl From<TradeType> for NormalizedSide {
    fn from(item: TradeType) -> Self {
        match item {
            TradeType::Long => NormalizedSide::Buy,
            TradeType::Short => NormalizedSide::Sell,
        }
    }
}

impl From<String> for NormalizedSide {
    fn from(item: String) -> Self {
        match item.as_str() {
            "BUY" => NormalizedSide::Buy,
            "SELL" => NormalizedSide::Sell,
            _ => panic!("unknown Side: {}", item),
        }
    }
}

impl From<NormalizedSide> for String {
    fn from(item: NormalizedSide) -> Self {
        match item {
            NormalizedSide::Buy => String::from("BUY"),
            NormalizedSide::Sell => String::from("SELL"),
        }
    }
}

impl NormalizedSide {
    pub fn opposite(&self) -> Self {
        match self {
            NormalizedSide::Buy => NormalizedSide::Sell,
            NormalizedSide::Sell => NormalizedSide::Buy,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Default, Deserialize, Serialize)]
pub enum NormalizedTimeInForce {
    #[default]
    GTC,
    IOC,
    FOK,
    GTX,
}

impl From<String> for NormalizedTimeInForce {
    fn from(item: String) -> Self {
        match item.as_str() {
            "GTC" => NormalizedTimeInForce::GTC,
            "IOC" => NormalizedTimeInForce::IOC,
            "FOK" => NormalizedTimeInForce::FOK,
            "GTX" => NormalizedTimeInForce::GTX,
            _ => panic!("unknown TimeInForce: {}", item),
        }
    }
}

impl From<NormalizedTimeInForce> for String {
    fn from(item: NormalizedTimeInForce) -> Self {
        match item {
            NormalizedTimeInForce::GTC => String::from("GTC"),
            NormalizedTimeInForce::IOC => String::from("IOC"),
            NormalizedTimeInForce::FOK => String::from("FOK"),
            NormalizedTimeInForce::GTX => String::from("GTX"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NormalizedExecutionType {
    New,
    #[serde(rename = "CANCELED")]
    Cancelled,
    Replaced,
    Rejected,
    Trade,
    Expired,
    TradePrevention,
    Unknown,
}

impl From<String> for NormalizedExecutionType {
    fn from(status: String) -> Self {
        match status.as_str() {
            "NEW" => NormalizedExecutionType::New,
            "CANCELED" => NormalizedExecutionType::Cancelled,
            "REJECTED" => NormalizedExecutionType::Rejected,
            "REPLACED" => NormalizedExecutionType::Replaced,
            "TRADE" => NormalizedExecutionType::Trade,
            "EXPIRED" => NormalizedExecutionType::Expired,
            "TRADE_PREVENTION" => NormalizedExecutionType::TradePrevention,
            _ => panic!("unrecognized execution type: {}", status),
        }
    }
}

impl Into<String> for NormalizedExecutionType {
    fn into(self) -> String {
        match self {
            NormalizedExecutionType::New => "NEW",
            NormalizedExecutionType::Cancelled => "CANCELED",
            NormalizedExecutionType::Replaced => "REPLACED",
            NormalizedExecutionType::Rejected => "REJECTED",
            NormalizedExecutionType::Expired => "EXPIRED",
            NormalizedExecutionType::Trade => "TRADE",
            NormalizedExecutionType::TradePrevention => "TRADE_PREVENTION",
            NormalizedExecutionType::Unknown => "UNKNOWN",
        }
        .to_string()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, strum_macros::EnumString)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NormalizedOrderStatus {
    New,
    PartiallyFilled,
    Filled,
    #[serde(alias = "CANCELED")]
    Cancelled,
    PendingCancel,
    Expired,
    Rejected,
    Replaced,
}

impl From<String> for NormalizedOrderStatus {
    fn from(status: String) -> Self {
        match status.as_str() {
            "NEW" => NormalizedOrderStatus::New,
            "PARTIALLY_FILLED" => NormalizedOrderStatus::PartiallyFilled,
            "FILLED" => NormalizedOrderStatus::Filled,
            "PENDING_CANCEL" => NormalizedOrderStatus::PendingCancel,
            "CANCELED" => NormalizedOrderStatus::Cancelled,
            "EXPIRED" => NormalizedOrderStatus::Expired,
            "REJECTED" => NormalizedOrderStatus::Rejected,
            "REPLACED" => NormalizedOrderStatus::Replaced,
            _ => panic!("unrecognized order status: {}", status),
        }
    }
}

impl Into<String> for NormalizedOrderStatus {
    fn into(self) -> String {
        match self {
            NormalizedOrderStatus::New => "NEW",
            NormalizedOrderStatus::PartiallyFilled => "PARTIALLY_FILLED",
            NormalizedOrderStatus::Filled => "FILLED",
            NormalizedOrderStatus::PendingCancel => "PENDING_CANCEL",
            NormalizedOrderStatus::Cancelled => "CANCELED",
            NormalizedOrderStatus::Expired => "EXPIRED",
            NormalizedOrderStatus::Rejected => "REJECTED",
            NormalizedOrderStatus::Replaced => "REPLACED",
        }
        .to_string()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Lot {
    pub quantity:      ValueType,
    pub price:         ValueType,
    pub trigger_price: Option<ValueType>,
}

#[derive(Clone, Copy, Debug)]
pub enum NormalizedOrderEventSource {
    UserTradeEvent,
    Transaction,
}

#[derive(Clone)]
pub struct NormalizedOrderEvent {
    pub symbol:                                     Ustr,
    pub exchange_order_id:                          u64,
    pub local_order_id:                             OrderId,
    pub trade_id:                                   i64,
    pub time_in_force:                              NormalizedTimeInForce,
    pub side:                                       NormalizedSide,
    pub order_type:                                 NormalizedOrderType,
    pub current_execution_type:                     NormalizedExecutionType,
    pub current_order_status:                       NormalizedOrderStatus,
    pub order_price:                                OrderedValueType,
    pub quantity:                                   OrderedValueType,
    pub avg_filled_price:                           OrderedValueType,
    pub last_filled_price:                          OrderedValueType,
    pub last_executed_quantity:                     OrderedValueType,
    pub last_quote_asset_transacted_quantity:       OrderedValueType,
    pub cumulative_filled_quantity:                 OrderedValueType,
    pub cumulative_quote_asset_transacted_quantity: OrderedValueType,
    pub quote_order_quantity:                       OrderedValueType, // TODO: check if this is correct, possibly the naming is wrong in the binance crate
    pub commission:                                 OrderedValueType,
    pub commission_asset:                           Option<Ustr>,
    pub latest_fill:                                Option<OrderFill>,
    pub is_trade_the_maker_side:                    bool,
    pub order_reject_reason:                        String,
    pub source:                                     NormalizedOrderEventSource,
    pub transaction_time:                           DateTime<Utc>,
    pub event_time:                                 DateTime<Utc>,
}

impl AsRef<NormalizedOrderEvent> for NormalizedOrderEvent {
    fn as_ref(&self) -> &NormalizedOrderEvent {
        self
    }
}

impl Debug for NormalizedOrderEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("NormalizedOrderEvent")
            .field("symbol", &self.symbol)
            .field("exchange_order_id", &self.exchange_order_id)
            .field("local_order_id", &self.local_order_id)
            .field("trade_id", &self.trade_id)
            .field("time_in_force", &self.time_in_force)
            .field("side", &self.side)
            .field("order_type", &self.order_type)
            .field("execution_type", &self.current_execution_type)
            .field("status", &self.current_order_status)
            .field("price", &self.order_price.0)
            .field("quantity", &self.quantity.0)
            .field("avg_filled_price", &self.avg_filled_price.0)
            .field("last_filled_price", &self.last_filled_price.0)
            .field("last_filled_quantity", &self.last_executed_quantity.0)
            .field("last_filled_quote", &self.last_quote_asset_transacted_quantity.0)
            .field("accumulated_filled_quantity", &self.cumulative_filled_quantity.0)
            .field("accumulated_filled_quote", &self.cumulative_quote_asset_transacted_quantity.0)
            .field("quote_order_quantity", &self.quote_order_quantity.0)
            .field("commission", &self.commission.0)
            .field("commission_asset", &self.commission_asset)
            .field("latest_fill", &self.latest_fill)
            .field("is_buyer_maker", &self.is_trade_the_maker_side)
            .field("order_reject_reason", &self.order_reject_reason)
            .field("trade_ts", &self.transaction_time)
            .field("event_ts", &self.event_time)
            .finish()
    }
}

/*
impl From<Transaction> for NormalizedOrderEvent {
    fn from(t: Transaction) -> Self {
        let mut trade_id = 0;
        let mut commission = f!(0.0);
        let mut commission_asset = None;
        let mut status = NormalizedOrderStatus::New;
        let mut accumulated_filled_quantity = f!(0.0);
        let mut accumulated_filled_quote = f!(0.0);
        let mut event_ts = NaiveDateTime::from_timestamp_opt(t.transact_time as i64 / 1000, 0)
            .expect("invalid timestamp")
            .and_utc();

        let fills = match t.fills {
            None => vec![],
            Some(fills) => {
                let len = fills.len();

                fills
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        trade_id = f.trade_id.unwrap_or_default();
                        commission = f!(f.commission);
                        commission_asset = Some(ustr(f.commission_asset.as_str()));
                        status = if i == len - 1 { NormalizedOrderStatus::Filled } else { NormalizedOrderStatus::PartiallyFilled };
                        let fill_status = if i == len - 1 { OrderFillStatus::Filled } else { OrderFillStatus::PartiallyFilled };
                        accumulated_filled_quantity += f!(f.qty);
                        accumulated_filled_quote += f!(f.qty) * f!(f.price);

                        OrderFill {
                            status:   fill_status,
                            metadata: OrderFillMetadata {
                                price:                       f!(f.price),
                                quantity:                    f!(f.qty),
                                quote_equivalent:            f!(f.qty) * f!(f.price),
                                accumulated_filled_quantity: accumulated_filled_quantity,
                                accumulated_filled_quote:    accumulated_filled_quote,
                                timestamp:                   event_ts.clone(),
                            },
                        }
                    })
                    .collect::<Vec<_>>()
            }
        };

        // WARNING: This is a crude attempt to adapt the binance transaction to the normalized order event.
        Self {
            symbol:                                     ustr(t.symbol.as_str()),
            exchange_order_id:                          t.order_id,
            local_order_id:                             s2u64(t.client_order_id.as_str()).expect("invalid client order id"),
            trade_id:                                   0,
            time_in_force:                              NormalizedTimeInForce::from(t.time_in_force),
            side:                                       NormalizedSide::from(t.side),
            order_type:                                 NormalizedOrderType::from(t.type_name),
            current_execution_type:                     NormalizedExecutionType::Unknown,
            current_order_status:                       NormalizedOrderStatus::from(t.status),
            order_price:                                f!(t.price),
            quantity:                                   f!(t.executed_qty),
            avg_filled_price:                           f!(t.price),
            last_filled_price:                          f!(t.price),
            last_executed_quantity:                     f!(t.executed_qty),
            last_quote_asset_transacted_quantity:       f!(t.cummulative_quote_qty),
            cumulative_filled_quantity:                 accumulated_filled_quantity,
            cumulative_quote_asset_transacted_quantity: accumulated_filled_quote,
            quote_order_quantity:                       f!(t.cummulative_quote_qty),
            commission:                                 f!(0.0),
            commission_asset:                           None,
            latest_fill:                                fills.last().cloned(),
            is_trade_the_maker_side:                    false,
            order_reject_reason:                        "".to_string(),
            source:                                     NormalizedOrderEventSource::Transaction,
            transaction_time:                           t.transact_time,
            event_time:                                 0,
            event_type:                                 ustr("TRADE"),
        }
    }
}
 */

#[derive(Debug, Clone, Default)]
pub struct NormalizedBalanceEvent {
    pub balance:                  Vec<(Ustr, OrderedValueType, OrderedValueType)>, // (asset, free, locked)
    pub event_time:               DateTime<Utc>,
    pub last_account_update_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    Local,
    Exchange,
}
