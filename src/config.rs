use crate::timeframe::Timeframe;
use anyhow::Result;
use config::{Config as Settings, File, FileFormat, Map, Value, *};
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize, Serializer};
use std::{
    collections::HashMap,
    fmt::{Debug, Display, Formatter},
};
use yata::core::{PeriodType, ValueType};

use chrono::Duration;
use ndarray::AssignElem;
use serde::{
    de::{self, Deserializer, SeqAccess, Unexpected, Visitor},
    ser::SerializeStruct,
};
use std::{fmt, ops::DerefMut};
use ustr::Ustr;

use crate::timeset::TimeSet;

use crate::{f, types::OrderedValueType};
/// Zeroize is for securely deleting data from memory
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Main configuration structure
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MainConfig {
    pub quote_asset:       Ustr,
    pub series_capacity:   usize,
    pub trade_window_size: usize,
}

impl Default for MainConfig {
    fn default() -> Self {
        Self {
            quote_asset:       "USDT".into(),
            series_capacity:   1000,
            trade_window_size: 100,
        }
    }
}

/// Exchange configuration structure with sensitive fields
#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct ExchangeConfig {
    pub rest_endpoint:      String,
    pub websocket_endpoint: String,
    pub key:                String,
    pub secret:             String,
    pub restricted_key:     String,
    pub restricted_secret:  String,
    pub key_testnet:        Option<String>,
    pub secret_testnet:     Option<String>,
}

/// Debug trait implementation that hides sensitive info
impl Debug for ExchangeConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "SECRET")
    }
}

/// Database configuration structure with sensitive fields
#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
pub struct DatabaseConfig {
    pub hostname: String,
    pub username: String,
    pub password: String,
    pub database: String,
    pub port:     i16,
}

/// Debug trait implementation that hides sensitive info
impl Debug for DatabaseConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "SECRET")
    }
}

// ----------------------------------------------------------------------------
// Structure defining timeframe specific configuration
// ----------------------------------------------------------------------------
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default)]
pub struct TimeframeConfig {
    pub timeframe:                 Timeframe,
    pub recent_candles_only:       Option<bool>,
    pub initial_number_of_candles: Option<usize>,
    pub download_days:             Option<usize>,
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<TimeSet<TimeframeConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    struct TimeframeVisitor;

    impl<'de> Visitor<'de> for TimeframeVisitor {
        type Value = TimeSet<TimeframeConfig>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("sequence of timeframe configurations")
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: SeqAccess<'de>,
        {
            let mut timeframes = TimeSet::new();
            while let Some(timeframe_map) = seq.next_element::<Map<String, Value>>()? {
                let timeframe: Timeframe = timeframe_map
                    .get("timeframe")
                    .ok_or_else(|| de::Error::missing_field("timeframe"))?
                    .clone()
                    .into_string()
                    .map_err(|e| de::Error::custom(format!("invalid timeframe: {}", e)))?
                    .as_str()
                    .parse()
                    .map_err(de::Error::custom)?;

                let recent_candles_only: Option<bool> = timeframe_map.get("recent_candles_only").and_then(|v| v.clone().into_bool().ok());
                let initial_number_of_candles = timeframe_map
                    .get("initial_number_of_candles")
                    .and_then(|v| v.clone().into_uint().ok().map(|x| x as usize));
                let download_days: Option<usize> = timeframe_map.get("download_days").and_then(|v| v.clone().into_uint().ok().map(|x| x as usize));

                timeframes.set(
                    timeframe,
                    TimeframeConfig {
                        timeframe,
                        recent_candles_only,
                        initial_number_of_candles,
                        download_days,
                    },
                );
            }

            Ok(timeframes)
        }
    }

    deserializer.deserialize_seq(TimeframeVisitor)
}

// ----------------------------------------------------------------------------
// ============================================================================
// ----------------------------------------------------------------------------

/// Coin specific configuration structure
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CoinConfig {
    pub name:                        Ustr,
    pub is_enabled:                  bool,
    pub is_favorite:                 Option<bool>,
    pub timeframes:                  Option<Vec<Timeframe>>,
    pub trade_timeframe:             Option<Timeframe>,
    pub fetch_candles_from_api_only: Option<bool>,
    pub entry_cooldown_ms:           Option<usize>,
    pub exit_cooldown_ms:            Option<usize>,
    pub min_candles:                 Option<usize>,
    pub periods:                     Option<Vec<PeriodType>>,
    pub max_quote_allocated:         Option<OrderedValueType>,
    pub min_quote_per_grid:          Option<OrderedValueType>,
    pub max_quote_per_grid:          Option<OrderedValueType>,
    pub max_orders:                  Option<usize>,
    pub max_orders_per_trade:        Option<usize>,
    pub max_orders_per_cooldown:     Option<usize>,
    pub max_retry_attempts:          Option<usize>,
    pub retry_delay_ms:              Option<usize>,
    pub entry_grid_orders:           Option<usize>,
    pub exit_grid_orders:            Option<usize>,
    pub buy_fee_ratio:               Option<OrderedValueType>,
    pub sell_fee_ratio:              Option<OrderedValueType>,
    pub min_order_spacing_ratio:     Option<OrderedValueType>,
    pub initial_stop_ratio:          Option<OrderedValueType>,
    pub stop_limit_offset_ratio:     Option<OrderedValueType>,
    pub initial_trailing_step_ratio: Option<OrderedValueType>,
    pub trailing_step_ratio:         Option<OrderedValueType>,
    pub baseline_risk_ratio:         Option<OrderedValueType>,
    pub is_zero_maker_fee:           Option<bool>,
    pub is_zero_taker_fee:           Option<bool>,
    pub is_iceberg_trading_enabled:  Option<bool>,
    pub is_long_trading_enabled:     Option<bool>,
    pub is_short_trading_enabled:    Option<bool>,
}

impl CoinConfig {
    pub fn dummy() -> Self {
        Self {
            name:                        "DUMMY".into(),
            is_enabled:                  true,
            is_favorite:                 Some(true),
            timeframes:                  Some(vec![Timeframe::M1, Timeframe::M5, Timeframe::H1]),
            trade_timeframe:             Some(Timeframe::M1),
            fetch_candles_from_api_only: Some(true),
            entry_cooldown_ms:           Some(3000),
            exit_cooldown_ms:            Some(3000),
            min_candles:                 Some(10946),
            periods:                     Some(vec![3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946]),
            max_quote_allocated:         Some(f!(1000.0)),
            min_quote_per_grid:          Some(f!(20.0)),
            max_quote_per_grid:          Some(f!(100.0)),
            max_orders:                  Some(100),
            max_orders_per_trade:        Some(10),
            max_orders_per_cooldown:     Some(10),
            max_retry_attempts:          Some(2),
            retry_delay_ms:              Some(300),
            entry_grid_orders:           Some(5),
            exit_grid_orders:            Some(5),
            buy_fee_ratio:               Some(f!(0.001)),
            sell_fee_ratio:              Some(f!(0.001)),
            min_order_spacing_ratio:     Some(f!(0.002)),
            initial_stop_ratio:          Some(f!(0.01)),
            stop_limit_offset_ratio:     Some(f!(0.001)),
            initial_trailing_step_ratio: Some(f!(0.01)),
            trailing_step_ratio:         Some(f!(0.02)),
            baseline_risk_ratio:         Some(f!(0.02)),
            is_zero_maker_fee:           Some(false),
            is_zero_taker_fee:           Some(false),
            is_iceberg_trading_enabled:  Some(true),
            is_long_trading_enabled:     Some(false),
            is_short_trading_enabled:    Some(false),
        }
    }
}

/// Configuration defaults
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigDefaults {
    pub timeframes:                  Vec<Timeframe>,
    pub trade_timeframe:             Timeframe,
    pub recent_candles_only:         bool,
    pub initial_number_of_candles:   usize,
    pub entry_cooldown_ms:           usize,
    pub exit_cooldown_ms:            usize,
    pub periods:                     Vec<PeriodType>,
    pub max_quote_allocated:         OrderedValueType,
    pub max_orders:                  usize,
    pub max_orders_per_trade:        usize,
    pub max_orders_per_cooldown:     usize,
    pub max_retry_attempts:          usize,
    pub retry_delay_ms:              usize,
    pub entry_grid_orders:           usize,
    pub exit_grid_orders:            usize,
    pub min_quote_per_grid:          OrderedValueType,
    pub max_quote_per_grid:          OrderedValueType,
    pub min_order_spacing_ratio:     OrderedValueType,
    pub initial_stop_ratio:          OrderedValueType,
    pub stop_limit_offset_ratio:     OrderedValueType,
    pub initial_trailing_step_ratio: OrderedValueType,
    pub trailing_step_ratio:         OrderedValueType,
    pub trailing_use_volatility:     bool,
    pub baseline_risk_ratio:         OrderedValueType,
    pub is_zero_maker_fee:           bool,
    pub is_zero_taker_fee:           bool,
    pub is_iceberg_trading_enabled:  bool,
    pub is_long_trading_enabled:     bool,
    pub is_short_trading_enabled:    bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LoggingConfig {
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self { level: "info".to_string() }
    }
}

/// Top-level configuration structure
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub main:                      MainConfig,
    pub exchange:                  ExchangeConfig,
    pub database:                  DatabaseConfig,
    pub rpc:                       RpcConfig,
    pub logging:                   LoggingConfig,
    pub defaults:                  ConfigDefaults,
    pub default_timeframe_configs: Vec<TimeframeConfig>,
    pub coins:                     Vec<CoinConfig>,
}

impl Config {
    pub fn get_coin(&self, name: &str) -> Option<CoinConfig> {
        self.coins.iter().find(|c| c.name == name).cloned()
    }

    pub fn get_default_timeframe_config(&self, timeframe: Timeframe) -> Option<TimeframeConfig> {
        self.default_timeframe_configs.iter().find(|c| c.timeframe == timeframe).cloned()
    }

    pub fn from_file_with_env(env_prefix: &str, config_path: &str) -> Result<Config, ConfigError> {
        let mut config = config::Config::builder()
            .add_source(config::File::with_name(config_path))
            .add_source(config::Environment::with_prefix(env_prefix));

        if std::env::var("BABYLON_TEST").is_ok() {
            config = config.add_source(config::File::with_name("config.test.toml"));
        }

        config.build().expect("failed to initialize config").try_deserialize()
    }

    pub fn from_env() -> Result<Config, ConfigError> {
        config::Config::builder()
            .add_source(config::Environment::with_prefix("BABYLON"))
            .build()
            .expect("failed to initialize config")
            .try_deserialize()
    }

    pub fn save(&self, config_path: &str) -> Result<(), ConfigError> {
        let toml = toml::to_string(&self).unwrap();
        std::fs::write(config_path, toml).unwrap();
        Ok(())
    }
}

impl Serialize for Config {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Config", 8)?;
        s.serialize_field("main", &self.main)?;
        s.serialize_field("exchange", &self.exchange)?;
        s.serialize_field("database", &self.database)?;
        s.serialize_field("rpc", &self.rpc)?;
        s.serialize_field("logging", &self.logging)?;
        s.serialize_field("defaults", &self.defaults)?;
        s.serialize_field("default_timeframe_configs", &self.default_timeframe_configs)?;
        s.serialize_field("coins", &self.coins)?;
        s.end()
    }
}

/// Configuration for the RPC service
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RpcConfig {
    pub stream_websocket_address:         String,
    pub rpc_http_address:                 String,
    pub rpc_websocket_address:            String,
    pub max_websocket_connections:        usize,
    pub max_subscriptions_per_connection: usize,
    pub require_authentication:           bool,
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            stream_websocket_address:         "127.0.0.1:3032".to_string(),
            rpc_http_address:                 "127.0.0.1:3033".to_string(),
            rpc_websocket_address:            "127.0.0.1:3034".to_string(),
            max_websocket_connections:        100,
            max_subscriptions_per_connection: 10,
            require_authentication:           false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use std::fs;

    #[test]
    fn test_coin_config() {
        let config = Config::from_file_with_env("BABYLON", "config.toml");

        println!("{:#?}", config);
    }

    #[test]
    fn test_add_and_delete_coin() {
        // Load the existing configuration
        let data = fs::read_to_string("config.toml").unwrap();
        let mut config: toml::Value = toml::from_str(&data).unwrap();

        // Add a new coin
        let mut new_coin = toml::map::Map::new();
        new_coin.insert("name".to_string(), toml::Value::String("NEWCOIN".to_string()));
        new_coin.insert("is_enabled".to_string(), toml::Value::Boolean(true));
        new_coin.insert("is_favorite".to_string(), toml::Value::Boolean(false));
        new_coin.insert("timeframes".to_string(), toml::Value::Array(vec![toml::Value::String("1m".to_string())]));

        let coins = config.get_mut("coins").unwrap().as_array_mut().unwrap();
        coins.push(toml::Value::Table(new_coin));

        println!("{:#?}", config);

        // Write the modified configuration back to the file
        let toml = toml::to_string(&config).unwrap();

        println!("{:#?}", toml);

        fs::write("config.test.toml", toml).unwrap();

        /*
        // Reload the configuration and check that the new coin was added
        let data = fs::read_to_string("config.toml").unwrap();
        let mut config: toml::Value = toml::from_str(&data).unwrap();
        let coins = config.get("coins").unwrap().as_array().unwrap();
        assert!(coins.iter().any(|coin| coin.get("name").unwrap().as_str().unwrap() == "NEWCOIN"));

        // Delete the coin
        let coins = config.get_mut("coins").unwrap().as_array_mut().unwrap();
        let index = coins.iter().position(|coin| coin.get("name").unwrap().as_str().unwrap() == "NEWCOIN").unwrap();
        coins.remove(index);

        // Write the modified configuration back to the file
        let toml = toml::to_string(&config).unwrap();
        // fs::write("config.toml", toml).unwrap();

        // Reload the configuration and check that the coin was deleted
        let data = fs::read_to_string("config.toml").unwrap();
        let config: toml::Value = toml::from_str(&data).unwrap();
        let coins = config.get("coins").unwrap().as_array().unwrap();
        assert!(!coins.iter().any(|coin| coin.get("name").unwrap().as_str().unwrap() == "NEWCOIN"));
         */
    }
}
