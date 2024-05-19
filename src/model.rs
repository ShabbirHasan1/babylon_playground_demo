use crate::{
    candle::Candle,
    f,
    id::OrderId,
    types::{NormalizedExecutionType, NormalizedOrderStatus, NormalizedOrderType, NormalizedSide, NormalizedTimeInForce, OrderedValueType},
    util::{s2f, s2of},
};
use chrono::{DateTime, Utc};
use ordered_float::OrderedFloat;
use serde::{
    de,
    de::{MapAccess, SeqAccess, Visitor},
    Deserializer,
};
use serde_derive::{Deserialize, Serialize};
use serde_with::serde_as;
use std::fmt;
use ustr::Ustr;
use yata::core::ValueType;

// ----------------------------------------------------------------------------
// A custom deserializer for the f64 array in the PartialDepth struct
// ----------------------------------------------------------------------------

struct F64ArrayVisitor;

impl<'de> Visitor<'de> for F64ArrayVisitor {
    type Value = [[OrderedValueType; 2]; 20];

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of 20 arrays of strings")
    }

    fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
    where
        S: SeqAccess<'de>,
    {
        let mut arr = [[OrderedValueType::default(); 2]; 20];

        for i in 0..20 {
            let value: [String; 2] = seq.next_element()?.ok_or(de::Error::invalid_length(i, &self))?;
            arr[i][0] = s2of(&value[0]);
            arr[i][1] = s2of(&value[1]);
        }

        Ok(arr)
    }
}

fn from_str_to_f64_array<'de, D>(deserializer: D) -> Result<[[OrderedValueType; 2]; 20], D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(F64ArrayVisitor)
}

// ----------------------------------------------------------------------------
// A custom deserializer for the f64 array in the DepthUpdate struct
// ----------------------------------------------------------------------------

struct F64VecArrayVisitor;

impl<'de> Visitor<'de> for F64VecArrayVisitor {
    type Value = Vec<[OrderedValueType; 2]>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of arrays of strings")
    }

    fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
    where
        S: SeqAccess<'de>,
    {
        let mut vec = Vec::new();

        while let Some(value) = seq.next_element()? {
            let arr: [String; 2] = value;
            let a = s2of(&arr[0]);
            let b = s2of(&arr[1]);
            vec.push([a, b]);
        }

        Ok(vec)
    }
}

fn from_str_to_f64_vec_array<'de, D>(deserializer: D) -> Result<Vec<[OrderedValueType; 2]>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(F64VecArrayVisitor)
}

// ----------------------------------------------------------------------------
// A custom deserializer for the OrderedValueType
// ----------------------------------------------------------------------------

struct OrderedValueTypeVisitor;

impl<'de> Visitor<'de> for OrderedValueTypeVisitor {
    type Value = OrderedValueType;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an OrderedValueType represented as a string")
    }

    fn visit_str<E>(self, value: &str) -> Result<OrderedValueType, E>
    where
        E: de::Error,
    {
        Ok(s2of(value))
    }
}

fn from_str_to_ordered_value_type<'de, D>(deserializer: D) -> Result<OrderedValueType, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_str(OrderedValueTypeVisitor)
}

// Same as above, but for Option<OrderedValueType>

struct OrderedValueTypeOptionalVisitor;

impl<'de> Visitor<'de> for OrderedValueTypeOptionalVisitor {
    type Value = Option<OrderedValueType>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an optional OrderedValueType represented as a string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Option<OrderedValueType>, E>
    where
        E: de::Error,
    {
        if value.is_empty() {
            Ok(Some(OrderedValueType::default()))
        } else {
            Ok(Some(s2of(value)))
        }
    }
}

fn from_str_to_ordered_value_type_option<'de, D>(deserializer: D) -> Result<OrderedValueType, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_str(OrderedValueTypeVisitor)
}

// ----------------------------------------------------------------------------
// A custom deserializer to handle the array of strings and floats representing a Candle
// ----------------------------------------------------------------------------

struct CandleVisitor;

impl<'de> Visitor<'de> for CandleVisitor {
    type Value = Candle;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of strings and floats representing a Candle")
    }

    fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
    where
        S: SeqAccess<'de>,
    {
        let open_time: DateTime<Utc> = seq.next_element()?.ok_or(de::Error::invalid_length(0, &self))?;
        let open: String = seq.next_element()?.ok_or(de::Error::invalid_length(1, &self))?;
        let high: String = seq.next_element()?.ok_or(de::Error::invalid_length(2, &self))?;
        let low: String = seq.next_element()?.ok_or(de::Error::invalid_length(3, &self))?;
        let close: String = seq.next_element()?.ok_or(de::Error::invalid_length(4, &self))?;
        let volume: String = seq.next_element()?.ok_or(de::Error::invalid_length(5, &self))?;
        let close_time: DateTime<Utc> = seq.next_element()?.ok_or(de::Error::invalid_length(6, &self))?;
        let quote_asset_volume: String = seq.next_element()?.ok_or(de::Error::invalid_length(7, &self))?;
        let number_of_trades: i64 = seq.next_element()?.ok_or(de::Error::invalid_length(8, &self))?;
        let taker_buy_base_asset_volume: String = seq.next_element()?.ok_or(de::Error::invalid_length(9, &self))?;
        let taker_buy_quote_asset_volume: String = seq.next_element()?.ok_or(de::Error::invalid_length(10, &self))?;
        let is_final: bool = seq.next_element()?.ok_or(de::Error::invalid_length(11, &self))?;

        Ok(Candle {
            symbol: Default::default(),
            timeframe: Default::default(),
            open_time,
            open: open.parse().unwrap(),
            high: high.parse().unwrap(),
            low: low.parse().unwrap(),
            close: close.parse().unwrap(),
            volume: volume.parse().unwrap(),
            close_time,
            quote_asset_volume: quote_asset_volume.parse().unwrap(),
            number_of_trades,
            taker_buy_base_asset_volume: taker_buy_base_asset_volume.parse().unwrap(),
            taker_buy_quote_asset_volume: taker_buy_quote_asset_volume.parse().unwrap(),
            is_final,
        })
    }
}

fn from_str_array_to_candle<'de, D>(deserializer: D) -> Result<Candle, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(CandleVisitor)
}

// ----------------------------------------------------------------------------
// A custom deserializer for SymbolRules
// ----------------------------------------------------------------------------

fn deserialize_symbol_rules<'de, D>(deserializer: D) -> Result<Rules, D::Error>
where
    D: Deserializer<'de>,
{
    struct SymbolRulesVisitor;

    impl<'de> Visitor<'de> for SymbolRulesVisitor {
        type Value = Rules;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a sequence of filter objects")
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Rules, S::Error>
        where
            S: SeqAccess<'de>,
        {
            let mut symbol_rules = Rules::default();

            // Since we're iterating over a sequence, deserialize each element of the sequence as a generic serde_json::Value to inspect it
            while let Some(elem) = seq.next_element::<serde_json::Value>()? {
                let filter_type = elem.get("filterType").and_then(serde_json::Value::as_str);

                match filter_type {
                    Some("PRICE_FILTER") => {
                        let filter: PriceFilterRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.price_filter = Some(filter);
                    }

                    Some("LOT_SIZE") => {
                        let filter: LotSizeRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.lot_size = Some(filter);
                    }

                    Some("MARKET_LOT_SIZE") => {
                        let filter: MarketLotSizeRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.market_lot_size = Some(filter);
                    }

                    Some("MIN_NOTIONAL") => {
                        let filter: MinNotionalRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.min_notional = Some(filter);
                    }

                    Some("NOTIONAL") => {
                        let filter: NotionalRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.notional = Some(filter);
                    }

                    Some("ICEBERG_PARTS") => {
                        let filter: IcebergPartsRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.iceberg_parts = Some(filter);
                    }

                    Some("MAX_NUM_ORDERS") => {
                        let filter: MaxNumOrdersRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.max_num_orders = Some(filter);
                    }

                    Some("MAX_NUM_ALGO_ORDERS") => {
                        let filter: MaxNumAlgoOrdersRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.max_num_algo_orders = Some(filter);
                    }

                    Some("MAX_NUM_ICEBERG_ORDERS") => {
                        let filter: MaxNumIcebergOrdersRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.max_num_iceberg_orders = Some(filter);
                    }

                    Some("MAX_POSITION") => {
                        let filter: MaxPositionRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.max_position = Some(filter);
                    }

                    Some("TRAILING_DELTA") => {
                        let filter: TrailingDeltaRule = serde_json::from_value(elem.clone()).map_err(de::Error::custom)?;
                        symbol_rules.trailing_delta = Some(filter);
                    }

                    _ => {}
                }
            }

            Ok(symbol_rules)
        }
    }

    deserializer.deserialize_seq(SymbolRulesVisitor)
}

// ----------------------------------------------------------------------------
// A custom deserializer for the array of strings representing the order types
// ----------------------------------------------------------------------------

struct NormalizedOrderTypeVecVisitor;

impl<'de> Visitor<'de> for NormalizedOrderTypeVecVisitor {
    type Value = Vec<NormalizedOrderType>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a sequence of strings")
    }

    fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
    where
        S: SeqAccess<'de>,
    {
        let mut vec: Vec<NormalizedOrderType> = Vec::with_capacity(seq.size_hint().unwrap_or(0));

        while let Some(value) = seq.next_element::<&str>()? {
            let normalized_order_type: NormalizedOrderType = value.parse().map_err(de::Error::custom)?;
            vec.push(normalized_order_type);
        }

        Ok(vec)
    }
}

fn from_str_to_normalized_order_type_vec<'de, D>(deserializer: D) -> Result<Vec<NormalizedOrderType>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(NormalizedOrderTypeVecVisitor)
}

#[derive(Debug, Deserialize)]
pub struct WebsocketResponse {
    pub stream: Ustr,
    pub data:   WebsocketMessage,
}

#[derive(Debug, Deserialize)]
pub struct WebsocketUserStreamResponse {
    pub stream: Ustr,
    pub data:   WebsocketUserMessage,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum WebsocketMessage {
    ExchangeTrade(ExchangeTradeEvent),
    DiffDepth(DiffDepth),
    Kline(CandleData),
    PartialDepth(PartialDepth),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "e")]
pub enum WebsocketUserMessage {
    #[serde(rename = "executionReport")]
    OrderUpdate(OrderUpdateEvent),

    #[serde(rename = "outboundAccountPosition")]
    AccountUpdate(AccountUpdateEvent),

    #[serde(rename = "balanceUpdate")]
    BalanceUpdate(BalanceUpdateEvent),
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct CandleData {
    #[serde(rename = "E")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub event_time: DateTime<Utc>,

    #[serde(rename = "s")]
    pub symbol: Ustr,

    #[serde(rename = "k")]
    pub candle: Candle,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct Depth {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,

    #[serde(deserialize_with = "from_str_to_f64_vec_array")]
    pub bids: Vec<[OrderedValueType; 2]>,

    #[serde(deserialize_with = "from_str_to_f64_vec_array")]
    pub asks: Vec<[OrderedValueType; 2]>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct PartialDepth {
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: u64,

    #[serde(deserialize_with = "from_str_to_f64_array")]
    pub bids: [[OrderedValueType; 2]; 20],

    #[serde(deserialize_with = "from_str_to_f64_array")]
    pub asks: [[OrderedValueType; 2]; 20],
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct DiffDepth {
    #[serde(rename = "E")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub event_time: DateTime<Utc>,

    #[serde(rename = "s")]
    pub symbol: Ustr,

    #[serde(rename = "U")]
    pub first_update_id: u64,

    #[serde(rename = "u")]
    pub final_update_id: u64,

    #[serde(rename = "b")]
    #[serde(deserialize_with = "from_str_to_f64_vec_array")]
    pub bids: Vec<[OrderedValueType; 2]>,

    #[serde(rename = "a")]
    #[serde(deserialize_with = "from_str_to_f64_vec_array")]
    pub asks: Vec<[OrderedValueType; 2]>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct AccountInformation {
    #[serde(rename = "makerCommission")]
    pub maker_commission: u32,

    #[serde(rename = "takerCommission")]
    pub taker_commission: u32,

    #[serde(rename = "buyerCommission")]
    pub buyer_commission: u32,

    #[serde(rename = "sellerCommission")]
    pub seller_commission: u32,

    #[serde(rename = "commissionRates")]
    pub commission_rates: CommissionRates,

    #[serde(rename = "canTrade")]
    pub can_trade: bool,

    #[serde(rename = "canWithdraw")]
    pub can_withdraw: bool,

    #[serde(rename = "canDeposit")]
    pub can_deposit: bool,

    #[serde(rename = "brokered")]
    pub brokered: bool,

    #[serde(rename = "requireSelfTradePrevention")]
    pub require_self_trade_prevention: bool,

    #[serde(rename = "preventSor")]
    pub prevent_sor: bool,

    #[serde(rename = "updateTime")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub update_time: DateTime<Utc>,

    #[serde(rename = "accountType")]
    pub account_type: Ustr,

    pub balances: Vec<Balance>,

    pub permissions: Vec<Ustr>,

    pub uid: u64,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct CommissionRates {
    #[serde(rename = "maker")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub maker: OrderedValueType,

    #[serde(rename = "taker")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub taker: OrderedValueType,

    #[serde(rename = "buyer")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub buyer: OrderedValueType,

    #[serde(rename = "seller")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub seller: OrderedValueType,
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq, Eq, Clone, Copy)]
pub struct Balance {
    #[serde(rename = "a", alias = "asset")]
    pub asset: Ustr,

    #[serde(rename = "f", alias = "free")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub free: OrderedValueType,

    #[serde(rename = "l", alias = "locked")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub locked: OrderedValueType,
}

impl Balance {
    pub fn new(asset: Ustr, free: OrderedValueType, locked: OrderedValueType) -> Self {
        Self { asset, free, locked }
    }

    pub fn lock(&mut self, amount: OrderedValueType) {
        self.free -= amount;
        self.locked += amount;
    }

    pub fn unlock(&mut self, amount: OrderedValueType) {
        self.free += amount;
        self.locked -= amount;
    }

    pub fn asset(&self) -> Ustr {
        self.asset
    }

    pub fn free(&self) -> OrderedValueType {
        self.free
    }

    pub fn locked(&self) -> OrderedValueType {
        self.locked
    }

    pub fn total(&self) -> OrderedValueType {
        self.free + self.locked
    }
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct ExchangeInformation {
    pub timezone:         Ustr,
    #[serde(rename = "serverTime")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub server_time:      DateTime<Utc>,
    #[serde(rename = "rateLimits")]
    pub rate_limits:      Vec<RateLimit>,
    #[serde(rename = "exchangeFilters")]
    pub exchange_filters: Vec<Ustr>,
    pub symbols:          Vec<Symbol>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct RateLimit {
    #[serde(rename = "rateLimitType")]
    pub rate_limit_type: Ustr,
    pub interval:        Ustr,
    #[serde(rename = "intervalNum")]
    pub interval_num:    u32,
    pub limit:           u32,
}

#[serde_as]
#[derive(Debug, Deserialize, Clone)]
pub struct Symbol {
    pub symbol:                              Ustr,
    pub status:                              Ustr,
    #[serde(rename = "baseAsset")]
    pub base_asset:                          Ustr,
    #[serde(rename = "baseAssetPrecision")]
    pub base_asset_precision:                u32,
    #[serde(rename = "quoteAsset")]
    pub quote_asset:                         Ustr,
    #[serde(rename = "quotePrecision")]
    pub quote_precision:                     u32,
    #[serde(rename = "quoteAssetPrecision")]
    pub quote_asset_precision:               u32,
    #[serde(rename = "baseCommissionPrecision")]
    pub base_commission_precision:           u32,
    #[serde(rename = "quoteCommissionPrecision")]
    pub quote_commission_precision:          u32,
    #[serde(rename = "orderTypes")]
    #[serde(deserialize_with = "from_str_to_normalized_order_type_vec")]
    pub order_types:                         Vec<NormalizedOrderType>,
    #[serde(rename = "icebergAllowed")]
    pub iceberg_allowed:                     bool,
    #[serde(rename = "ocoAllowed")]
    pub oco_allowed:                         bool,
    #[serde(rename = "quoteOrderQtyMarketAllowed")]
    pub quote_order_qty_market_allowed:      bool,
    #[serde(rename = "allowTrailingStop")]
    pub allow_trailing_stop:                 bool,
    #[serde(rename = "cancelReplaceAllowed")]
    pub cancel_replace_allowed:              bool,
    // pub filters:                             Vec<SymbolFilter>,
    #[serde(rename = "filters")]
    #[serde(deserialize_with = "deserialize_symbol_rules")]
    pub rules:                               Rules,
    #[serde(rename = "isSpotTradingAllowed")]
    pub is_spot_trading_allowed:             bool,
    #[serde(rename = "isMarginTradingAllowed")]
    pub is_margin_trading_allowed:           bool,
    pub permissions:                         Vec<Ustr>,
    #[serde(rename = "defaultSelfTradePreventionMode")]
    pub default_self_trade_prevention_mode:  Ustr,
    #[serde(rename = "allowedSelfTradePreventionModes")]
    pub allowed_self_trade_prevention_modes: Vec<Ustr>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct PriceFilterRule {
    #[serde(rename = "minPrice")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub min_price: OrderedValueType,

    #[serde(rename = "maxPrice")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub max_price: OrderedValueType,

    #[serde(rename = "tickSize")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub tick_size: OrderedValueType,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct PercentPriceRule {
    #[serde(rename = "multiplierUp")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub multiplier_up: OrderedValueType,

    #[serde(rename = "multiplierDown")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub multiplier_down: OrderedValueType,

    #[serde(rename = "avgPriceMins")]
    pub avg_price_mins: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct LotSizeRule {
    #[serde(rename = "minQty")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub min_qty: OrderedValueType,

    #[serde(rename = "maxQty")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub max_qty: OrderedValueType,

    #[serde(rename = "stepSize")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub step_size: OrderedValueType,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct MarketLotSizeRule {
    #[serde(rename = "minQty")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub min_qty: OrderedValueType,

    #[serde(rename = "maxQty")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub max_qty: OrderedValueType,

    #[serde(rename = "stepSize")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub step_size: OrderedValueType,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct MinNotionalRule {
    #[serde(rename = "minNotional")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub min_notional: OrderedValueType,

    #[serde(rename = "applyToMarket")]
    pub apply_to_market: bool,

    #[serde(rename = "avgPriceMins")]
    pub avg_price_mins: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct NotionalRule {
    #[serde(rename = "minNotional")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub min_notional: OrderedValueType,

    #[serde(rename = "applyMinToMarket")]
    pub apply_min_to_market: bool,

    #[serde(rename = "maxNotional")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub max_notional: OrderedValueType,

    #[serde(rename = "applyMaxToMarket")]
    pub apply_max_to_market: bool,

    #[serde(rename = "avgPriceMins")]
    pub avg_price_mins: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct IcebergPartsRule {
    #[serde(rename = "limit")]
    pub limit: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct MaxNumIcebergOrdersRule {
    #[serde(rename = "maxNumIcebergOrders")]
    pub max_num_iceberg_orders: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct MaxNumOrdersRule {
    #[serde(rename = "maxNumOrders")]
    pub max_num_orders: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct MaxNumAlgoOrdersRule {
    #[serde(rename = "maxNumAlgoOrders")]
    pub max_num_algo_orders: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct MaxPositionRule {
    #[serde(rename = "maxPosition")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub max_position: OrderedValueType,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct TrailingDeltaRule {
    #[serde(rename = "minTrailingAboveDelta")]
    pub min_trailing_above_delta: u32,

    #[serde(rename = "maxTrailingAboveDelta")]
    pub max_trailing_above_delta: u32,

    #[serde(rename = "minTrailingBelowDelta")]
    pub min_trailing_below_delta: u32,

    #[serde(rename = "maxTrailingBelowDelta")]
    pub max_trailing_below_delta: u32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(tag = "filterType")]
pub enum SymbolFilterRule {
    #[serde(rename = "PRICE_FILTER")]
    PriceFilter(PriceFilterRule),

    #[serde(rename = "PERCENT_PRICE")]
    PercentPrice(PercentPriceRule),

    #[serde(rename = "LOT_SIZE")]
    LotSize(LotSizeRule),

    #[serde(rename = "MARKET_LOT_SIZE")]
    MarketLotSize(MarketLotSizeRule),

    #[serde(rename = "NOTIONAL")]
    Notional(NotionalRule),

    #[serde(rename = "MIN_NOTIONAL")]
    MinNotional(MinNotionalRule),

    #[serde(rename = "ICEBERG_PARTS")]
    IcebergParts(IcebergPartsRule),

    #[serde(rename = "MAX_NUM_ORDERS")]
    MaxNumOrders(MaxNumOrdersRule),

    #[serde(rename = "MAX_NUM_ALGO_ORDERS")]
    MaxNumAlgoOrders(MaxNumAlgoOrdersRule),

    #[serde(rename = "MAX_NUM_ICEBERG_ORDERS")]
    MaxNumIcebergOrders(MaxNumIcebergOrdersRule),

    #[serde(rename = "MAX_POSITION")]
    MaxPosition(MaxPositionRule),

    #[serde(rename = "TRAILING_DELTA")]
    TrailingDelta(TrailingDeltaRule),
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
pub struct Rules {
    #[serde(rename = "priceFilter", default, skip_serializing_if = "Option::is_none")]
    pub price_filter: Option<PriceFilterRule>,

    #[serde(rename = "percentPrice", default, skip_serializing_if = "Option::is_none")]
    pub percent_price: Option<PercentPriceRule>,

    #[serde(rename = "lotSize", default, skip_serializing_if = "Option::is_none")]
    pub lot_size: Option<LotSizeRule>,

    #[serde(rename = "marketLotSize", default, skip_serializing_if = "Option::is_none")]
    pub market_lot_size: Option<MarketLotSizeRule>,

    #[serde(rename = "minNotional", default, skip_serializing_if = "Option::is_none")]
    pub min_notional: Option<MinNotionalRule>,

    #[serde(rename = "notional", default, skip_serializing_if = "Option::is_none")]
    pub notional: Option<NotionalRule>,

    #[serde(rename = "icebergParts", default, skip_serializing_if = "Option::is_none")]
    pub iceberg_parts: Option<IcebergPartsRule>,

    #[serde(rename = "maxNumIcebergOrders", default, skip_serializing_if = "Option::is_none")]
    pub max_num_iceberg_orders: Option<MaxNumIcebergOrdersRule>,

    #[serde(rename = "maxNumOrders", default, skip_serializing_if = "Option::is_none")]
    pub max_num_orders: Option<MaxNumOrdersRule>,

    #[serde(rename = "maxNumAlgoOrders", default, skip_serializing_if = "Option::is_none")]
    pub max_num_algo_orders: Option<MaxNumAlgoOrdersRule>,

    #[serde(rename = "maxPosition", default, skip_serializing_if = "Option::is_none")]
    pub max_position: Option<MaxPositionRule>,

    #[serde(rename = "trailingDelta", default, skip_serializing_if = "Option::is_none")]
    pub trailing_delta: Option<TrailingDeltaRule>,
}

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct OrderUpdateEvent {
    /// Event time
    #[serde(rename = "E")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub event_time: DateTime<Utc>,

    /// Symbol
    #[serde(rename = "s")]
    pub symbol: Ustr,

    /// Client order ID
    #[serde(rename = "c")]
    pub client_order_id: String,

    /// Side
    #[serde(rename = "S")]
    pub side: NormalizedSide,

    /// Order type
    #[serde(rename = "o")]
    pub order_type: NormalizedOrderType,

    /// Time in force
    #[serde(rename = "f")]
    pub time_in_force: NormalizedTimeInForce,

    /// Order quantity
    #[serde(rename = "q")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub order_quantity: OrderedValueType,

    /// Order price
    #[serde(rename = "p")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub order_price: OrderedValueType,

    /// Stop price
    #[serde(rename = "P")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub stop_price: OrderedValueType,

    /// Iceberg quantity
    #[serde(rename = "F")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub iceberg_quantity: OrderedValueType,

    /// Order list ID; -1 if not an OCO order
    #[serde(rename = "g")]
    pub order_list_id: i64,

    /// Original client order ID; This is the ID that the new order is replacing.
    #[serde(rename = "C")]
    pub original_client_order_id: String,

    /// Current execution type
    #[serde(rename = "x")]
    pub current_execution_type: NormalizedExecutionType,

    /// Current order status
    #[serde(rename = "X")]
    pub current_order_status: NormalizedOrderStatus,

    /// Order reject reason; will be an error code.
    #[serde(rename = "r")]
    pub order_reject_reason: String,

    /// Order ID
    #[serde(rename = "i")]
    pub order_id: i64,

    /// Last executed quantity
    #[serde(rename = "l")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub last_executed_quantity: OrderedValueType,

    /// Cumulative filled quantity
    #[serde(rename = "z")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub cumulative_filled_quantity: OrderedValueType,

    /// Last executed price
    #[serde(rename = "L")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub last_executed_price: OrderedValueType,

    /// Commission amount
    #[serde(rename = "n")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub commission_amount: OrderedValueType,

    /// Commission asset
    #[serde(rename = "N")]
    pub commission_asset: Option<Ustr>,

    /// Transaction time
    #[serde(rename = "T")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub transaction_time: DateTime<Utc>,

    /// Trade ID
    #[serde(rename = "t")]
    pub trade_id: i64,

    /// Prevented Match Id
    /// NOTE: This is only visible if the order expired due to STP
    #[serde(rename = "v")]
    pub prevented_match_id: Option<i64>,

    /// Ignore
    #[serde(rename = "I")]
    pub ignore: i64,

    /// Is the order on the book?
    #[serde(rename = "w")]
    pub is_order_on_the_book: bool,

    /// Is this trade the maker side? (true if the trade is maker side)
    #[serde(rename = "m")]
    pub is_trade_the_maker_side: bool,

    /// Ignore
    #[serde(rename = "M")]
    pub ignore2: bool,

    /// Order creation time
    #[serde(rename = "O")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub order_creation_time: DateTime<Utc>,

    /// Cumulative quote asset transacted quantity
    #[serde(rename = "Z")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub cumulative_quote_asset_transacted_quantity: OrderedValueType,

    /// Last quote asset transacted quantity (i.e. lastPrice * lastQty)
    #[serde(rename = "Y")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub last_quote_asset_transacted_quantity: OrderedValueType,

    /// Quote Order Quantity
    #[serde(rename = "Q")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub quote_order_quantity: OrderedValueType,

    /// Working Time
    /// NOTE: This is only visible if the order has been placed on the book.
    #[serde(rename = "W")]
    #[serde_as(as = "Option<serde_with::TimestampMilliSeconds>")]
    pub working_time: Option<DateTime<Utc>>,

    /// Self-Trade Prevention Mode
    #[serde(rename = "V")]
    pub self_trade_prevention_mode: Ustr,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct BalanceUpdateEvent {
    #[serde(rename = "E")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub event_time: DateTime<Utc>,

    #[serde(rename = "a")]
    pub asset: Ustr,

    #[serde(rename = "d")]
    #[serde(deserialize_with = "from_str_to_ordered_value_type")]
    pub balance_delta: OrderedValueType,

    #[serde(rename = "T")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub clear_time: DateTime<Utc>,
}

#[serde_as]
#[derive(Debug, Deserialize)]
pub struct AccountUpdateEvent {
    #[serde(rename = "E")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub event_time: DateTime<Utc>,

    #[serde(rename = "u")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub time_of_last_account_update: DateTime<Utc>,

    #[serde(rename = "B")]
    pub balances: Vec<Balance>,
}

#[derive(Debug, Deserialize)]
pub struct ListenKey {
    #[serde(rename = "listenKey")]
    pub listen_key: String,
}

#[serde_as]
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ExchangeTradeEvent {
    #[serde(rename = "E")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub event_time: DateTime<Utc>,

    #[serde(rename = "t")]
    pub trade_id: u64,

    #[serde(rename = "s")]
    pub symbol: Ustr,

    #[serde(rename = "p")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub price: OrderedValueType,

    #[serde(rename = "q")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub quantity: OrderedValueType,

    #[serde(rename = "m")]
    pub is_buyer_maker: bool,

    #[serde(rename = "T")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub trade_time: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{f, timeframe::Timeframe, util::parse_candle_data_array};
    use ustr::ustr;

    #[test]
    fn test_deserialize_candle() {
        let text = r#"{"stream":"btcusdt@kline_1s","data":{"e":"kline","E":1706597270004,"s":"BTCUSDT","k":{"t":1706597269000,"T":1706597269999,"s":"BTCUSDT","i":"1s","f":276120251,"L":276120254,"o":"43297.45000000","c":"43296.79000000","h":"43298.97000000","l":"43296.79000000","v":"0.10212000","n":4,"x":true,"q":"4421.55717160","V":"0.04325000","Q":"1872.67514430","B":"0"}}}"#;
        let msg: WebsocketResponse = serde_json::from_str(text).unwrap();

        let WebsocketMessage::Kline(c) = msg.data else {
            panic!("Expected Kline data");
        };

        assert_eq!(msg.stream, ustr("btcusdt@kline_1s"));
        assert_eq!(c.event_time.timestamp_millis(), 1706597270004);
        assert_eq!(c.symbol, ustr("BTCUSDT"));
        assert_eq!(c.candle.open_time.timestamp_millis(), 1706597269000);
        assert_eq!(c.candle.close_time.timestamp_millis(), 1706597269999);
        assert_eq!(c.candle.open, 43297.45000000);
        assert_eq!(c.candle.high, 43298.97000000);
        assert_eq!(c.candle.low, 43296.79000000);
        assert_eq!(c.candle.close, 43296.79000000);
        assert_eq!(c.candle.volume, 0.10212000);
        assert_eq!(c.candle.number_of_trades, 4);
        assert_eq!(c.candle.quote_asset_volume, 4421.55717160);
        assert_eq!(c.candle.taker_buy_quote_asset_volume, 0.04325000);
        assert_eq!(c.candle.taker_buy_base_asset_volume, 1872.67514430);
        assert_eq!(c.candle.is_final, true);
    }

    #[test]
    fn test_deserialize_trade() {
        let text = r#"{"stream":"btcusdt@trade","data":{"e":"trade","E":1706602794682,"s":"BTCUSDT","t":276229663,"p":"43381.54000000","q":"0.10820000","b":2707014328,"a":2707014327,"T":1706602794681,"m":false,"M":true}}"#;
        let msg: WebsocketResponse = serde_json::from_str(text).unwrap();

        let WebsocketMessage::ExchangeTrade(t) = msg.data else {
            panic!("Expected Trade data");
        };

        assert_eq!(msg.stream, ustr("btcusdt@trade"));
        assert_eq!(t.event_time.timestamp_millis(), 1706602794682);
        assert_eq!(t.symbol, ustr("BTCUSDT"));
        assert_eq!(t.trade_id, 276229663);
        assert_eq!(t.price, 43381.54000000);
        assert_eq!(t.quantity, 0.10820000);
        assert_eq!(t.is_buyer_maker, false);
        assert_eq!(t.trade_time.timestamp_millis(), 1706602794681);
    }

    #[test]
    fn test_deserialize_partial_depth() {
        let text = r#"{
          "stream": "btcusdt@depth20@100ms",
          "data": {
            "lastUpdateId": 5941894191,
            "bids": [
              [
                "65465.84000000",
                "0.05161000"
              ],
              [
                "65465.83000000",
                "0.06000000"
              ],
              [
                "65465.82000000",
                "0.00304000"
              ],
              [
                "65465.76000000",
                "0.00295000"
              ],
              [
                "65465.73000000",
                "0.05331000"
              ],
              [
                "65465.61000000",
                "0.00296000"
              ],
              [
                "65465.52000000",
                "0.00302000"
              ],
              [
                "65465.45000000",
                "0.00300000"
              ],
              [
                "65465.32000000",
                "0.05500000"
              ],
              [
                "65465.20000000",
                "0.00066000"
              ],
              [
                "65465.18000000",
                "0.05000000"
              ],
              [
                "65465.00000000",
                "0.00500000"
              ],
              [
                "65464.62000000",
                "0.05000000"
              ],
              [
                "65464.40000000",
                "0.03060000"
              ],
              [
                "65464.11000000",
                "0.29732000"
              ],
              [
                "65464.00000000",
                "0.09110000"
              ],
              [
                "65463.95000000",
                "0.04580000"
              ],
              [
                "65463.80000000",
                "0.03060000"
              ],
              [
                "65463.39000000",
                "0.81916000"
              ],
              [
                "65463.16000000",
                "0.00010000"
              ]
            ],
            "asks": [
              [
                "65468.89000000",
                "0.00303000"
              ],
              [
                "65468.90000000",
                "0.00295000"
              ],
              [
                "65468.91000000",
                "0.00796000"
              ],
              [
                "65468.92000000",
                "0.00302000"
              ],
              [
                "65468.93000000",
                "0.00300000"
              ],
              [
                "65469.06000000",
                "0.02881000"
              ],
              [
                "65469.19000000",
                "0.00473000"
              ],
              [
                "65470.00000000",
                "0.09110000"
              ],
              [
                "65470.28000000",
                "0.00011000"
              ],
              [
                "65470.78000000",
                "0.02000000"
              ],
              [
                "65470.79000000",
                "0.10820000"
              ],
              [
                "65470.80000000",
                "0.03060000"
              ],
              [
                "65471.00000000",
                "0.04580000"
              ],
              [
                "65471.33000000",
                "0.00011000"
              ],
              [
                "65471.70000000",
                "0.05500000"
              ],
              [
                "65471.80000000",
                "0.03060000"
              ],
              [
                "65471.88000000",
                "0.00937000"
              ],
              [
                "65472.12000000",
                "0.05500000"
              ],
              [
                "65472.26000000",
                "0.05000000"
              ],
              [
                "65472.40000000",
                "0.03060000"
              ]
            ]
          }
        }"#;

        let msg: WebsocketResponse = serde_json::from_str(text).unwrap();

        let WebsocketMessage::PartialDepth(d) = msg.data else {
            panic!("Expected PartialDepth data");
        };

        assert_eq!(msg.stream, ustr("btcusdt@depth20@100ms"));
        assert_eq!(d.last_update_id, 5941894191);
        assert_eq!(d.bids[0], [65465.84000000, 0.05161000]);
        assert_eq!(d.bids[19], [65463.16000000, 0.00010000]);
        assert_eq!(d.asks[0], [65468.89000000, 0.00303000]);
        assert_eq!(d.asks[19], [65472.40000000, 0.03060000]);
    }

    #[test]
    fn test_deserialize_diff_depth() {
        let text = r#"{
          "stream": "btcusdt@depth@100ms",
          "data": {
            "e": "depthUpdate",
            "E": 1709558365583,
            "s": "BTCUSDT",
            "U": 5941894137,
            "u": 5941894218,
            "b": [
              [
                "65467.94000000",
                "0.00000000"
              ],
              [
                "65465.95000000",
                "0.05161000"
              ],
              [
                "65465.94000000",
                "0.06042000"
              ],
              [
                "65465.93000000",
                "0.00000000"
              ],
              [
                "65465.92000000",
                "0.00000000"
              ],
              [
                "65465.91000000",
                "0.00000000"
              ],
              [
                "65465.90000000",
                "0.00000000"
              ],
              [
                "65465.89000000",
                "0.00000000"
              ],
              [
                "65465.88000000",
                "0.00000000"
              ],
              [
                "65465.87000000",
                "0.00000000"
              ],
              [
                "65465.86000000",
                "0.00000000"
              ],
              [
                "65465.85000000",
                "0.00000000"
              ],
              [
                "65465.84000000",
                "0.00000000"
              ],
              [
                "65465.83000000",
                "0.00000000"
              ],
              [
                "65465.82000000",
                "0.00000000"
              ],
              [
                "65465.81000000",
                "0.00000000"
              ],
              [
                "65465.80000000",
                "0.00000000"
              ],
              [
                "65465.79000000",
                "0.00000000"
              ],
              [
                "65465.78000000",
                "0.00000000"
              ],
              [
                "65465.77000000",
                "0.00000000"
              ],
              [
                "65465.76000000",
                "0.00000000"
              ],
              [
                "65465.75000000",
                "0.00000000"
              ],
              [
                "65465.74000000",
                "0.00000000"
              ],
              [
                "65465.73000000",
                "0.05331000"
              ],
              [
                "65465.61000000",
                "0.00296000"
              ],
              [
                "65465.52000000",
                "0.00000000"
              ],
              [
                "65465.05000000",
                "0.00000000"
              ],
              [
                "65463.40000000",
                "0.00000000"
              ],
              [
                "65458.11000000",
                "0.00763000"
              ],
              [
                "65455.51000000",
                "0.00000000"
              ],
              [
                "65455.14000000",
                "0.00311000"
              ],
              [
                "65454.87000000",
                "0.00000000"
              ],
              [
                "65442.45000000",
                "0.00000000"
              ],
              [
                "65438.53000000",
                "0.00000000"
              ]
            ],
            "a": [
              [
                "65468.88000000",
                "0.00000000"
              ],
              [
                "65468.89000000",
                "0.00000000"
              ],
              [
                "65468.90000000",
                "0.00000000"
              ],
              [
                "65468.91000000",
                "0.00000000"
              ],
              [
                "65468.92000000",
                "0.00000000"
              ],
              [
                "65468.93000000",
                "0.00000000"
              ],
              [
                "65468.94000000",
                "0.00000000"
              ],
              [
                "65469.06000000",
                "0.02839000"
              ],
              [
                "65469.07000000",
                "0.00000000"
              ],
              [
                "65470.94000000",
                "0.00000000"
              ],
              [
                "65473.80000000",
                "0.00948000"
              ],
              [
                "65473.84000000",
                "0.00153000"
              ],
              [
                "65475.99000000",
                "0.00000000"
              ],
              [
                "65477.99000000",
                "0.02903000"
              ],
              [
                "65479.42000000",
                "0.03380000"
              ],
              [
                "65479.43000000",
                "0.00000000"
              ],
              [
                "65482.28000000",
                "0.00093000"
              ],
              [
                "65483.60000000",
                "0.00000000"
              ],
              [
                "65489.99000000",
                "0.00000000"
              ],
              [
                "65496.06000000",
                "0.03358000"
              ]
            ]
          }
        }"#;

        let msg: WebsocketResponse = serde_json::from_str(text).unwrap();

        if let WebsocketMessage::DiffDepth(u) = msg.data {
            assert_eq!(msg.stream, ustr("btcusdt@depth@100ms"));
            assert_eq!(u.event_time.timestamp_millis(), 1709558365583);
            assert_eq!(u.symbol, ustr("BTCUSDT"));
            assert_eq!(u.first_update_id, 5941894137);
            assert_eq!(u.final_update_id, 5941894218);
            assert_eq!(u.bids[0], [65467.94000000, 0.00000000]);
            assert_eq!(u.bids[33], [65438.53000000, 0.00000000]);
            assert_eq!(u.asks[0], [65468.88000000, 0.00000000]);
            assert_eq!(u.asks[19], [65496.06000000, 0.03358000]);
        } else {
            panic!("Expected DepthUpdate data");
        }
    }

    #[test]
    fn test_deserialize_account_information() {
        let text = r#"{
              "makerCommission": 15,
              "takerCommission": 15,
              "buyerCommission": 0,
              "sellerCommission": 0,
              "commissionRates": {
                "maker": "0.00150000",
                "taker": "0.00150000",
                "buyer": "0.00000000",
                "seller": "0.00000000"
              },
              "canTrade": true,
              "canWithdraw": true,
              "canDeposit": true,
              "brokered": false,
              "requireSelfTradePrevention": false,
              "preventSor": false,
              "updateTime": 123456789,
              "accountType": "SPOT",
              "balances": [
                {
                  "asset": "BTC",
                  "free": "4723846.89208129",
                  "locked": "0.00000000"
                },
                {
                  "asset": "LTC",
                  "free": "4763368.68006011",
                  "locked": "0.00000000"
                }
              ],
              "permissions": [
                "SPOT"
              ],
              "uid": 354937868
        }"#;

        let a: AccountInformation = serde_json::from_str(text).unwrap();

        println!("{:#?}", a);

        assert_eq!(a.maker_commission, 15);
        assert_eq!(a.taker_commission, 15);
        assert_eq!(a.buyer_commission, 0);
        assert_eq!(a.seller_commission, 0);
        assert_eq!(a.commission_rates.maker, 0.0015);
        assert_eq!(a.commission_rates.taker, 0.0015);
        assert_eq!(a.commission_rates.buyer, 0.0000);
        assert_eq!(a.commission_rates.seller, 0.0000);
        assert_eq!(a.can_trade, true);
        assert_eq!(a.can_withdraw, true);
        assert_eq!(a.can_deposit, true);
        assert_eq!(a.brokered, false);
        assert_eq!(a.require_self_trade_prevention, false);
        assert_eq!(a.prevent_sor, false);
        assert_eq!(a.update_time.timestamp_millis(), 123456789);
        assert_eq!(a.account_type, ustr("SPOT"));
        assert_eq!(
            a.balances,
            vec![
                Balance {
                    asset:  ustr("BTC"),
                    free:   f!(4723846.89208129),
                    locked: f!(0.00000000),
                },
                Balance {
                    asset:  ustr("LTC"),
                    free:   f!(4763368.68006011),
                    locked: f!(0.00000000),
                }
            ]
        );
        assert_eq!(a.permissions, vec![ustr("SPOT")]);
        assert_eq!(a.uid, 354937868);
    }

    #[test]
    fn test_deserialize_candle_data_array() {
        let text = r#"[[1709964395000,"68128.34000000","68128.94000000","68127.41000000","68127.55000000","0.01653000",1709964395999,"1126.15351250",6,"0.00234000","159.42144140","0"],[1709964396000,"68127.64000000","68128.00000000","68125.00000000","68126.76000000","0.11810000",1709964396999,"8045.68029330",9,"0.03996000","2722.35128460","0"],[1709964397000,"68126.63000000","68126.90000000","68124.47000000","68125.65000000","0.01783000",1709964397999,"1214.69095820",12,"0.01106000","753.48148030","0"],[1709964398000,"68127.20000000","68129.18000000","68125.59000000","68127.68000000","0.08621000",1709964398999,"5873.35273140",7,"0.04500000","3065.81145660","0"],[1709964399000,"68128.92000000","68128.92000000","68126.99000000","68126.99000000","0.13940000",1709964399999,"9497.13258740",25,"0.13864000","9445.35572100","0"],[1709964400000,"68128.80000000","68131.46000000","68128.54000000","68130.15000000","0.57451000",1709964400999,"39141.41213170",40,"0.49752000","33896.12745320","0"],[1709964401000,"68128.91000000","68129.89000000","68124.11000000","68127.00000000","0.45657000",1709964401999,"31104.43041120",24,"0.27137000","18487.54205340","0"],[1709964402000,"68124.24000000","68127.31000000","68124.24000000","68124.83000000","0.04583000",1709964402999,"3122.19146460",5,"0.01487000","1013.05309970","0"],[1709964403000,"68124.44000000","68125.85000000","68121.94000000","68124.26000000","0.41852000",1709964403999,"28510.86468230",15,"0.08860000","6035.70149020","0"],[1709964404000,"68124.31000000","68124.61000000","68123.93000000","68124.61000000","0.05227000",1709964404999,"3560.85305440",6,"0.00037000","25.20595610","0"],[1709964405000,"68123.37000000","68124.24000000","68122.70000000","68122.70000000","0.07427000",1709964405999,"5059.52276260",7,"0.00026000","17.71230240","0"],[1709964406000,"68123.06000000","68125.92000000","68123.06000000","68125.92000000","0.00128000",1709964406999,"87.19780280",3,"0.00000000","0.00000000","0"],[1709964407000,"68127.56000000","68130.23000000","68127.56000000","68130.23000000","0.47724000",1709964407999,"32513.98982840",30,"0.45704000","31137.77219040","0"],[1709964408000,"68128.92000000","68129.94000000","68127.00000000","68129.39000000","0.01234000",1709964408999,"840.69306380",7,"0.00224000","152.61017180","0"],[1709964409000,"68129.21000000","68131.59000000","68129.21000000","68129.92000000","0.14662000",1709964409999,"9989.15738670",6,"0.14662000","9989.15738670","0"],[1709964410000,"68129.10000000","68130.49000000","68127.73000000","68130.37000000","0.04965000",1709964410999,"3382.63426660",7,"0.03744000","2550.78331200","0"],[1709964411000,"68130.67000000","68131.25000000","68130.67000000","68131.00000000","0.08133000",1709964411999,"5541.09831520",6,"0.05497000","3745.17385400","0"],[1709964412000,"68128.84000000","68128.84000000","68128.36000000","68128.50000000","0.03825000",1709964412999,"2605.92768040",8,"0.00020000","13.62567200","0"],[1709964413000,"68127.19000000","68127.19000000","68127.19000000","68127.19000000","0.00020000",1709964413999,"13.62543800",2,"0.00000000","0.00000000","0"],[1709964414000,"68125.08000000","68126.72000000","68123.72000000","68123.72000000","0.28567000",1709964414999,"19461.13902380",9,"0.00015000","10.21900800","0"],[1709964415000,"68123.32000000","68125.28000000","68122.69000000","68125.28000000","0.65228000",1709964415999,"44435.52578930",26,"0.16347000","11136.39245530","0"],[1709964416000,"68126.88000000","68128.89000000","68126.11000000","68126.16000000","0.20414000",1709964416999,"13907.38138820",12,"0.12725000","8669.15737830","0"]]"#;
        let candles = parse_candle_data_array(ustr("BTCUSDT"), Timeframe::M1, text);
    }

    #[test]
    fn test_deserialize_exchange_information() {
        let text = std::fs::read_to_string("testdata/exchange_information_09032024.json").expect("Failed to read exchange info file");
        let info: ExchangeInformation = serde_json::from_str(&text).unwrap();

        for symbol in &info.symbols {
            if symbol.symbol != ustr("BTCUSDT") {
                continue;
            }

            println!("{:#?}", symbol);
        }
    }

    #[test]
    fn test_deserialize_order_update() {
        let text = r#"{
          "stream": "KtvCT7YVqbeXP5D3H2hVTx3TeugwNJmuivvkJUIhkSkeYZnBFSWJEmcMKQOe",
          "data": {
            "e": "executionReport",
            "E": 1710396160782,
            "s": "ZILUSDT",
            "c": "electron_f9bc82079bba464ca50e72979d9",
            "S": "BUY",
            "o": "TAKE_PROFIT_LIMIT",
            "f": "GTC",
            "q": "153893.60000000",
            "p": "0.01209000",
            "P": "0.01212000",
            "F": "0.00000000",
            "g": -1,
            "C": "electron_5909a7a0fac74c479ec9e22f6a3",
            "x": "CANCELED",
            "X": "CANCELED",
            "r": "NONE",
            "i": 1380064647,
            "l": "0.00000000",
            "z": "0.00000000",
            "L": "0.00000000",
            "n": "0",
            "N": null,
            "T": 1710396160781,
            "t": -1,
            "I": 2863263556,
            "w": false,
            "m": false,
            "M": false,
            "O": 1710349845846,
            "Z": "0.00000000",
            "Y": "0.00000000",
            "Q": "0.00000000",
            "V": "EXPIRE_MAKER"
          }
        }"#;

        let update: WebsocketUserStreamResponse = serde_json::from_str(text).unwrap();

        println!("{:#?}", update);
    }

    #[test]
    fn test_deserialize_account_update() {
        let text = r#"{
          "stream": "99P5ObhbZjp45SGPon4XtBS6L6316GFY9drzIsoAi24gEVOMRYbrFsF9VtYl",
          "data": {
            "e": "outboundAccountPosition",
            "E": 1710232037808,
            "u": 1710232037808,
            "B": [
              {
                "a": "BNB",
                "f": "0.00000187",
                "l": "0.00000000"
              },
              {
                "a": "USDT",
                "f": "1860.57447796",
                "l": "0.00000000"
              },
              {
                "a": "OAX",
                "f": "0.79800000",
                "l": "0.00000000"
              }
            ]
          }
        }"#;

        let update: WebsocketUserStreamResponse = serde_json::from_str(text).unwrap();

        println!("{:#?}", update);
    }

    #[test]
    fn test_deserialize_balance_update() {
        let text = r#"{
            "stream": "99P5ObhbZjp45SGPon4XtBS6L6316GFY9drzIsoAi24gEVOMRYbrFsF9VtYl",
            "data": {
                "e": "balanceUpdate",
                "E": 1710232612885,
                "a": "USDT",
                "d": "10717.00000000",
                "T": 1710232612884
            }
        }"#;

        let update: WebsocketUserStreamResponse = serde_json::from_str(text).unwrap();

        println!("{:#?}", update);
    }
}
