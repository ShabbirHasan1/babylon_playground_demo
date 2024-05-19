use futures_util::stream::StreamExt;
use std::{
    ops::DerefMut,
    sync::{atomic::AtomicBool, Arc},
    thread,
    time::Instant,
};

use anyhow::Result;
use binance_spot_connector_rust::{
    market::klines::KlineInterval,
    market_stream::{diff_depth::DiffDepthStream, kline::KlineStream, partial_depth::PartialDepthStream, trade::TradeStream},
    tokio_tungstenite::BinanceWebSocketClient,
};
use crossbeam::channel::Sender;
use crossbeam_channel::{unbounded, SendError};
use itertools::Itertools;
use log::{debug, error, info, warn};
use ordered_float::OrderedFloat;
use parking_lot::RwLock;
use ustr::Ustr;
use yata::core::{PeriodType, ValueType, Window};

use crate::{
    balance::BalanceSheet,
    candle_manager::CandleManager,
    candle_series::{CandleSeries, ComputedState},
    coin_manager::CoinManager,
    config::{CoinConfig, ConfigDefaults, TimeframeConfig},
    database::Database,
    exchange_client::ExchangeClientMessage,
    id::OrderId,
    instruction::InstructionExecutionResult,
    model::{AccountInformation, Symbol, WebsocketMessage, WebsocketResponse},
    ratelimiter::UnifiedRateLimiter,
    state_moving_average::FrozenMovingAverageState,
    timeframe::TimeframeError,
    timeset::TimeSet,
    trader::Trader,
    types::{DataPoint, ExchangeApi, OrderedValueType, Status},
    util::{max_period, sort_timeframes_by_duration, FIB_SEQUENCE},
    Coin, Config, Timeframe,
};

#[derive(Debug)]
pub enum CoinWorkerMessage {
    WebsocketMessage(WebsocketMessage),
    ExecutedInstruction(InstructionExecutionResult),
    AckOrderOpened {
        local_order_id: OrderId,
        price:          OrderedValueType,
        quantity:       OrderedValueType,
    },
    AckOrderOpenFailed {
        local_order_id: OrderId,
    },
    AckOrderReplaced {
        local_order_id: OrderId,
        old_price:      OrderedValueType,
        old_quantity:   OrderedValueType,
        new_price:      OrderedValueType,
        new_quantity:   OrderedValueType,
    },
    OrderFinalized {
        local_order_id: OrderId,
        status:         Status,
    },
}

pub(crate) struct CoinWorker {
    pub sender: Sender<CoinWorkerMessage>,
}

impl CoinWorker {
    pub fn new(
        graphical_mode: bool,
        runtime_handle: tokio::runtime::Handle,
        config: Config,
        config_defaults: ConfigDefaults,
        coin_config: CoinConfig,
        timeframe_configs: TimeSet<TimeframeConfig>,
        symbol: Ustr,
        symbol_metadata: Symbol,
        account: Arc<AccountInformation>,
        db: Database,
        exchange_api: ExchangeApi,
        balance: BalanceSheet,
        ratelimiter: UnifiedRateLimiter,
        manager: Arc<RwLock<CoinManager>>,
        trader: Trader,
        exchange_client_sender: Sender<ExchangeClientMessage>,
        datapoint_sender: Sender<DataPoint>,
    ) -> Result<Self> {
        let is_running = AtomicBool::new(true);
        let timeframes = sort_timeframes_by_duration(timeframe_configs.iter().map(|(timeframe, _)| timeframe).collect::<Vec<Timeframe>>());
        let trailing_window_size = config.main.series_capacity;

        // ----------------------------------------------------------------------------
        // WARNING
        // The channel is unbounded the buffer because I find this as the most
        // appropriate way to keep the websocket events while the candle manager is
        // updating the database
        // ----------------------------------------------------------------------------
        let (coin_worker_sender, coin_worker_receiver) = unbounded::<CoinWorkerMessage>();

        // ----------------------------------------------------------------------------
        // Updating candle database and loading candles from database
        // WARNING: Running this twice to catch up on the time spent on the initial load
        // ----------------------------------------------------------------------------
        let mut candle_manager =
            CandleManager::new(coin_config.clone(), config_defaults.clone(), timeframe_configs.clone(), db.clone(), exchange_api.clone(), ratelimiter.clone());

        for i in 0..1 {
            match i {
                0 => {
                    warn!("{}: Running candle update", symbol);
                }
                1 => {
                    warn!("{}: Running additional time to catch up", symbol);
                }

                _ => {}
            }

            let mut join_handles = vec![];

            for &timeframe in timeframes.iter() {
                join_handles.push(thread::spawn({
                    // NOTE: cloning each instance of candle manager to run in parallel
                    let mut cm = candle_manager.clone();

                    move || {
                        match cm.fetch(timeframe) {
                            Ok(_) => {
                                info!("Finished updating candles for {} on {:?}", symbol, timeframe);
                            }
                            Err(e) => {
                                error!("Error updating candles for {} on {:?}: {:?}", symbol, timeframe, e);
                            }
                        }

                        /*
                        // FIXME: Some gaps are "natural" and cannot be fixed by this, need a safety stop
                        // NOTE: This is to fix gaps in the database after the initial load
                        if i == 1 {
                            match cm.fix_gaps(timeframe) {
                                Ok(_) => {
                                    info!("{}: Fixed gaps for {:?}", name, timeframe);
                                }
                                Err(e) => {
                                    error!("{}: Failed to fix gaps for {:?}: {:?}", name, timeframe, e);
                                }
                            }
                        }
                         */
                    }
                }));
            }

            // waiting for all updates to finish
            for handle in join_handles {
                handle.join().unwrap();
            }
        }

        // ----------------------------------------------------------------------------
        // Loading candles from database and initializing initial state
        // WARNING: If this is wrong, then all the calculations will be wrong
        // ----------------------------------------------------------------------------
        // The series of candles for each timeframe
        let mut series = TimeSet::<CandleSeries>::new();

        /// The capacity of the series
        let series_capacity = config.main.series_capacity;

        for timeframe in timeframes.iter() {
            // Getting symbol's configuration for this timeframe
            let timeframe_config = timeframe_configs.get(*timeframe)?;

            // Updating this timeframe right before fetching candles
            candle_manager.fetch(*timeframe)?;

            // Need to know how many candles are in the database for this timeframe
            // NOTE: Counting candles after updating the database
            let candle_count = candle_manager.count_candles(*timeframe)?;

            // Finding the actual number of candles to fetch from the database
            let fetch_count = match max_period(FIB_SEQUENCE.as_slice(), candle_count) {
                None => {
                    // IMPORTANT: Unset timeframes in the TimeSet will not be used in the calculations
                    warn!("{}: Not enough candles in the database for {}, skipping this timeframe", symbol, timeframe.as_str());
                    continue;
                }
                Some(fetch_count) => fetch_count,
            };

            // Fetching candles from the database
            let time = Instant::now();
            let historical_candles = candle_manager.load_timeframe_vec(*timeframe, fetch_count)?;
            let fetching_time = time.elapsed();

            // Pushing the fetched candles into the series
            let time = Instant::now();
            for (i, c) in historical_candles.into_iter().enumerate() {
                if i == 0 {
                    series.set(*timeframe, CandleSeries::new(symbol, *timeframe, None, series_capacity, fetch_count, false, false, graphical_mode))?;
                    series.get_mut(*timeframe)?.push_candle(c)?;
                } else {
                    series.get_mut(*timeframe)?.push_candle(c)?;
                }
            }
            let pushing_time = time.elapsed();

            info!(
                "{}: Fetched {} candles out of {} existing, from the database for {} in {:?}, processed in {:?}, total time: {:?}",
                symbol,
                fetch_count,
                candle_count,
                timeframe.as_str(),
                fetching_time,
                pushing_time,
                fetching_time + pushing_time
            );
        }

        for (timeframe, series) in series.iter_mut() {
            info!("{}: {} candles in series for {}: {}", symbol, series.num_candles(), timeframe.as_str(), series.head_candle().unwrap().timeframe);
        }

        // ----------------------------------------------------------------------------
        // Initializing coin context
        // ----------------------------------------------------------------------------

        // WARNING: Flagging the candle series as LIVE
        for (_, series) in series.iter_mut() {
            series.is_live = true;
        }

        let depth = exchange_api.fetch_depth(symbol.as_str(), Some(20))?;

        let mut coin = Coin::new(
            config.clone(),
            config_defaults,
            timeframe_configs.clone(),
            timeframes,
            coin_config.clone(),
            symbol_metadata.clone(),
            account,
            db,
            exchange_api.clone(),
            depth,
            balance,
            ratelimiter,
            manager,
            trader,
            series,
            candle_manager,
            exchange_client_sender,
            coin_worker_sender.clone(),
            datapoint_sender.clone(),
        )?;

        // websocket event processor
        thread::spawn({
            let coin_worker_sender = coin_worker_sender.clone();

            // ----------------------------------------------------------------------------
            // IMPORTANT: Websocket event processor
            // ----------------------------------------------------------------------------

            move || {
                // Spawning a task that handles the websocket stream
                runtime_handle.spawn({
                    let symbol = symbol.as_str();

                    async move {
                        loop {
                            // Establish connection
                            let (mut conn, _) = BinanceWebSocketClient::connect_async_default().await.expect("Failed to connect");

                            // Configuring the streams
                            let mut streams: Vec<binance_spot_connector_rust::Stream> = vec![TradeStream::new(symbol).into()]; // Trade stream is always needed

                            // Adding order book stream
                            // TODO: Make this configurable and add to the coin config
                            streams.push(PartialDepthStream::from_100ms(symbol, 20).into());

                            // Adding order book delta stream
                            // streams.push(DiffDepthStream::from_100ms(symbol).into());

                            // Adding kline streams
                            for (timeframe, _) in timeframe_configs.iter() {
                                // Converting timeframe to kline interval, skipping unsupported custom timeframes
                                let kline_interval = match KlineInterval::try_from(timeframe) {
                                    Ok(interval) => interval,
                                    Err(TimeframeError::UnsupportedCustomTimeframe(timeframe)) => {
                                        warn!("{}: Skipping custom timeframe: {}", symbol, timeframe.as_str());
                                        continue;
                                    }
                                    Err(err) => {
                                        panic!("{}: Failed to convert timeframe to kline interval: {:?}", symbol, err);
                                    }
                                };

                                streams.push(KlineStream::new(symbol, kline_interval).into());
                            }

                            // Subscribe to streams
                            conn.subscribe(&streams).await;

                            // Processing incoming messages
                            while let Some(message) = conn.as_mut().next().await {
                                match message {
                                    Ok(message) => {
                                        let Ok(WebsocketResponse { stream, data }) = serde_json::from_str(message.to_text().unwrap()) else {
                                            debug!("Failed to parse message: {}", message.to_text().unwrap());
                                            continue;
                                        };

                                        // Relaying the message to the coin worker that spawned this task to process the event,
                                        // this is done to avoid blocking the websocket stream with (relatively) heavy computations
                                        if let Err(err) = coin_worker_sender.send(CoinWorkerMessage::WebsocketMessage(data)) {
                                            error!("Failed to send websocket message to coin worker: {:?}", err);
                                        };
                                    }
                                    Err(err) => {
                                        error!("Error reading message: {:?}", err);
                                    }
                                }
                            }

                            warn!("{}: Websocket stream ended, reconnecting", symbol);
                            // TODO: Add a reconnecting strategy
                            // TODO: Add a delay before reconnecting
                            // TODO: Add a maximum number of reconnects
                            // TODO: Add a backoff strategy
                            // TODO: Add a way to stop the reconnecting

                            // Disconnect
                            conn.close().await.expect("Failed to disconnect");
                        }
                    }
                });

                // ----------------------------------------------------------------------------
                // IMPORTANT: Coin worker event processor
                // ----------------------------------------------------------------------------

                loop {
                    match coin_worker_receiver.recv() {
                        Ok(message) => {
                            match message {
                                CoinWorkerMessage::WebsocketMessage(data) => match data {
                                    WebsocketMessage::ExchangeTrade(trade) => coin.write().push_trade(trade).unwrap(),
                                    WebsocketMessage::Kline(kline) => coin.write().push_candle(kline.candle).unwrap(),
                                    WebsocketMessage::PartialDepth(partial_depth) => coin
                                        .write()
                                        .update_orderbook_partial_20(partial_depth.last_update_id, partial_depth.bids, partial_depth.asks)
                                        .unwrap(),
                                    WebsocketMessage::DiffDepth(diff_depth) => coin
                                        .write()
                                        .update_orderbook_diff(diff_depth.final_update_id, diff_depth.bids, diff_depth.asks)
                                        .unwrap(),
                                    _ => {}
                                },

                                CoinWorkerMessage::ExecutedInstruction(result) => {
                                    coin.write().push_instruction_result(result).expect("pushed instruction result");
                                }

                                CoinWorkerMessage::AckOrderOpenFailed { local_order_id } => {
                                    // coin.write().confirm_roundtrip_failed_to_open(id).expect("confirmed roundtrip failed to open");
                                }

                                CoinWorkerMessage::AckOrderOpened {
                                    local_order_id,
                                    price,
                                    quantity,
                                } => {
                                    /*
                                    coin.write()
                                        .confirm_roundtrip_opened(id, price, exit_price)
                                        .expect("confirmed roundtrip opened");
                                     */
                                }

                                CoinWorkerMessage::AckOrderReplaced {
                                    local_order_id,
                                    old_price,
                                    old_quantity,
                                    new_price,
                                    new_quantity,
                                } => {
                                    /*
                                    coin.write()
                                        .confirm_roundtrip_replaced(id, old_entry_price, old_exit_price, new_entry_price, new_exit_price)
                                        .expect("confirmed roundtrip replaced");
                                     */
                                }

                                CoinWorkerMessage::OrderFinalized { local_order_id, status } => {
                                    // coin.write().push_roundtrip_feedback(id, status).expect("pushed roundtrip feedback");
                                }
                            }
                        }

                        Err(err) => {
                            panic!("failed to receive websocket event: {}", err);
                        }
                    }
                }
            }
        });

        Ok(Self { sender: coin_worker_sender })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{database::DatabaseBackend, util::timeframe_configs_to_timeset};

    use binance_spot_connector_rust::{
        hyper::{BinanceHttpClient, Error},
        market::{self, klines::KlineInterval},
    };
    use chrono::{Duration, Utc};
    use env_logger::Builder;
    use ustr::ustr;

    #[tokio::test]
    async fn test_binance_connector() {
        Builder::from_default_env().filter(None, log::LevelFilter::Info).init();
        let client = BinanceHttpClient::default();
        let request = market::klines("BNBUSDT", KlineInterval::Minutes1).start_time((Utc::now() - Duration::days(10)).timestamp_millis() as u64);
        let data = client.send(request).await.unwrap().into_body_str().await.unwrap();

        println!("{:#?}", data);
    }

    /*
    #[tokio::test]
    async fn test_initialization() {
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).unwrap();
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::M1;
        let exchange_api = ExchangeApi::default();
        let coin_config = config.coins.iter().next().unwrap().clone();
        let timeframes = sort_timeframes_by_duration(timeframe_configs.iter().map(|(timeframe, _)| timeframe).collect::<Vec<Timeframe>>());
        let name = coin_config.name.clone();

        // FIXME: don't clone the coin_config and timeframe_configs after done with testing
        let mut candle_manager =
            CandleManager::new(coin_config.clone(), config.defaults.clone(), timeframe_configs.clone(), db, exchange_api, UnifiedRateLimiter::default());

        // ----------------------------------------------------------------------------
        // Loading candles from database and initializing initial state
        // WARNING: If this is wrong, then all the calculations will be wrong
        // ----------------------------------------------------------------------------
        // NOTE: The following frozen states are pre-computed with historical candles
        let mut frozen_indicator_state = TimeSet::<FrozenIndicatorState>::new();
        let mut frozen_moving_average_state = Arc::new(RwLock::new(TimeSet::<FrozenMovingAverageState>::new()));
        let mut state: TimeSet<(ComputedState, Window<ComputedState>)> = Default::default();

        // NOTE: This is the list moving average lengths that will be used for the initial state
        let seq = [
            3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765, 10946, 17711, 28657, 46368, 75025, 121393, 196418, 317811, 514229,
            832040, 1346269,
        ];

        for timeframe in timeframes.iter() {
            // Getting symbol's configuration for this timeframe
            let timeframe_config = timeframe_configs.get(*timeframe).unwrap();

            // Updating this timeframe right before fetching candles
            candle_manager.update(*timeframe).unwrap();

            // Need to know how many candles are in the database for this timeframe
            // NOTE: Counting candles after updating the database
            let candle_count = candle_manager.count_candles(*timeframe).unwrap();

            // Finding the actual number of candles to fetch from the database
            let fetch_count = match max_seq_leq(seq.as_slice(), candle_count) {
                None => {
                    // IMPORTANT: Unset timeframes in the TimeSet will not be used in the calculations
                    println!("{}: Not enough candles in the database for {}, skipping this timeframe", name, timeframe.as_str());
                    continue;
                }
                Some(fetch_count) => fetch_count,
            };

            // ----------------------------------------------------------------------------
            // IMPORTANT: Composing the compound state from the precomputed states
            // ----------------------------------------------------------------------------

            println!(
                "{}: Computing a window of {} historical values over {} candles in total and states for {} timeframe",
                coin_config.name,
                100,
                fetch_count,
                timeframe_config.timeframe.as_str()
            );

            let it = Instant::now();

            // Loading candles from database for this timeframe
            let (head_candle, trailing_candles) = candle_manager.load_timeframe_vec_head_tail(*timeframe, fetch_count as PeriodType).unwrap();

            /*
            if *timeframe == Timeframe::M30 {
                for candle in trailing_candles {
                    println!("{:?} {}", candle.open_time, candle.close);
                }

                eprintln!("head_candle = {:#?}", head_candle);
                panic!();
            }
             */

            let (precomputed_frozen_indicator_state, computed_indicator_states, head_indicator_state) =
                FrozenIndicatorState::new(timeframe_config, 100, &trailing_candles).unwrap();

            // println!("{:#?}", computed_indicator_states);
            // println!("{:#?}", head_indicator_state);
            // panic!();

            let (precomputed_frozen_moving_average_state, computed_moving_average_states, head_moving_average_state) =
                FrozenMovingAverageState::new(timeframe_config, 100, &trailing_candles).unwrap();

            // Initializing the frozen state timeset for this timeframe
            frozen_indicator_state.set(*timeframe, precomputed_frozen_indicator_state);

            // TODO: Moving average state is not actually frozen, rename this later
            frozen_moving_average_state.write().set(*timeframe, precomputed_frozen_moving_average_state);

            // IMPORTANT: Re-combining the precomputed states into the window
            for (i, ((candle, computed_indicator_state), computed_moving_average_state)) in trailing_candles
                .iter()
                .rev()
                // IMPORTANT: +1 because we need to take the head candle into account
                .take(computed_indicator_states.len() as usize + 1)
                .rev()
                .zip(computed_indicator_states.iter_rev())
                .zip(computed_moving_average_states.iter_rev())
                .enumerate()
            {
                if candle.open_time != computed_indicator_state.candle.open_time && candle.open_time != computed_moving_average_state.candle.open_time {
                    eprintln!("candle = {:#?}", candle);
                    eprintln!("computed_indicator_state.candle = {:#?}", computed_indicator_state.candle);
                    eprintln!("computed_moving_average_state.candle = {:#?}", computed_moving_average_state.candle);

                    panic!("{}: Candles don't match", name);
                }

                if computed_indicator_state.candle.open_time != computed_moving_average_state.candle.open_time {
                    eprintln!("computed_indicator_state.candle = {:#?}", computed_indicator_state.candle);
                    eprintln!("computed_moving_average_state.candle = {:#?}", computed_moving_average_state.candle);

                    panic!("{}: Candles don't match", name);
                }

                // NOTE: This is a hack to initialize the window
                if i == 0 {
                    // Initializing the time set with the head state of this timeframe
                    state.set(
                        *timeframe,
                        (
                            ComputedState::new(candle.clone(), head_indicator_state, head_moving_average_state),
                            Window::new(100, ComputedState::new(candle.clone(), computed_indicator_state.clone(), computed_moving_average_state.clone())),
                        ),
                    );
                } else {
                    // Pushing the precomputed compound state into the window
                    state.get_mut(*timeframe).unwrap().1.push(ComputedState::new(
                        candle.clone(),
                        computed_indicator_state.clone(),
                        computed_moving_average_state.clone(),
                    ));
                }
            }

            println!(
                "{}: Fetched {} candles out of {} existing, from the database for {} in {:?}",
                name,
                fetch_count,
                candle_count,
                timeframe.as_str(),
                it.elapsed()
            );
        }
        }
     */
}
