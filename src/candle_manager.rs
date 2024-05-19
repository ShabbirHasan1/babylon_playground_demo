use crate::{
    candle::Candle,
    config::CoinConfig,
    database::{Database, DatabaseError},
    timeframe::Timeframe,
    timeset::TimeSet,
};
use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use itertools::Itertools;
use std::{cmp::min, sync::Arc};
use yata::core::{PeriodType, ValueType, Window};

use crate::{
    config::{ConfigDefaults, TimeframeConfig},
    database::DatabaseBackend,
    ratelimiter::UnifiedRateLimiter,
    types::{ExchangeApi, ExchangeApiError},
    util::{days_in_month, format_date_ym, format_date_ymd, format_date_ymd_hms, s2f, timeframe_configs_to_timeset},
};
use binance_spot_connector_rust::{
    market::klines::Klines,
    ureq::{Error, Response},
};
use csv::ReaderBuilder;
use log::{error, info, warn};
use std::{
    io::{Cursor, Read},
    ops::{Add, Sub},
    thread::sleep,
    time::Instant,
};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use ustr::Ustr;
use zip::read::ZipArchive;

#[derive(Error, Debug)]
pub enum CandleManagerError {
    #[error("Backup not found: {date:?} {timeframe:?} {symbol:?}")]
    BackupNotFound {
        symbol:    Ustr,
        timeframe: Timeframe,
        date:      CandleBackupDate,
    },

    #[error("No more candles in the backup: {date:?} {timeframe:?} {symbol:?}")]
    BackupNoMoreCandles {
        symbol:    Ustr,
        timeframe: Timeframe,
        date:      CandleBackupDate,
    },

    #[error("No timeframes found: {symbol:?}")]
    NoTimeframes { symbol: Ustr },

    #[error("No candles in the local storage (download from exchange backup first)")]
    EmptyLocalHistory { symbol: Ustr, timeframe: Timeframe },

    #[error("Error: {0}")]
    Error(String),

    #[error("Exchange error: {0:#?}")]
    ExchangeError(String),

    #[error(transparent)]
    DatabaseError(#[from] DatabaseError),

    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),

    #[error(transparent)]
    ExchangeApiError(#[from] ExchangeApiError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CandleBackupDate {
    Day { year: i32, month: u32, day: u32 },
    Month { year: i32, month: u32 },
}

impl CandleBackupDate {
    pub fn day_from_date(date: NaiveDate) -> Self {
        Self::Day {
            year:  date.year(),
            month: date.month(),
            day:   date.day(),
        }
    }

    pub fn month_from_date(date: NaiveDate) -> Self {
        Self::Month {
            year:  date.year(),
            month: date.month(),
        }
    }

    pub fn to_date(&self) -> NaiveDate {
        match self {
            CandleBackupDate::Day { year, month, day } =>
                NaiveDate::from_ymd_opt(*year, *month, *day).expect("Failed to convert CandleBackupDate::Day to date"),
            CandleBackupDate::Month { year, month } => NaiveDate::from_ymd_opt(*year, *month, 1).expect("Failed to convert CandleBackupDate::Month to date"),
        }
    }

    pub fn to_formatted_string(&self) -> String {
        match self {
            CandleBackupDate::Day { year, month, day } => format_date_ymd(self.to_date()),
            CandleBackupDate::Month { year, month } => format_date_ym(self.to_date()),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CandleManagerState {
    Empty,
    Actual,
    Outdated {
        latest_candle_open_time_utc: DateTime<Utc>,
        time_passed:                 Duration,
    },
}

#[derive(Clone)]
pub struct CandleManager {
    coin_config:       CoinConfig,
    config_defaults:   ConfigDefaults,
    timeframe_configs: TimeSet<TimeframeConfig>,
    exchange_api:      ExchangeApi,
    db:                Database,
    ratelimiter:       UnifiedRateLimiter,
}

impl CandleManager {
    pub const NUM_WORKER_THREADS: usize = 4;
    pub const CHUNK_SIZE: usize = 1000;
    pub const SLEEP_ON_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    pub const SLEEP_ON_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(5);
    pub const DOWNLOAD_DAYS_DEFAULT: usize = 937;

    pub fn new(
        coin_config: CoinConfig,
        config_defaults: ConfigDefaults,
        timeframe_configs: TimeSet<TimeframeConfig>,
        db: Database,
        exchange_api: ExchangeApi,
        ratelimiter: UnifiedRateLimiter,
    ) -> Self {
        Self {
            coin_config,
            config_defaults,
            timeframe_configs,
            exchange_api,
            db,
            ratelimiter,
        }
    }

    pub fn symbol(&self) -> &str {
        &self.coin_config.name
    }

    pub fn count_candles(&mut self, timeframe: Timeframe) -> Result<usize, CandleManagerError> {
        Ok(self.db.count_candles(self.coin_config.name, timeframe)?)
    }

    pub fn count_candles_in_range(
        &mut self,
        timeframe: Timeframe,
        start_time: DateTime<Utc>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<usize, CandleManagerError> {
        Ok(self.db.count_candles_in_range(self.coin_config.name, timeframe, start_time, end_time)?)
    }

    pub fn oldest_candle(&mut self, timeframe: Timeframe) -> Result<Candle, CandleManagerError> {
        Ok(self.db.get_oldest_candle(self.coin_config.name, timeframe)?)
    }

    pub fn latest_candle(&mut self, timeframe: Timeframe) -> Result<Candle, CandleManagerError> {
        Ok(self.db.get_latest_candle(self.coin_config.name, timeframe)?)
    }

    pub fn get_recent_candles(&mut self, timeframe: Timeframe, num_candles: usize) -> Result<Vec<Candle>, CandleManagerError> {
        Ok(self.db.get_recent_candles(self.coin_config.name, timeframe, num_candles)?)
    }

    pub fn get_all_candles(&mut self, timeframe: Timeframe) -> Result<Vec<Candle>, CandleManagerError> {
        Ok(self.db.get_all_candles(self.coin_config.name, timeframe)?)
    }

    pub fn inspect(&mut self, timeframe: Timeframe) -> Result<CandleManagerState, CandleManagerError> {
        match self.db.get_latest_candle(self.coin_config.name, timeframe) {
            Ok(latest_candle) => {
                let now = Utc::now();
                let time_passed = now - latest_candle.open_time;

                if time_passed < timeframe.duration() {
                    return Ok(CandleManagerState::Actual);
                }

                Ok(CandleManagerState::Outdated {
                    latest_candle_open_time_utc: latest_candle.open_time,
                    time_passed,
                })
            }
            Err(DatabaseError::CandleNotFound) => Ok(CandleManagerState::Empty),
            Err(e) => Err(e.into()),
        }
    }

    pub fn load_timeframe_window(&mut self, timeframe: Timeframe, window_size: PeriodType) -> Result<(Candle, Window<Candle>), CandleManagerError> {
        match self.get_recent_candles(timeframe, window_size as usize) {
            Ok(candles) => {
                // IMPORTANT: deducting 1 from window size because the last candle is the head.
                let mut w = Window::new(window_size - 1, candles[0]);

                // The last candle is the head.
                let head = candles[candles.len() - 1].clone();

                for c in candles.iter().take(candles.len() - 1).skip(1) {
                    w.push(*c);
                }

                Ok((head, w))
            }
            Err(e) => {
                error!("failed to load candles for {} timeframe: {}", timeframe.to_string(), e);
                Err(e)
            }
        }
    }

    pub fn load_timeframe_vec(&mut self, timeframe: Timeframe, candle_count: usize) -> Result<Vec<Candle>, CandleManagerError> {
        match self.get_recent_candles(timeframe, candle_count) {
            Ok(mut candles) => Ok(candles),
            Err(e) => {
                error!("failed to load candles for {} timeframe: {}", timeframe.to_string(), e);
                Err(e)
            }
        }
    }

    pub fn load_timeframe_vec_head_tail(&mut self, timeframe: Timeframe, window_size: usize) -> Result<(Candle, Vec<Candle>), CandleManagerError> {
        match self.load_timeframe_vec(timeframe, window_size) {
            Ok(mut candles) => {
                // The last candle is the head.
                let head = candles.pop().expect("candles is empty");

                // NOTE: Now `candles` contains everything except for the head.
                Ok((head, candles))
            }
            Err(e) => {
                error!("failed to load candles for {} timeframe (head + tail): {}", timeframe.to_string(), e);
                Err(e)
            }
        }
    }

    pub fn plan_candle_download(&self, num_days: i64) -> Vec<CandleBackupDate> {
        let mut out = vec![];

        // Start with yesterday's date.
        let mut current_date = Utc::now().date_naive() - Duration::days(1);

        // Get the number of days remaining in the current month.
        let days_remaining_in_current_month = current_date.day();

        // Get the number of days to be downloaded daily.
        let num_daily = min(days_remaining_in_current_month as i64, num_days);

        // Get daily candles for the remaining days in the current month or remaining number of days, whichever is smaller.
        let daily_dates: Vec<NaiveDate> = (0..num_daily).map(|d| current_date - Duration::days(d)).collect();

        for date in daily_dates {
            out.push(CandleBackupDate::day_from_date(date));
        }

        // Calculate the remaining number of months to download.
        let num_months = (num_days - num_daily) / 30;
        let remaining_days = (num_days - num_daily) % 30;

        // Download monthly candles for the remaining months.
        for _ in 0..num_months {
            // Subtract a month from the current date.
            current_date = current_date.with_day(1).unwrap() - Duration::days(1);
            out.push(CandleBackupDate::month_from_date(current_date));
        }

        // Download remaining daily candles after all months have been downloaded.
        let daily_dates: Vec<NaiveDate> = (0..remaining_days).map(|d| current_date - Duration::days(d)).collect();

        for date in daily_dates {
            out.push(CandleBackupDate::day_from_date(date));
        }

        out
    }

    /*
    pub fn download_daily_candles(&self, symbol: Ustr, timeframe: Timeframe, date: NaiveDate) -> Result<Vec<Candle>, CandleManagerError> {
        let client = reqwest::Client::new();
        let baseurl = "https://data.binance.vision/data/spot/daily/klines";
        let mut out = vec![];

        let response = client
            .get(format!("{}/{}/{}/{}-{}-{}.zip", baseurl, symbol, timeframe.to_string(), symbol, timeframe.to_string(), date.format("%Y-%m-%d")).as_str())
            .send()?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CandleManagerError::BackupNotFound {
                symbol,
                timeframe,
                date: CandleBackupDate::day_from_date(date),
            });
        }

        if response.status().is_success() {
            let bytes = response.bytes()?;
            let reader = Cursor::new(bytes);
            let mut archive = ZipArchive::new(reader).unwrap();

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer).unwrap();

                let cursor = Cursor::new(buffer);
                let mut reader = ReaderBuilder::new().from_reader(cursor);

                for result in reader.records() {
                    let record = result.unwrap();
                    out.push(Candle {
                        symbol,
                        timeframe,
                        // open_time: NaiveDateTime::from_timestamp_opt(record[0].parse::<i64>().unwrap() / 1000, 0).unwrap(),
                        // close_time: NaiveDateTime::from_timestamp_opt(record[6].parse::<i64>().unwrap() / 1000, 0).unwrap(),
                        open_time: NaiveDateTime::from_timestamp_opt(record[0].parse::<i64>().unwrap(), 0).unwrap().and_utc(),
                        close_time: NaiveDateTime::from_timestamp_opt(record[6].parse::<i64>().unwrap(), 0).unwrap().and_utc(),
                        open: record[1].parse::<ValueType>().unwrap(),
                        high: record[2].parse::<ValueType>().unwrap(),
                        low: record[3].parse::<ValueType>().unwrap(),
                        close: record[4].parse::<ValueType>().unwrap(),
                        volume: record[5].parse::<ValueType>().unwrap(),
                        number_of_trades: record[8].parse::<i64>().unwrap(),
                        quote_asset_volume: record[7].parse::<ValueType>().unwrap(),
                        taker_buy_quote_asset_volume: record[9].parse::<ValueType>().unwrap(),
                        taker_buy_base_asset_volume: record[10].parse::<ValueType>().unwrap(),
                        is_final: true,
                    });
                }
            }
        }

        Ok(out)
    }

    // Function to download monthly candles
    pub fn download_monthly_candles(&self, symbol: Ustr, timeframe: Timeframe, date: NaiveDate) -> Result<Vec<Candle>, CandleManagerError> {
        let client = reqwest::Client::new();
        let baseurl = "https://data.binance.vision/data/spot/monthly/klines";
        let mut out = vec![];

        let response = client
            .get(format!("{}/{}/{}/{}-{}-{}.zip", baseurl, symbol, timeframe.to_string(), symbol, timeframe.to_string(), date.format("%Y-%m")).as_str())
            .send()?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CandleManagerError::BackupNotFound {
                symbol,
                timeframe,
                date: CandleBackupDate::month_from_date(date),
            });
        }

        if response.status().is_success() {
            let bytes = response.bytes()?;
            let reader = Cursor::new(bytes);

            let mut archive = match ZipArchive::new(reader) {
                Ok(archive) => archive,
                Err(err) => {
                    error!("{}: Failed to extract monthly archive for timeframe {}: {:#?}", symbol.as_str(), timeframe.as_str(), err);
                    return Err(CandleManagerError::Error(err.to_string()));
                }
            };

            for i in 0..archive.len() {
                let mut file = archive.by_index(i).unwrap();
                let mut buffer = Vec::new();
                file.read_to_end(&mut buffer).unwrap();

                let cursor = Cursor::new(buffer);
                let mut reader = ReaderBuilder::new().from_reader(cursor);

                for result in reader.records() {
                    let record = result.unwrap();
                    out.push(Candle {
                        symbol,
                        timeframe,
                        open_time: NaiveDateTime::from_timestamp_opt(record[0].parse::<i64>().unwrap() / 1000, 0)
                            .unwrap()
                            .and_utc(),
                        close_time: NaiveDateTime::from_timestamp_opt(record[6].parse::<i64>().unwrap() / 1000, 0)
                            .unwrap()
                            .and_utc(),
                        open: record[1].parse::<ValueType>().unwrap(),
                        high: record[2].parse::<ValueType>().unwrap(),
                        low: record[3].parse::<ValueType>().unwrap(),
                        close: record[4].parse::<ValueType>().unwrap(),
                        volume: record[5].parse::<ValueType>().unwrap(),
                        number_of_trades: record[8].parse::<i64>().unwrap(),
                        quote_asset_volume: record[7].parse::<ValueType>().unwrap(),
                        taker_buy_quote_asset_volume: record[9].parse::<ValueType>().unwrap(),
                        taker_buy_base_asset_volume: record[10].parse::<ValueType>().unwrap(),
                        is_final: true,
                    });
                }
            }
        }

        Ok(out)
    }
     */

    fn split_time_range(&self, start: DateTime<Utc>, end: DateTime<Utc>, interval_duration: Duration) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
        let mut ranges = Vec::new();
        let mut current_start = start;

        while current_start < end {
            let current_end = min(current_start + interval_duration, end);
            ranges.push((current_start, current_end));
            current_start = current_end;
        }

        ranges
    }

    pub fn fetch_initial(&mut self, num_candles: usize) -> Result<(), CandleManagerError> {
        // Check if the coin is known in the database, if not, create it
        if !self.db.is_coin_known(self.coin_config.name.clone())? {
            self.db.create_coin(self.coin_config.name.clone())?;
        }

        let timeframe_configs = self
            .timeframe_configs
            .iter()
            .map(|(timeframe, config)| (timeframe, config.clone()))
            .collect_vec();

        let timeframe_configs_with_counts = timeframe_configs
            .into_iter()
            .map(|(timeframe, config)| (timeframe, config, self.count_candles(timeframe).expect("Failed to count candles")))
            .collect_vec();

        // Iterate over all timeframes
        for (timeframe, timeframe_config, candle_count) in timeframe_configs_with_counts {
            // If there are no candles for the current timeframe, fetch them
            if candle_count == 0 {
                // Calculate the start time of the oldest candle we want to fetch
                let mut start_time = Utc::now() - timeframe.duration() * (num_candles as i32);

                if start_time.timestamp() < 0 {
                    start_time = Utc.timestamp_nanos(0);
                }

                // Fetch and store the candles
                self.fetch_and_store_candles_between(timeframe, start_time, None)?;
            }
        }

        Ok(())
    }

    pub fn fetch_updates(&mut self, timeframe: Timeframe) -> Result<(), CandleManagerError> {
        // Get the latest candle in the database
        let latest_db_candle = self.latest_candle(timeframe)?;

        // Calculate the end time for fetching candles
        // NOTE: We fetch candles for the next 2 intervals to make sure we have the latest data
        let end_time = Utc::now() + timeframe.duration() * 2;

        // Fetch and store the candles
        self.fetch_and_store_candles_between(timeframe, latest_db_candle.open_time, Some(end_time))?;

        Ok(())
    }

    pub fn fetch(&mut self, timeframe: Timeframe) -> Result<(), CandleManagerError> {
        let symbol = self.coin_config.name;

        let recent_candles_only = self
            .timeframe_configs
            .get(timeframe)
            .unwrap()
            .recent_candles_only
            .unwrap_or(self.config_defaults.recent_candles_only);

        let num_candles = self
            .timeframe_configs
            .get(timeframe)
            .unwrap()
            .initial_number_of_candles
            .unwrap_or(self.config_defaults.initial_number_of_candles);

        if recent_candles_only {
            self.fetch_initial(1000)?;
        } else {
            self.fetch_initial(num_candles)?;
            self.fetch_updates(timeframe)?;
        }

        Ok(())
    }

    fn fetch_and_store_candles_between(
        &mut self,
        timeframe: Timeframe,
        start_time: DateTime<Utc>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<Candle>, CandleManagerError> {
        let it = Instant::now();
        let symbol = self.coin_config.name;
        let mut out = vec![];

        let mut curr_start_time = Some(start_time);
        let mut candle_counter = 0;
        let mut new_candles_count = 0;
        let mut retries = 0;
        let max_retries = 3;

        info!(
            "{}: Fetching timeframe={:?} start={} and end={}",
            symbol,
            timeframe,
            format_date_ymd_hms(start_time.naive_utc()),
            end_time.map(|t| format_date_ymd_hms(t.naive_utc())).unwrap_or("now".to_string())
        );

        'outer: loop {
            let it = Instant::now();

            let new_candles = 'inner: loop {
                self.ratelimiter.wait_for_raw_request_limit(2);
                self.ratelimiter.wait_for_request_weight_limit(2);

                let mut candles = self
                    .exchange_api
                    .fetch_klines(symbol.as_str(), timeframe, curr_start_time, end_time, Some(Self::CHUNK_SIZE))?;

                if candles.is_empty() {
                    break candles;
                }

                if candles.last().unwrap().duration_since_open_time() < timeframe.duration() {
                    candles.last_mut().unwrap().is_final = false;
                }

                break candles;
            };

            /*
            if timeframe == Timeframe::S1 && new_candles.len() < 3 {
                warn!("{}: Not enough candles for S1 timeframe, stopping ...", symbol);
                break 'outer;
            }
             */

            if new_candles.is_empty() {
                break;
            }

            curr_start_time = Some(new_candles.last().unwrap().open_time);
            let mut last_candle = Candle::default();

            let it = Instant::now();

            for candle in new_candles {
                if let Some(end) = end_time {
                    if candle.open_time > end {
                        break 'outer;
                    }
                }

                if let Err(e) = self.db.upsert_candle(symbol, candle) {
                    panic!("{}: Failed to upsert candle: {:?}", symbol, e);
                }

                last_candle = candle;
                candle_counter += 1;
                new_candles_count += 1;
            }

            if last_candle.duration_since_open_time() < timeframe.duration() {
                break 'outer;
            }

            info!(
                "{}: n={} candles for {:?} in {:?}: open_time={} total_counter={} execution_time={:?}",
                symbol,
                Self::CHUNK_SIZE,
                timeframe,
                it.elapsed(),
                format_date_ymd_hms(last_candle.open_time.naive_utc()),
                candle_counter,
                it.elapsed()
            );

            // If we have fetched less than the chunk size, we are done
            if new_candles_count < Self::CHUNK_SIZE {
                break;
            }

            // Reset the new candles counter
            new_candles_count = 0;
        }

        info!("{}: Finished loading {} candles for {:?} in total {:?}", symbol, candle_counter, timeframe, it.elapsed());

        Ok(out)
    }

    /*
    fn fetch_and_store_recent_candles_from_api(&mut self, timeframe: Timeframe) -> Result<Vec<Candle>, CandleManagerError> {
        let it = Instant::now();
        let symbol = self.coin_config.name;
        let mut out = vec![];

        // Inspecting to determine the state of the local history
        let last_open_time = match self.inspect(timeframe) {
            Ok(state) => match state {
                CandleManagerState::Empty => {
                    warn!("{}: Local history is empty, please download backup from the exchange first", symbol);
                    return Err(CandleManagerError::EmptyLocalHistory { symbol, timeframe })?;
                }
                CandleManagerState::Actual => {
                    info!("{}: Local history is actual, no need to download candles", symbol);
                    return Ok(vec![]);
                }
                CandleManagerState::Outdated {
                    latest_candle_open_time_utc,
                    time_passed,
                } => {
                    info!("{}: Local history is outdated, loading candles from {}; chunk_size={}", symbol, latest_candle_open_time_utc, Self::CHUNK_SIZE);
                    latest_candle_open_time_utc
                }
            },

            Err(e) => {
                error!("{}: Error while inspecting candles: {:?}", symbol, e);
                return Err(e);
            }
        };

        let mut curr_start_time = last_open_time.timestamp();
        let mut candle_counter = 0;
        let mut retries = 0;
        let max_retries = 3;

        'outer: loop {
            // ----------------------------------------------------------------------------
            // Download candles from exchange
            // ----------------------------------------------------------------------------

            let new_candles = 'inner: loop {
                self.ratelimiter.wait_for_raw_request_limit(1);
                self.ratelimiter.wait_for_request_weight_limit(1);

                // ----------------------------------------------------------------------------
                // Downloading candles
                // ----------------------------------------------------------------------------

                let result = self
                    .exchange_api
                    .market
                    .get_klines(symbol.as_str(), timeframe.to_string(), Self::CHUNK_SIZE as u16, Some((curr_start_time * 1000) as u64), None)
                    .map(|AllKlineSummaries(klines)| klines);

                match result {
                    Ok(summary) =>
                        break summary
                            .into_iter()
                            .map(|kline| {
                                let mut candle = Candle {
                                    symbol,
                                    timeframe: Timeframe::from(timeframe.to_string()),
                                    open_time: NaiveDateTime::from_timestamp_opt(kline.open_time / 1000, 0).unwrap().and_utc(),
                                    close_time: NaiveDateTime::from_timestamp_opt(kline.close_time / 1000, 0).unwrap().and_utc(),
                                    open: s2f(&kline.open),
                                    high: s2f(&kline.high),
                                    low: s2f(&kline.low),
                                    close: s2f(&kline.close),
                                    volume: s2f(&kline.volume),
                                    number_of_trades: kline.number_of_trades,
                                    quote_asset_volume: s2f(&kline.quote_asset_volume),
                                    taker_buy_quote_asset_volume: s2f(&kline.taker_buy_quote_asset_volume),
                                    taker_buy_base_asset_volume: s2f(&kline.taker_buy_base_asset_volume),
                                    is_final: true,
                                };

                                // WARNING: setting `is_final` to false if the time passed since open_time is less than the timeframe duration
                                if Utc::now() - candle.open_time < timeframe.duration() {
                                    candle.is_final = false;
                                }

                                candle
                            })
                            .collect_vec(),
                    Err(e) => {
                        match e {
                            binance::errors::Error(kind, state) => match kind {
                                ErrorKind::ReqError(e) =>
                                    if e.is_timeout() && retries < max_retries {
                                        // WARNING: This is a hack to circumvent the timeout error when loading candles from the exchange API
                                        warn!(
                                            "{}: Timeout while loading {} candles from exchange API, retrying in 5 seconds",
                                            symbol.as_str(),
                                            timeframe.as_str()
                                        );
                                        retries += 1;
                                        sleep(Self::SLEEP_ON_TIMEOUT);
                                        continue 'inner;
                                    },

                                _ => {}
                            },

                            _ => {
                                error!("{}: failed to load {} candles from exchange API: {:?}", symbol.as_str(), timeframe.as_str(), e);
                                return Err(CandleManagerError::ExchangeError(e.to_string()))?;
                            }
                        }
                    }
                }
            };

            if new_candles.is_empty() {
                break;
            }

            curr_start_time = new_candles.last().unwrap().open_time.timestamp();
            let mut last_candle = Candle::default();

            for candle in new_candles {
                if let Err(e) = self.db.upsert_candle(symbol, candle) {
                    panic!("{}: Failed to upsert candle: {:?}", symbol.as_str(), e);
                }

                last_candle = candle;
                candle_counter += 1;
            }

            if last_candle.duration_since_open_time() < timeframe.duration() {
                break 'outer;
            }

            info!(
                "{}: Finished loading {} candles for {:?} in {:?}: open_time={} total_counter={}",
                symbol.as_str(),
                Self::CHUNK_SIZE,
                timeframe.as_str(),
                it.elapsed(),
                format_date_ymd_hms(last_candle.open_time.naive_utc()),
                candle_counter
            );
        }

        info!("{}: Finished loading {} candles for {:?} in total {:?}", symbol, candle_counter, timeframe, it.elapsed());

        Ok(out)
    }

    pub fn update(&mut self, timeframe: Timeframe) -> Result<(), CandleManagerError> {
        let symbol = self.coin_config.name;
        let coin_config = self.coin_config.clone();

        // IMPORTANT: Before everything!
        // We need to check if the coin is known in the database, because we might have
        // added a new coin to the config, but we haven't created it in the database yet.
        if !self.db.is_coin_known(symbol)? {
            self.db.create_coin(coin_config.name)?;
        }

        let state = self.inspect(timeframe)?;
        let local_timeframe_config = self.timeframe_configs.get(timeframe).unwrap().clone();
        let num_candles = local_timeframe_config
            .initial_number_of_candles
            .unwrap_or(self.config_defaults.initial_number_of_candles) as i64;
        let fetch_candles_from_api_only = local_timeframe_config
            .fetch_candles_from_api_only
            .unwrap_or(self.config_defaults.fetch_candles_from_api_only);
        let download_days = local_timeframe_config.download_days.unwrap_or(Self::DOWNLOAD_DAYS_DEFAULT) as i64;

        match state {
            // ----------------------------------------------------------------------------
            // We have no candles at all, so we need to download older candles first
            // from the exchange backup to the local database
            // ----------------------------------------------------------------------------
            CandleManagerState::Empty => {
                let it = Instant::now();
                let dates = self.plan_candle_download(download_days);
                let mut latest_open_time = NaiveDateTime::from_timestamp_opt(0, 0).unwrap().and_utc();

                for date in dates {
                    match date {
                        CandleBackupDate::Day { .. } => {
                            info!("{}: Downloading daily historical {} candles for {} ...", symbol, timeframe.to_string(), date.to_formatted_string());
                            let candles = match self.download_daily_candles(symbol, timeframe, date.to_date()) {
                                Ok(candles) => candles,
                                Err(CandleManagerError::BackupNotFound { .. }) => {
                                    let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
                                    let yesterday = today - Duration::days(1);

                                    // NOTE: allowing for a 3 day delay in the backup
                                    if date.to_date() <= yesterday.date() && date.to_date() >= (today.date() - Duration::days(3)) {
                                        // WARNING: CAUTION
                                        // This is a hack, but it's the best we can do for now, because the
                                        // missing day will be compensated from the API further down the line
                                        warn!(
                                            "{}: No {} backup found for {}, skipping for now and downloading what we can",
                                            symbol,
                                            timeframe.to_string(),
                                            date.to_formatted_string()
                                        );
                                        continue;
                                    } else {
                                        warn!("{}: No more {} backup found to download", symbol, timeframe.to_string());
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to download daily historical candles: {:?}", e);
                                    return Err(e);
                                }
                            };

                            for candle in candles {
                                latest_open_time = candle.open_time;

                                self.db.upsert_candle(symbol, candle).expect("Failed to upsert historical candle (daily)");
                            }
                        }

                        CandleBackupDate::Month { year, month } => {
                            info!("{}: Downloading monthly historical {} candles for {} ...", symbol, timeframe.to_string(), date.to_formatted_string());

                            let candles = match self.download_monthly_candles(symbol, timeframe, date.to_date()) {
                                Ok(candles) => candles,
                                Err(CandleManagerError::BackupNotFound { .. }) => {
                                    let this_month = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
                                    let last_month = this_month - Duration::days(days_in_month(this_month.year(), this_month.month()) as i64);

                                    if date == CandleBackupDate::month_from_date(last_month.date()) {
                                        warn!(
                                            "{}: No {} backup found for last month, skipping for now and downloading what we can",
                                            symbol,
                                            timeframe.to_string()
                                        );

                                        // TODO: possibly, if done this way, it will need proper checking that candle times connect seamlessly
                                        warn!("{}: Downloading previous month {} candles for {} (as individual days because the monthly backup is not yet available) ...", symbol, timeframe.to_string(), date.to_formatted_string());

                                        let mut compensated_candles = Vec::new();

                                        // Download last month's candles individually
                                        for day in 1..=days_in_month(last_month.year(), last_month.month()) {
                                            let date_in_last_month = NaiveDate::from_ymd_opt(last_month.year(), last_month.month(), day)
                                                .expect("Failed to create date in last month");

                                            match self.download_daily_candles(symbol.clone(), timeframe, date_in_last_month) {
                                                Ok(candles) => compensated_candles.extend(candles),
                                                Err(CandleManagerError::BackupNotFound { .. }) => {
                                                    warn!(
                                                        "{}: No more {} backup found to download (compensating month with daily backups)",
                                                        symbol,
                                                        timeframe.to_string()
                                                    );
                                                    break;
                                                }
                                                Err(e) => {
                                                    warn!("{}: Failed to download monthly historical {} candles: {:?}", symbol, timeframe.as_str(), e);
                                                    return Err(e);
                                                }
                                            };
                                        }

                                        compensated_candles
                                    } else {
                                        warn!("{}: No more {} backup found to download", symbol, timeframe.to_string());
                                        break;
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to download monthly historical candles: {:?}", e);
                                    return Err(e);
                                }
                            };

                            for candle in candles {
                                latest_open_time = candle.open_time;

                                self.db
                                    .upsert_candle(symbol.clone(), candle)
                                    .expect("Failed to upsert historical candle (monthly)");
                            }
                        }
                    }
                }

                info!("{}: Finished downloading {} candles from exchange backup in {:?}", symbol, timeframe.to_string(), it.elapsed());

                // Call update again to continue with the next step (see below)
                self.update(timeframe)
            }

            // ----------------------------------------------------------------------------
            // We have all historical candles, but we are missing recent candles
            // ----------------------------------------------------------------------------
            CandleManagerState::Outdated {
                latest_candle_open_time_utc,
                time_passed,
            } => {
                info!(
                    "{}: Missing {} recent candles; latest_candle_open_time_utc is {}",
                    symbol,
                    timeframe.to_string(),
                    format_date_ymd_hms(latest_candle_open_time_utc.naive_utc())
                );

                // If there is a considerable time gap since the last candle -- for instance, if it's older than today's date --
                // then downloading the remaining candles could mean dealing with a large amount of data.
                // To avoid this, we check if the 'latest_candle_open_time_utc' is older than today's date.
                // If so, we opt to download the required candles from the exchange backup.
                // If the last candle is not that old, we can simply fetch the remaining candles directly from the API.

                let now = Utc::now();
                let today = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();

                // If latest_candle_open_time_utc is older than today's date, then we need to download
                // from the exchange backup, otherwise we can just load the remaining candles from the exchange API
                // if latest_candle_open_time_utc < today {
                if latest_candle_open_time_utc < (now - timeframe.duration()) {
                    // Delete all candles if they are too old and download them again by calling update() again
                    if time_passed > Duration::days(download_days) {
                        // FIXME: Give this another thought for just in case, but I'm pretty sure this could harm more than it could help
                        /*
                        info!("{}: Deleting all {} candles for because they are way too old", self.symbol(), timeframe.to_string());
                        self.reset(timeframe)?;
                         */
                        warn!("{}: Before, we deleted all candles that were too old, but now we're just downloading the missing candles from the exchange backup instead", symbol.as_str());
                        return self.update(timeframe);
                    }

                    // We need to account for situations where we're required to download candles from
                    // the exchange backup, but the candles aren't available because they're too recent.
                    // For instance, just after midnight, the exchange backup might not yet have the
                    // candles for today. In these cases, we have two alternatives:
                    // 1. Wait until the exchange backup updates with the new candles before downloading them.
                    // 2. Load the candles directly from the API. However, this approach is suboptimal as it
                    // involves downloading all candles, resulting in high data usage and longer processing time.
                    // Nonetheless, it's our only recourse when the candles aren't available in the exchange backup.

                    // So, implementing option 2 above (load candles directly from the API) if the attempt to download
                    // yesterday's candles from the exchange backup fails. This will be a temporary solution until
                    // the exchange backup is updated with the missing candles. So, the logic must be as follows:
                    // 1. Try to download yesterday's candles from the exchange backup.
                    // 2. If the download fails, then download as planned up to day before yesterday.
                    // 3. Then download the remaining candles directly from the API.

                    // Determine how many days we're missing, then download them from the exchange backup.
                    let missing_days = self.plan_candle_download((today.and_utc() - latest_candle_open_time_utc).num_days());

                    if !missing_days.is_empty() {
                        info!(
                            "{}: Loading {:?} missing daily candles from exchange backup ...",
                            self.symbol(),
                            missing_days.iter().map(|d| d.to_formatted_string()).collect::<Vec<_>>()
                        );
                        for day in missing_days {
                            info!("{}: Downloading daily historical {} candles for {} ...", self.symbol(), timeframe.to_string(), day.to_formatted_string());
                            match self.download_daily_candles(symbol.clone(), timeframe, day.to_date()) {
                                Ok(candles) =>
                                    for candle in candles {
                                        self.db
                                            .upsert_candle(symbol.clone(), candle)
                                            .expect("Failed to upsert historical candle (daily)");
                                    },

                                Err(CandleManagerError::BackupNotFound { symbol, timeframe, date }) => {
                                    // ----------------------------------------------------------------------------
                                    // NOTE: Edge case, but it can happen if the exchange backup doesn't have yesterday's candles yet
                                    // ----------------------------------------------------------------------------
                                    let yesterday = today - Duration::days(1);
                                    let max_outdated_age = today.date() - Duration::days(3);

                                    if date.to_date() <= yesterday.date() && date.to_date() >= max_outdated_age {
                                        warn!(
                                            "{}: Backup for yesterday's {} candles not found, loading from exchange API (if not older than {})",
                                            symbol,
                                            timeframe.as_str(),
                                            max_outdated_age
                                        );
                                        warn!(
                                            "{}: Failed to load {} candles from exchange backup for {}: {}",
                                            symbol,
                                            timeframe.to_string(),
                                            date.to_date(),
                                            CandleManagerError::BackupNotFound {
                                                symbol: symbol.clone(),
                                                timeframe,
                                                date
                                            }
                                        );
                                        warn!("{}: Loading missing daily candles from exchange API instead ...", self.symbol());
                                        self.fetch_and_store_recent_candles_from_api(timeframe)
                                            .expect("Failed to load missing candles from exchange API (compensating for missing candles in exchange backup)");
                                    } else {
                                        // Otherwise the backup is missing candles for a day that is not yesterday (older)
                                        // Assuming that we've reached the end of the historical candles, we can safely ignore this error.
                                        // This could happen if the coin is quite new and the exchange backup doesn't have as many candles as we've requested yet.
                                        // WARNING: Calling update again to return to normal course
                                        warn!(
                                        "{}: Failed to load missing daily {} candles from exchange backup (possibly because there aren't any more in the backup): {}",
                                        self.symbol(),
                                        timeframe.to_string(),
                                        CandleManagerError::BackupNotFound {
                                            symbol: symbol.clone(),
                                            timeframe,
                                            date
                                        }
                                    );
                                        return self.update(timeframe);
                                    }
                                }

                                Err(e) => {
                                    error!("Failed to load missing daily candles from exchange backup: {}", e);
                                    return Err(e);
                                }
                            }
                        }
                    }

                    // Now calling update() again will load the remaining candles from the API.
                    // return self.update(timeframe);
                }

                let it = Instant::now();
                info!("{}: Loading remaining historical candles from the API ...", self.symbol());

                self.fetch_and_store_recent_candles_from_api(timeframe)
                    .expect("Failed to load and store recent candles from exchange");

                info!("{}: Finished loading recent candles from the API for {:?} in {:?}", self.symbol(), timeframe, it.elapsed());

                // Call update again to continue with the next step (see below).
                self.update(timeframe)
            }

            // ----------------------------------------------------------------------------
            // At this point we have all historical and recent candles in the database,
            // we're completely up to date and can start streaming candles from the exchange
            // ----------------------------------------------------------------------------
            CandleManagerState::Actual => {
                info!("{}: Candles for {:?} are up to date", self.symbol(), timeframe);
                Ok(())
            }
        }
    }
     */

    pub fn rollback(&mut self, symbol: Ustr, timeframe: Timeframe, period: Duration, from_most_recent: bool) -> Result<(), CandleManagerError> {
        let result = self
            .db
            .rollback_coin(self.coin_config.name.clone(), timeframe, period, from_most_recent)
            .map_err(|err| {
                error!("{}: Failed to rollback {} by {:?}: {}", symbol, timeframe.to_string(), period, err);
                CandleManagerError::DatabaseError(err)
            });

        warn!("{}: Rolled back {} by {:?} ...", self.symbol(), timeframe.to_string(), period);

        result
    }

    pub fn find_gaps(&mut self, timeframe: Timeframe) -> Result<Vec<(Timeframe, DateTime<Utc>, DateTime<Utc>)>, CandleManagerError> {
        let mut gaps = vec![];
        let candles = self.get_all_candles(timeframe)?;

        if candles.is_empty() {
            return Ok(gaps);
        }

        let mut prev_candle = candles[0].clone();
        for candle in candles.iter().skip(1) {
            if candle.open_time - prev_candle.open_time > timeframe.duration() {
                gaps.push((timeframe, prev_candle.open_time, candle.open_time));
            }

            prev_candle = candle.clone();
        }

        Ok(gaps)
    }

    pub fn fix_gaps(&mut self, timeframe: Timeframe) -> Result<(), CandleManagerError> {
        // ----------------------------------------------------------------------------
        // download candles from exchange API to fill gaps in the database (if any)
        // ----------------------------------------------------------------------------

        let it = Instant::now();
        let gaps = self.find_gaps(timeframe)?;

        if gaps.is_empty() {
            info!("{}: No gaps found for {:?} in the database", self.symbol(), timeframe);
            return Ok(());
        }

        info!("{}: Found {} gaps for {:?} in the database", self.symbol(), gaps.len(), timeframe);

        for (timeframe, start, end) in gaps {
            info!("{}: Downloading missing {} candles from {} to {} ...", self.symbol(), timeframe.to_string(), start, end);
            self.fetch_and_store_candles_between(timeframe, start.sub(timeframe.duration()), Some(end.add(timeframe.duration())))?;
        }

        info!("{}: Filled in missing candles for {:?} in {:?}", self.symbol(), timeframe, it.elapsed());

        Ok(())
    }

    pub fn reset(&mut self, timeframe: Timeframe) -> Result<(), CandleManagerError> {
        warn!("{}: Resetting {} candles ...", self.symbol(), timeframe.to_string());

        self.db.delete_candles(self.coin_config.name.clone(), timeframe).map_err(|e| {
            error!("Failed to delete {} candles: {}", timeframe.to_string(), e);
            CandleManagerError::DatabaseError(e)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        candle_manager::CandleManager, config::Config, database::Database, timeframe::Timeframe, types::ExchangeApi, util::timeframe_configs_to_timeset,
    };

    #[tokio::test]
    async fn test_candle_manager() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::S1;
        let exchange_api = ExchangeApi::default();
        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        // inspect BTCUSDT for timeframe and print the result (format `time_passed` as 1d 2h 3m 4s)
        let state = cm.inspect(timeframe).unwrap();
        match state {
            CandleManagerState::Actual => println!("actual"),
            CandleManagerState::Outdated {
                latest_candle_open_time_utc,
                time_passed,
            } => {
                // format time_passed as 1d 2h 3m 4s
                let mut time_passed = time_passed;
                let days = time_passed.num_days();
                time_passed = time_passed - Duration::days(days);
                let hours = time_passed.num_hours();
                time_passed = time_passed - Duration::hours(hours);
                let minutes = time_passed.num_minutes();
                time_passed = time_passed - Duration::minutes(minutes);
                let seconds = time_passed.num_seconds();

                println!(
                    "outdated: latest_candle_open_time_utc = {:?}, time_passed = {}d {}h {}m {}s",
                    latest_candle_open_time_utc, days, hours, minutes, seconds
                );
            }
            CandleManagerState::Empty => println!("empty"),
        }
    }

    #[tokio::test]
    async fn test_candle_manager_update() {
        /*
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::S1;
        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        // inspect BTCUSDT for timeframe and print the result (format `time_passed` as 1d 2h 3m 4s)
        let progress = cm.update(timeframe).unwrap();
        // let progress = progress.read();
        println!("{:?}", progress);
         */
    }

    #[tokio::test]
    async fn test_candle_manager_delete() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::S1;
        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        // inspect BTCUSDT for timeframe and print the result (format `time_passed` as 1d 2h 3m 4s)
        cm.reset(timeframe).unwrap();
    }

    #[tokio::test]
    async fn test_candle_manager_downloader() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::M1;
        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        // cm.delete(timeframe).unwrap();

        let candle_dates = cm.plan_candle_download(682);
        println!("{:#?}", candle_dates);

        /*
        println!("{:#?}", cm.oldest_candle("BTCUSDT", timeframe));
        println!("{:#?}", cm.latest_candle("BTCUSDT", timeframe));

        println!("{:#?}", cm.inspect(timeframe));
        cm.update(timeframe).unwrap();
        println!("{:#?}", cm.inspect(timeframe));

        println!("{:#?}", cm.oldest_candle("BTCUSDT", timeframe));
        println!("{:#?}", cm.latest_candle("BTCUSDT", timeframe));
         */
    }

    #[tokio::test]
    async fn test_get_recent_candles() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::S1;
        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        let it = Instant::now();
        let candles = cm.get_recent_candles(timeframe, 1346269).unwrap();
        println!("{:#?}", it.elapsed());
        // println!("{:#?}", candles);
        println!("{:#?}", candles.first());
        println!("{:#?}", candles.last());
    }

    #[tokio::test]
    async fn test_load_timeframe_window() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::S1;
        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        let it = Instant::now();
        let (head, window) = cm.load_timeframe_window(timeframe, 1346269).unwrap();
        // let (head, window) = cm.load_timeframe_window(timeframe, 100).unwrap();
        println!("{:#?}", it.elapsed());

        eprintln!("head = {:#?}", head);
        eprintln!("window.newest() = {:#?}", window.newest());
        eprintln!("window.oldest() = {:#?}", window.oldest());
        eprintln!("window.get(0) = {:#?}", window.get(0));
        eprintln!("window.get(1) = {:#?}", window.get(1));
        eprintln!("window.get(2) = {:#?}", window.get(2));
        eprintln!("window.get(3) = {:#?}", window.get(3));
        eprintln!("window.get(4) = {:#?}", window.get(4));
    }

    #[test]
    fn test_find_gaps() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::M1;
        let exchange_api = ExchangeApi::default();
        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        let it = Instant::now();
        let gaps = cm.find_gaps(timeframe).unwrap();
        println!("{:#?}", it.elapsed());
        println!("{:#?}", gaps);
    }

    #[tokio::test]
    async fn test_fill_gaps() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::M1;
        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        let it = Instant::now();
        let gaps = cm.find_gaps(timeframe).unwrap();
        println!("{:#?}", it.elapsed());
        println!("{:#?}", gaps);

        let it = Instant::now();
        cm.fix_gaps(timeframe).unwrap();
        println!("{:#?}", it.elapsed());
    }

    #[test]
    fn test_fetch_initial() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();

        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::M1;

        let exchange_api = ExchangeApi::default();

        let coin_config = config.coins.iter().next().unwrap().clone();

        let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

        cm.fetch_initial(50000).expect("Failed to fetch initial candles");
    }
}
