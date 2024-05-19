use crate::{candle::Candle, config::DatabaseConfig, timeframe::Timeframe};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use parking_lot::Mutex;
use postgres::{Client, Error, NoTls, Row};
use std::{str::FromStr, sync::Arc};
use thiserror::Error;
use ustr::{ustr, Ustr};
use yata::core::ValueType;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("Candle not found")]
    CandleNotFound,

    #[error("Error: {0}")]
    Error(String),

    #[error("Database error: {0}")]
    ConnectionError(String),

    #[error("Postgres error: {0}")]
    PostgresError(#[from] postgres::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// IMPORTANT: Functions that fetch data from the database should always return the data in ascending order of time.
pub trait DatabaseBackend {
    fn new(config: DatabaseConfig) -> Result<Self, Error>
    where
        Self: Sized;

    fn is_coin_known(&self, symbol: Ustr) -> Result<bool, DatabaseError>;
    fn create_coin(&mut self, symbol: Ustr) -> Result<(), DatabaseError>;
    fn delete_coin(&mut self, symbol: Ustr) -> Result<(), DatabaseError>;
    fn rollback_coin(&mut self, symbol: Ustr, timeframe: Timeframe, period: Duration, use_oldest_entry_as_start: bool) -> Result<(), DatabaseError>;
    fn upsert_candle(&mut self, symbol: Ustr, candle: Candle) -> Result<(), DatabaseError>;
    fn count_candles(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<usize, DatabaseError>;
    fn count_candles_in_range(
        &mut self,
        symbol: Ustr,
        timeframe: Timeframe,
        start_time: DateTime<Utc>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<usize, DatabaseError>;
    fn count_candles_per_timeframe(&mut self, symbol: Ustr) -> Result<(Ustr, Vec<(Timeframe, usize)>), DatabaseError>;
    fn get_oldest_candle(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<Candle, DatabaseError>;
    fn get_latest_candle(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<Candle, DatabaseError>;
    fn get_recent_candles(&mut self, symbol: Ustr, timeframe: Timeframe, n: usize) -> Result<Vec<Candle>, DatabaseError>;
    fn get_candles_in_range(
        &mut self,
        symbol: Ustr,
        timeframe: Timeframe,
        start_time: DateTime<Utc>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<Candle>, DatabaseError>;
    fn get_candles_chunk(
        &mut self,
        symbol: Ustr,
        timeframe: Timeframe,
        chunk_size: usize,
        last_fetched_open_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<Candle>, DatabaseError>;
    fn get_all_candles(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<Vec<Candle>, DatabaseError>;
    fn delete_candles(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<(), DatabaseError>;
    fn delete_older_candles(&mut self, symbol: Ustr, timeframe: Timeframe, retained_age: Duration) -> Result<(), DatabaseError>;
}

#[derive(Clone)]
pub struct Database {
    client: Arc<Mutex<Client>>,
}

impl DatabaseBackend for Database {
    fn new(config: DatabaseConfig) -> Result<Self, Error>
    where
        Self: Sized,
    {
        let connection_string = format!("host={} user={} password={} dbname={}", config.hostname, config.username, config.password, config.database);
        let client = Arc::new(Mutex::new(Client::connect(connection_string.as_str(), NoTls)?));

        Ok(Self { client })
    }

    fn is_coin_known(&self, symbol: Ustr) -> Result<bool, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.as_str().to_lowercase());

        let query = "SELECT EXISTS (
        SELECT FROM information_schema.tables 
        WHERE  table_schema = 'public'
        AND    table_name   = $1)";

        let row = client.query_one(query, &[&table_name])?;

        Ok(row.get("exists"))
    }

    fn create_coin(&mut self, symbol: Ustr) -> Result<(), DatabaseError> {
        let symbol_str = symbol.as_str().to_lowercase();
        let candle_table = format!("candles_{}", symbol_str);

        let mut client = self.client.lock();
        let mut transaction = client.transaction()?;

        transaction.execute(
            &format!(
                r#"
                CREATE TABLE IF NOT EXISTS {}
                (
                    symbol                       text             not null,
                    timeframe                    text             not null,
                    is_final                     boolean          not null,
                    number_of_trades             bigint           not null,
                    open_time                    timestamptz      not null,
                    close_time                   timestamptz      not null,
                    open                         double precision not null,
                    high                         double precision not null,
                    low                          double precision not null,
                    close                        double precision not null,
                    volume                       double precision not null,
                    quote_asset_volume           double precision not null,
                    taker_buy_base_asset_volume  double precision not null,
                    taker_buy_quote_asset_volume double precision not null,
                    CONSTRAINT {}_pk
                        PRIMARY KEY (timeframe, open_time)
                );
                "#,
                candle_table, candle_table
            ),
            &[],
        )?;

        transaction.commit()?;

        Ok(())
    }

    fn delete_coin(&mut self, symbol: Ustr) -> Result<(), DatabaseError> {
        let symbol_str = symbol.as_str().to_lowercase();
        let candle_table = format!("candles_{}", symbol_str);
        let mut client = self.client.lock();

        let mut transaction = client.transaction()?;
        transaction.execute(&format!("DROP TABLE IF EXISTS {};", candle_table), &[])?;
        transaction.commit()?;

        Ok(())
    }

    fn rollback_coin(&mut self, symbol: Ustr, timeframe: Timeframe, period: Duration, from_most_recent: bool) -> Result<(), DatabaseError> {
        let symbol_str = symbol.as_str().to_lowercase();
        let candle_table = format!("candles_{}", symbol_str);
        let mut client = self.client.lock();

        let mut transaction = client.transaction()?;

        // Determine the threshold_time based on the use_most_recent_entry_as_start flag
        let threshold_time = if from_most_recent {
            let select_query = format!("SELECT MAX(open_time) FROM {};", candle_table);
            let max_time: Option<NaiveDateTime> = transaction.query_one(&select_query, &[]).ok().map(|row| row.get(0));
            max_time.unwrap_or(Utc::now().naive_utc()) - period
        } else {
            Utc::now().naive_utc() - period
        };

        let delete_query = format!("DELETE FROM {} WHERE open_time > $1;", candle_table);
        transaction.execute(&delete_query, &[&threshold_time])?;
        transaction.commit()?;

        Ok(())
    }

    fn upsert_candle(&mut self, symbol: Ustr, candle: Candle) -> Result<(), DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());

        let query = format!(
            r#"
            INSERT INTO {} (symbol, timeframe, is_final, number_of_trades, open_time, close_time, open, high, low, close, volume, quote_asset_volume, taker_buy_base_asset_volume, taker_buy_quote_asset_volume)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (timeframe, open_time) DO UPDATE
            SET is_final = EXCLUDED.is_final, close = EXCLUDED.close, high = EXCLUDED.high, low = EXCLUDED.low, volume = EXCLUDED.volume, number_of_trades = EXCLUDED.number_of_trades, quote_asset_volume = EXCLUDED.quote_asset_volume, taker_buy_base_asset_volume = EXCLUDED.taker_buy_base_asset_volume, taker_buy_quote_asset_volume = EXCLUDED.taker_buy_quote_asset_volume
            "#,
            table_name
        );

        client.execute(
            &query,
            &[
                &symbol.as_str(),
                &candle.timeframe.as_str(),
                &candle.is_final,
                &candle.number_of_trades,
                &candle.open_time,
                &candle.close_time,
                &candle.open,
                &candle.high,
                &candle.low,
                &candle.close,
                &candle.volume,
                &candle.quote_asset_volume,
                &candle.taker_buy_base_asset_volume,
                &candle.taker_buy_quote_asset_volume,
            ],
        )?;

        Ok(())
    }

    fn count_candles(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<usize, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());
        let query = format!("SELECT COUNT(*) FROM {} WHERE timeframe = $1", table_name);
        let rows = client.query(&query, &[&timeframe.as_str()])?;

        Ok(if rows.is_empty() { 0 } else { rows[0].get::<_, i64>(0) as usize })
    }

    fn count_candles_in_range(
        &mut self,
        symbol: Ustr,
        timeframe: Timeframe,
        start_time: DateTime<Utc>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<usize, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());

        let rows = match end_time {
            Some(end_time) => {
                let query = format!("SELECT COUNT(*) FROM {} WHERE timeframe = $1 AND open_time >= $2 AND close_time <= $3", table_name);
                client.query(&query, &[&timeframe.as_str(), &start_time, &end_time])?
            }
            None => {
                let query = format!("SELECT COUNT(*) FROM {} WHERE timeframe = $1 AND open_time >= $2", table_name);
                client.query(&query, &[&timeframe.as_str(), &start_time])?
            }
        };

        Ok(if rows.is_empty() { 0 } else { rows[0].get::<_, i64>(0) as usize })
    }

    fn count_candles_per_timeframe(&mut self, symbol: Ustr) -> Result<(Ustr, Vec<(Timeframe, usize)>), DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.as_str());

        let query = format!(
            "SELECT timeframe, COUNT(*) 
            FROM {} 
            GROUP BY timeframe",
            table_name
        );

        let rows = client.query(&query, &[])?;

        let result = rows
            .iter()
            .map(|row| {
                let timeframe: String = row.get(0);
                let count: i64 = row.get(1);
                (Timeframe::from_str(&timeframe).unwrap(), count as usize)
            })
            .collect();

        Ok((symbol, result))
    }

    fn get_oldest_candle(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<Candle, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());
        let query = format!("SELECT * FROM {} WHERE timeframe = $1 ORDER BY open_time ASC LIMIT 1", table_name);
        let rows = client.query(&query, &[&timeframe.as_str()])?;

        if rows.is_empty() {
            Err(DatabaseError::CandleNotFound)
        } else {
            Ok(Candle::from_row(&rows[0]))
        }
    }

    fn get_latest_candle(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<Candle, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());
        let query = format!("SELECT * FROM {} WHERE timeframe = $1 ORDER BY open_time DESC LIMIT 1", table_name);
        let rows = client.query(&query, &[&timeframe.as_str()])?;

        if rows.is_empty() {
            Err(DatabaseError::CandleNotFound)
        } else {
            Ok(Candle::from_row(&rows[0]))
        }
    }

    fn get_recent_candles(&mut self, symbol: Ustr, timeframe: Timeframe, n: usize) -> Result<Vec<Candle>, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());
        let query = format!("SELECT * FROM {} WHERE timeframe = $1 ORDER BY open_time DESC LIMIT $2", table_name);
        let rows = client.query(&query, &[&timeframe.as_str(), &(n as i64)])?;
        Ok(rows.iter().rev().map(|row| Candle::from_row(row)).collect())
    }

    fn get_candles_in_range(
        &mut self,
        symbol: Ustr,
        timeframe: Timeframe,
        start_time: DateTime<Utc>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<Candle>, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());

        let rows = match end_time {
            Some(end_time) => {
                let query = format!("SELECT * FROM {} WHERE timeframe = $1 AND open_time >= $2 AND close_time <= $3 ORDER BY open_time ASC", table_name);
                client.query(&query, &[&timeframe.as_str(), &start_time, &end_time])?
            }
            None => {
                let query = format!("SELECT * FROM {} WHERE timeframe = $1 AND open_time >= $2 ORDER BY open_time ASC", table_name);
                client.query(&query, &[&timeframe.as_str(), &start_time])?
            }
        };

        Ok(rows.into_iter().rev().map(|row| Candle::from_row(&row)).collect())
    }

    fn get_candles_chunk(
        &mut self,
        symbol: Ustr,
        timeframe: Timeframe,
        chunk_size: usize,
        last_fetched_open_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<Candle>, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());

        let rows = match last_fetched_open_time {
            Some(last_fetched_open_time) => {
                let query = format!("SELECT * FROM {} WHERE timeframe = $1 AND open_time < $2 ORDER BY open_time DESC LIMIT $3", table_name);
                client.query(&query, &[&symbol.as_str(), &timeframe.as_str(), &last_fetched_open_time, &(chunk_size as i64)])?
            }
            None => {
                let query = format!("SELECT * FROM {} WHERE timeframe = $1 ORDER BY open_time DESC LIMIT $2", table_name);
                client.query(&query, &[&timeframe.as_str(), &(chunk_size as i64)])?
            }
        };

        Ok(rows.into_iter().rev().map(|row| Candle::from_row(&row)).collect())
    }

    fn get_all_candles(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<Vec<Candle>, DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());
        let query = format!("SELECT * FROM {} WHERE timeframe = $1 ORDER BY open_time ASC", table_name);
        let rows = client.query(&query, &[&timeframe.as_str()])?;
        Ok(rows.into_iter().map(|row| Candle::from_row(&row)).collect())
    }

    fn delete_candles(&mut self, symbol: Ustr, timeframe: Timeframe) -> Result<(), DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());

        let mut transaction = client.transaction()?;
        let query = format!("DELETE FROM {} WHERE timeframe = $1", table_name);
        transaction.execute(&query, &[&timeframe.as_str()])?;
        transaction.commit()?;

        Ok(())
    }

    fn delete_older_candles(&mut self, symbol: Ustr, timeframe: Timeframe, retained_age: Duration) -> Result<(), DatabaseError> {
        let mut client = self.client.lock();
        let table_name = format!("candles_{}", symbol.to_lowercase());
        let oldest_time = Utc::now() - retained_age;
        let query = format!("DELETE FROM {} WHERE timeframe = $1 AND open_time < $2", table_name);

        let mut transaction = client.transaction()?;
        transaction.execute(&query, &[&timeframe.as_str(), &oldest_time])?;
        transaction.commit()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_new_tables() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let mut db = Database::new(config.database.clone()).unwrap();
        let symbol = ustr("TESTCOIN");

        assert_eq!(db.is_coin_known(symbol).unwrap(), false);
        db.create_coin(symbol).unwrap();
        assert_eq!(db.is_coin_known(symbol).unwrap(), true);
        db.delete_coin(symbol).unwrap();
        assert_eq!(db.is_coin_known(symbol).unwrap(), false);
    }
}
