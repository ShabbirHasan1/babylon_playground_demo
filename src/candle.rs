use crate::{timeframe::Timeframe, types::OrderedValueType};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use ordered_float::OrderedFloat;
use postgres::Row;
use serde_derive::Deserialize;
use serde_json::Deserializer;
use serde_with::serde_as;
use std::{ops::Add, str::FromStr};
use ustr::{ustr, Ustr};
use yata::core::{ValueType, OHLCV};

use crate::util::{calc_relative_position_between, datetime_round_down_to_base_ms, remove_milliseconds, remove_seconds};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd)]
pub enum CandleColor {
    Green,
    Red,
}

#[serde_as]
#[derive(Debug, Clone, Copy, Default, PartialOrd, Deserialize)]
pub struct Candle {
    #[serde(rename = "s")]
    pub symbol: Ustr,

    #[serde(rename = "i")]
    pub timeframe: Timeframe,

    #[serde(rename = "t")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub open_time: DateTime<Utc>,

    #[serde(rename = "T")]
    #[serde_as(as = "serde_with::TimestampMilliSeconds")]
    pub close_time: DateTime<Utc>,

    #[serde(rename = "o")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub open: ValueType,

    #[serde(rename = "h")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub high: ValueType,

    #[serde(rename = "l")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub low: ValueType,

    #[serde(rename = "c")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub close: ValueType,

    #[serde(rename = "v")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub volume: ValueType,

    #[serde(rename = "n")]
    pub number_of_trades: i64,

    #[serde(rename = "q")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub quote_asset_volume: ValueType,

    #[serde(rename = "V")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub taker_buy_quote_asset_volume: ValueType,

    #[serde(rename = "Q")]
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub taker_buy_base_asset_volume: ValueType,

    #[serde(rename = "x")]
    pub is_final: bool,
}

impl Candle {
    pub fn from<T: OHLCV + ?Sized>(src: &T) -> Self {
        Self {
            symbol:                       Default::default(),
            timeframe:                    Timeframe::NA,
            open_time:                    Utc::now(),
            close_time:                   Utc::now(),
            open:                         src.open(),
            high:                         src.high(),
            low:                          src.low(),
            close:                        src.close(),
            volume:                       src.volume(),
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     false,
        }
    }

    pub fn from_row(row: &Row) -> Self {
        Candle {
            symbol:                       ustr(row.get::<_, &str>("symbol")),
            timeframe:                    Timeframe::from_str(row.get::<_, &str>("timeframe")).expect("Invalid timeframe"),
            open_time:                    row.get("open_time"),
            close_time:                   row.get("close_time"),
            open:                         row.get("open"),
            high:                         row.get("high"),
            low:                          row.get("low"),
            close:                        row.get("close"),
            volume:                       row.get("volume"),
            number_of_trades:             row.get("number_of_trades"),
            quote_asset_volume:           row.get("quote_asset_volume"),
            taker_buy_quote_asset_volume: row.get("taker_buy_quote_asset_volume"),
            taker_buy_base_asset_volume:  row.get("taker_buy_base_asset_volume"),
            is_final:                     row.get("is_final"),
        }
    }

    #[inline]
    pub fn color(&self) -> CandleColor {
        if self.is_green() {
            CandleColor::Green
        } else {
            CandleColor::Red
        }
    }

    #[inline]
    pub fn color_matches(&self, &other: &Self) -> bool {
        self.color() == other.color()
    }

    #[inline]
    pub fn is_green(&self) -> bool {
        OrderedFloat(self.close) >= OrderedFloat(self.open)
    }

    #[inline]
    pub fn is_red(&self) -> bool {
        OrderedFloat(self.close) < OrderedFloat(self.open)
    }

    #[inline]
    pub fn is_inside(&self, other: &Self) -> bool {
        self.low >= other.low && self.high <= other.high
    }

    #[inline]
    pub fn is_outside(&self, other: &Self) -> bool {
        self.low <= other.low && self.high >= other.high
    }

    #[inline]
    pub fn is_above(&self, other: &Self) -> bool {
        self.low >= other.high
    }

    #[inline]
    pub fn is_below(&self, other: &Self) -> bool {
        self.high <= other.low
    }

    #[inline]
    pub fn is_above_or_touching(&self, other: &Self) -> bool {
        self.low >= other.high || self.low == other.high
    }

    #[inline]
    pub fn is_below_or_touching(&self, other: &Self) -> bool {
        self.high <= other.low || self.high == other.low
    }

    #[inline]
    pub fn is_touching(&self, other: &Self) -> bool {
        self.low == other.high || self.high == other.low
    }

    #[inline]
    pub fn is_crossing(&self, other: &Self) -> bool {
        self.low < other.high && self.high > other.low
    }

    #[inline]
    pub fn is_crossing_or_touching(&self, other: &Self) -> bool {
        self.low <= other.high && self.high >= other.low
    }

    #[inline]
    pub fn is_above_or_crossing(&self, other: &Self) -> bool {
        self.low >= other.high
    }

    #[inline]
    pub fn is_below_or_crossing(&self, other: &Self) -> bool {
        self.high <= other.low
    }

    #[inline]
    pub fn duration_since_open_time(&self) -> Duration {
        Utc::now() - self.open_time
    }

    pub fn lower_shadow(&self) -> ValueType {
        self.open.min(self.close) - self.low
    }

    #[inline]
    pub fn upper_shadow(&self) -> ValueType {
        self.high - self.open.max(self.close)
    }

    #[inline]
    pub fn body_abs(&self) -> ValueType {
        (self.open - self.close).abs()
    }

    #[inline]
    pub fn body(&self) -> ValueType {
        self.open - self.close
    }

    #[inline]
    pub fn close_relative_to_body_position(&self) -> ValueType {
        let lower = self.open.min(self.close);
        let upper = self.open.max(self.close);
        calc_relative_position_between(self.close, lower, upper)
    }

    #[inline]
    pub fn close_relative_to_full_range_position(&self) -> ValueType {
        calc_relative_position_between(self.close, self.low, self.high)
    }
}

impl OHLCV for Candle {
    fn open(&self) -> ValueType {
        self.open
    }

    fn high(&self) -> ValueType {
        self.high
    }

    fn low(&self) -> ValueType {
        self.low
    }

    fn close(&self) -> ValueType {
        self.close
    }

    fn volume(&self) -> ValueType {
        self.volume
    }
}

impl From<&dyn OHLCV> for Candle {
    fn from(src: &dyn OHLCV) -> Self {
        Self::from(src)
    }
}

impl From<(ValueType, ValueType, ValueType, ValueType)> for Candle {
    fn from(value: (ValueType, ValueType, ValueType, ValueType)) -> Self {
        Self {
            symbol:                       Default::default(),
            timeframe:                    Default::default(),
            open_time:                    Default::default(),
            close_time:                   Default::default(),
            open:                         value.0,
            high:                         value.1,
            low:                          value.2,
            close:                        value.3,
            volume:                       ValueType::NAN,
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     false,
        }
    }
}

impl From<(ValueType, ValueType, ValueType, ValueType, ValueType)> for Candle {
    fn from(value: (ValueType, ValueType, ValueType, ValueType, ValueType)) -> Self {
        Self {
            symbol:                       Default::default(),
            timeframe:                    Default::default(),
            open_time:                    Default::default(),
            close_time:                   Default::default(),
            open:                         value.0,
            high:                         value.1,
            low:                          value.2,
            close:                        value.3,
            volume:                       value.4,
            number_of_trades:             0,
            quote_asset_volume:           0.0,
            taker_buy_quote_asset_volume: 0.0,
            taker_buy_base_asset_volume:  0.0,
            is_final:                     false,
        }
    }
}

impl PartialEq for Candle {
    fn eq(&self, other: &Self) -> bool {
        self.open.to_bits() == other.open.to_bits()
            && self.high.to_bits() == other.high.to_bits()
            && self.low.to_bits() == other.low.to_bits()
            && self.close.to_bits() == other.close.to_bits()
            && self.volume.to_bits() == other.volume.to_bits()
    }
}

impl Eq for Candle {}

#[cfg(test)]
mod tests {
    use super::*;
}
