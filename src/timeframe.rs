use binance_spot_connector_rust::market::klines::KlineInterval;
use chrono::{Datelike, Duration, Utc};
use itertools::Itertools;
use std::{
    array::IntoIter,
    borrow::{Borrow, BorrowMut},
    collections::hash_map::Entry,
    convert::Into,
    fmt::Display,
    ops::Index,
};

use nohash_hasher::IntMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::util::days_in_month;
use thiserror::Error;
use ustr::Ustr;

#[derive(Error, Debug)]
pub enum TimeframeError {
    #[error("Timeframe not found: {0:?}")]
    TimeframeNotFound(Timeframe),

    #[error("Timeframe has no value: {0:?}")]
    TimeframeNoValue(Timeframe),

    #[error("Timeframe already exists: {0:?}")]
    TimeframeAlreadyExists(Timeframe),

    #[error("Invalid timeframe: {0:?}")]
    InvalidTimeframe(String),

    #[error("Unsupported custom timeframe: {0:?}")]
    UnsupportedCustomTimeframe(Timeframe),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Copy, Clone, Debug, PartialOrd, PartialEq, Hash, Eq, Ord, Default)]
#[repr(u64)]
pub enum Timeframe {
    #[default]
    NA = 0,
    MS100 = 100,
    MS250 = 250,
    MS500 = 500,
    S1 = 1_000,
    S3 = 3_000,
    S5 = 5_000,
    S15 = 15_000,
    S30 = 30_000,
    M1 = 60_000,
    M3 = 180_000,
    M5 = 300_000,
    M15 = 900_000,
    M30 = 1_800_000,
    H1 = 3_600_000,
    H2 = 7_200_000,
    H4 = 14_400_000,
    H6 = 21_600_000,
    H8 = 28_800_000,
    H12 = 43_200_000,
    D1 = 86_400_000,
    D3 = 259_200_000,
    W1 = 604_800_000,
    MM1 = 2_592_000_000,
}

impl TryFrom<Timeframe> for KlineInterval {
    type Error = TimeframeError;

    fn try_from(timeframe: Timeframe) -> Result<Self, Self::Error> {
        match timeframe {
            Timeframe::NA => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::MS100 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::MS250 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::MS500 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::S1 => Ok(KlineInterval::Seconds1),
            Timeframe::S3 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::S5 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::S15 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::S30 => Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)),
            Timeframe::M1 => Ok(KlineInterval::Minutes1),
            Timeframe::M3 => Ok(KlineInterval::Minutes3),
            Timeframe::M5 => Ok(KlineInterval::Minutes5),
            Timeframe::M15 => Ok(KlineInterval::Minutes15),
            Timeframe::M30 => Ok(KlineInterval::Minutes30),
            Timeframe::H1 => Ok(KlineInterval::Hours1),
            Timeframe::H2 => Ok(KlineInterval::Hours2),
            Timeframe::H4 => Ok(KlineInterval::Hours4),
            Timeframe::H6 => Ok(KlineInterval::Hours6),
            Timeframe::H8 => Ok(KlineInterval::Hours8),
            Timeframe::H12 => Ok(KlineInterval::Hours12),
            Timeframe::D1 => Ok(KlineInterval::Days1),
            Timeframe::D3 => Ok(KlineInterval::Days3),
            Timeframe::W1 => Ok(KlineInterval::Weeks1),
            Timeframe::MM1 => Ok(KlineInterval::Months1),
        }
    }
}

impl Index<u64> for Timeframe {
    type Output = Self;

    fn index(&self, index: u64) -> &Self::Output {
        match index {
            1 => &Self::S1,
            60 => &Self::M1,
            180 => &Self::M3,
            300 => &Self::M5,
            900 => &Self::M15,
            1800 => &Self::M30,
            3600 => &Self::H1,
            7200 => &Self::H2,
            14400 => &Self::H4,
            21600 => &Self::H6,
            28800 => &Self::H8,
            43200 => &Self::H12,
            86400 => &Self::D1,
            259200 => &Self::D3,
            604800 => &Self::W1,
            2592000 => &Self::MM1,
            _ => &Self::NA,
        }
    }
}

impl nohash_hasher::IsEnabled for Timeframe {}

impl std::str::FromStr for Timeframe {
    type Err = TimeframeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1s" => Ok(Self::S1),
            "1m" => Ok(Self::M1),
            "3m" => Ok(Self::M3),
            "5m" => Ok(Self::M5),
            "15m" => Ok(Self::M15),
            "30m" => Ok(Self::M30),
            "1h" => Ok(Self::H1),
            "2h" => Ok(Self::H2),
            "4h" => Ok(Self::H4),
            "6h" => Ok(Self::H6),
            "8h" => Ok(Self::H8),
            "12h" => Ok(Self::H12),
            "1d" => Ok(Self::D1),
            "3d" => Ok(Self::D3),
            "1w" => Ok(Self::W1),
            "1M" => Ok(Self::MM1),
            _ => Err(TimeframeError::InvalidTimeframe(s.to_string())),
        }
    }
}

impl Serialize for Timeframe {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match *self {
            Self::S1 => "1s",
            Self::M1 => "1m",
            Self::M3 => "3m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H2 => "2h",
            Self::H4 => "4h",
            Self::H6 => "6h",
            Self::H8 => "8h",
            Self::H12 => "12h",
            Self::D1 => "1d",
            Self::D3 => "3d",
            Self::W1 => "1w",
            Self::MM1 => "1M",
            _ => "NA",
        })
    }
}

impl<'de> Deserialize<'de> for Timeframe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "1s" => Self::S1,
            "1m" => Self::M1,
            "3m" => Self::M3,
            "5m" => Self::M5,
            "15m" => Self::M15,
            "30m" => Self::M30,
            "1h" => Self::H1,
            "2h" => Self::H2,
            "4h" => Self::H4,
            "6h" => Self::H6,
            "8h" => Self::H8,
            "12h" => Self::H12,
            "1d" => Self::D1,
            "3d" => Self::D3,
            "1w" => Self::W1,
            "1M" => Self::MM1,
            _ => Self::NA,
        })
    }
}

impl Display for Timeframe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            Self::S1 => "1s",
            Self::M1 => "1m",
            Self::M3 => "3m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H2 => "2h",
            Self::H4 => "4h",
            Self::H6 => "6h",
            Self::H8 => "8h",
            Self::H12 => "12h",
            Self::D1 => "1d",
            Self::D3 => "3d",
            Self::W1 => "1w",
            Self::MM1 => "1M",
            _ => "NA",
        };

        write!(f, "{}", str)
    }
}

impl From<String> for Timeframe {
    fn from(value: String) -> Self {
        match value.as_str() {
            "100ms" => Self::MS100,
            "250ms" => Self::MS250,
            "500ms" => Self::MS500,
            "1s" => Self::S1,
            "3s" => Self::S3,
            "5s" => Self::S5,
            "15s" => Self::S15,
            "30s" => Self::S30,
            "1m" => Self::M1,
            "3m" => Self::M3,
            "5m" => Self::M5,
            "15m" => Self::M15,
            "30m" => Self::M30,
            "1h" => Self::H1,
            "2h" => Self::H2,
            "4h" => Self::H4,
            "6h" => Self::H6,
            "8h" => Self::H8,
            "12h" => Self::H12,
            "1d" => Self::D1,
            "3d" => Self::D3,
            "1w" => Self::W1,
            "1M" => Self::MM1,
            _ => panic!("unrecognized timeframe: {}", value),
        }
    }
}

impl Timeframe {
    // The minimum time offset between the close time of a candle and the open time of the next candle.
    const CLOSE_TIME_OFFSET: i64 = 1000; // 1 second for second timeframes (in milliseconds).
    const CLOSE_TIME_OFFSET_SUBSEC: i64 = 1; // 1 millisecond for sub-second timeframes.

    #[inline]
    pub fn duration(&self) -> Duration {
        match self {
            Timeframe::MS100 => Duration::milliseconds(100),
            Timeframe::MS250 => Duration::milliseconds(250),
            Timeframe::MS500 => Duration::milliseconds(500),
            Timeframe::S3 => Duration::seconds(3),
            Timeframe::S5 => Duration::seconds(5),
            Timeframe::S15 => Duration::seconds(15),
            Timeframe::S30 => Duration::seconds(30),
            Timeframe::S1 => Duration::seconds(1),
            Timeframe::M1 => Duration::minutes(1),
            Timeframe::M3 => Duration::minutes(3),
            Timeframe::M5 => Duration::minutes(5),
            Timeframe::M15 => Duration::minutes(15),
            Timeframe::M30 => Duration::minutes(30),
            Timeframe::H1 => Duration::hours(1),
            Timeframe::H2 => Duration::hours(2),
            Timeframe::H4 => Duration::hours(4),
            Timeframe::H6 => Duration::hours(6),
            Timeframe::H8 => Duration::hours(8),
            Timeframe::H12 => Duration::hours(12),
            Timeframe::D1 => Duration::days(1),
            Timeframe::D3 => Duration::days(3),
            Timeframe::W1 => Duration::weeks(1),
            Timeframe::MM1 => {
                let now = Utc::now();
                Duration::days(days_in_month(now.year(), now.month()) as i64)
            }
            Timeframe::NA => Duration::zero(),
        }
    }

    #[inline]
    pub fn close_time_offset(&self) -> Duration {
        // 1 microsecond for sub-second timeframes.
        if *self < Timeframe::S1 {
            return Duration::microseconds(1);
        }

        // Default time offset is 1 millisecond.
        Duration::milliseconds(1)
    }

    pub fn supported_timeframes() -> [Self; 23] {
        [
            Self::MS100,
            Self::MS250,
            Self::MS500,
            Self::S1,
            Self::S3,
            Self::S5,
            Self::S15,
            Self::S30,
            Self::M1,
            Self::M3,
            Self::M5,
            Self::M15,
            Self::M30,
            Self::H1,
            Self::H2,
            Self::H4,
            Self::H6,
            Self::H8,
            Self::H12,
            Self::D1,
            Self::D3,
            Self::W1,
            Self::MM1,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::S1 => "1s",
            Self::M1 => "1m",
            Self::M3 => "3m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M30 => "30m",
            Self::H1 => "1h",
            Self::H2 => "2h",
            Self::H4 => "4h",
            Self::H6 => "6h",
            Self::H8 => "8h",
            Self::H12 => "12h",
            Self::D1 => "1d",
            Self::D3 => "3d",
            Self::W1 => "1w",
            Self::MM1 => "1M",
            _ => "NA",
        }
    }
}
