// #![feature(test)]
#![allow(dead_code)]
#![allow(unused)]
#![warn(clippy::all, rust_2018_idioms)]
// #![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

extern crate core;

use futures_util::stream::StreamExt;

use std::{
    borrow::{Borrow, BorrowMut},
    collections::{btree_map::Entry, BTreeMap},
    fs,
    fs::File,
    io::{stdout, BufWriter},
    ops::DerefMut,
    process::exit,
    str::FromStr,
    sync::{atomic::AtomicBool, Arc},
    thread,
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use binance_spot_connector_rust::{
    market::klines::KlineInterval,
    market_stream::{diff_depth::DiffDepthStream, kline::KlineStream, partial_depth::PartialDepthStream, trade::TradeStream},
    tokio_tungstenite::BinanceWebSocketClient,
    user_data_stream::UserDataStream,
};
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use clap::Parser;
use countdown::Countdown;
use crossbeam_channel::{bounded, unbounded};
use eframe::{HardwareAcceleration, IconData};
use itertools::Itertools;
use log::{error, info, warn};
#[cfg(target_env = "msvc")]
use mimalloc::MiMalloc;
use num::Float;
use parking_lot::RwLock;
use ratelimiter::UnifiedRateLimiter;
use serde::{
    ser::{Serialize, SerializeMap, SerializeSeq, Serializer},
    Deserialize,
};
#[cfg(not(target_env = "msvc"))]
use tikv_jemallocator::Jemalloc;
use tokio::{runtime, runtime::Runtime};
use ustr::ustr;
use util::Snapshot;
use yata::methods::{Integral, ROC};

use crate::{
    app::App,
    candle::Candle,
    candle_manager::{CandleManager, CandleManagerState},
    cli::{Cli, Commands},
    coin::Coin,
    coin_manager::CoinManager,
    config::{CoinConfig, Config, RpcConfig},
    context::{Context, ContextInstance},
    cooldown::Cooldown,
    database::{Database, DatabaseBackend},
    model::{WebsocketMessage, WebsocketResponse, WebsocketUserMessage, WebsocketUserStreamResponse},
    playground_app::PlaygroundApp,
    playground_context::PlaygroundContextInstance,
    semaphore::Semaphore,
    simulator::{Simulator, SimulatorConfig},
    simulator_generator::GeneratorConfig,
    state_indicator::ComputedIndicatorState,
    timeframe::Timeframe,
    types::ExchangeApi,
    util::{coin_specific_timeframe_configs, extract_symbols, s2f, s2i64, timeframe_configs_to_timeset},
};

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(target_env = "msvc")]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod app;
mod app_orderbook;
mod appraiser;
mod balance;
mod candle;
mod candle_manager;
mod candle_pattern;
mod candle_series;
mod cli;
mod coin;
mod coin_manager;
mod coin_worker;
mod components;
mod config;
mod context;
mod cooldown;
mod countdown;
mod database;
mod exchange_client;
mod exchange_client_worker;
mod helpers;
mod id;
mod instruction;
mod leverage;
mod model;
mod order;
mod orderbook;
mod orderbook_delta;
mod orderbook_level;
mod pattern;
mod playground_app;
mod playground_context;
mod plot_action;
mod plot_action_bias;
mod plot_action_limit_order;
mod plot_action_roundtrip;
mod plot_action_ruler;
mod plot_action_trade_builder;
mod position;
mod position_accumulator;
mod process;
mod progress;
mod ratelimiter;
mod roundtrip;
mod semaphore;
mod series;
mod simulator;
mod simulator_generator;
mod simulator_order;
mod sliding_window;
mod state_analyzer;
mod state_crossover;
mod state_indicator;
mod state_indicator_deprecated;
mod state_indicator_util;
mod state_moving_average;
mod support_resistance;
mod timeframe;
mod timeset;
mod trade;
mod trade_closer;
mod trade_opener;
mod trader;
mod trigger;
mod trigger_delayed;
mod trigger_delayed_zone;
mod trigger_distance;
mod trigger_proximity;
mod types;
mod util;
mod waitgroup;

fn main() -> Result<()> {
    if std::path::Path::new(".env").exists() {
        dotenvy::dotenv()?;
    }

    /*
    // the process must exit on panic
    panic::set_hook(Box::new(move |info| {
        panic::take_hook()(info);
        process::exit(1);
    }));
     */

    // initializing CLI commands
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();

    match cli.command {
        // AddCoin command to add a new coin to the configuration
        Commands::AddCoin {
            config,
            name,
            is_enabled,
            is_favorite,
            timeframes,
        } => command_add_coin(config.as_str(), name, is_enabled, is_favorite, timeframes),

        // DeleteCoin command to delete a coin from the configuration
        Commands::DeleteCoin { config, name } => command_delete_coin(config.as_str(), name),

        // EditCoin command to edit a coin in the configuration
        Commands::EditCoin {
            config,
            name,
            is_enabled,
            is_favorite,
            timeframes,
        } => command_edit_coin(config.as_str(), name, is_enabled, is_favorite, timeframes),

        // ----------------------------------------------------------------------------
        // Run the bot
        // ----------------------------------------------------------------------------
        Commands::Run {
            config,
            is_graphical,
            is_restricted,
            favorite_only,
        } => command_run(config.as_str(), is_graphical, is_restricted, favorite_only),

        // ----------------------------------------------------------------------------
        // Run offline playground
        // ----------------------------------------------------------------------------
        Commands::RunPlayground { config } => command_run_playground(config.as_str()),

        // ----------------------------------------------------------------------------
        // Run an experiment
        // ----------------------------------------------------------------------------
        Commands::RunExperiment { config } => command_run_experiment(config.as_str()),

        // ----------------------------------------------------------------------------
        // Inspect the candle data
        // ----------------------------------------------------------------------------
        Commands::Inspect {
            config,
            symbol,
            timeframes,
            all_timeframes,
        } => command_inspect(config.as_str(), symbol),

        // ----------------------------------------------------------------------------
        // Initial fetch of the candle data for a symbol for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::InitialFetch {
            config,
            symbol,
            timeframes,
            all_timeframes,
        } => command_fetch_initial(config.as_str(), symbol, if all_timeframes { None } else { Some(timeframes) }),

        // ----------------------------------------------------------------------------
        // Update the candle data for a symbol for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::FetchUpdates {
            config,
            symbol,
            timeframes,
            all_timeframes,
        } => command_fetch_updates(config.as_str(), symbol, if all_timeframes { None } else { Some(timeframes) }),

        // ----------------------------------------------------------------------------
        // Update the candle data for a symbol for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::Update {
            config,
            symbol,
            timeframes,
            all_timeframes,
        } => command_update(config.as_str(), symbol, if all_timeframes { None } else { Some(timeframes) }),

        // ----------------------------------------------------------------------------
        // Update the candle data for all symbols for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::UpdateAll {
            config,
            timeframes,
            all_timeframes,
        } => command_update_all(config.as_str(), if all_timeframes { None } else { Some(timeframes) }),

        // ----------------------------------------------------------------------------
        // List the candle data for a symbol for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::List { config, simple, favorite_only } => command_list(config.as_str(), simple, favorite_only),

        // ----------------------------------------------------------------------------
        // Fix the candle data for a symbol for a set of specific or all timeframes
        // For example, if there are gaps in the data, this will try to fill them.
        // ----------------------------------------------------------------------------
        Commands::FixGaps {
            config,
            symbol,
            timeframes,
            all_timeframes,
        } => command_fix_gaps(config.as_str(), symbol, if all_timeframes { None } else { Some(timeframes) }),

        // ----------------------------------------------------------------------------
        // Delete the candle data for a symbol for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::Reset {
            config,
            symbol,
            timeframes,
            all_timeframes,
        } => command_reset_coin(config.as_str(), symbol, if all_timeframes { None } else { Some(timeframes) }),

        // ----------------------------------------------------------------------------
        // Delete the candle data for a symbol for a set of specific or all timeframes
        // ----------------------------------------------------------------------------
        Commands::Rollback {
            config,
            symbol,
            timeframes,
            all_timeframes,
            period,
            from_most_recent,
        } => command_rollback(config.as_str(), symbol, if all_timeframes { None } else { Some(timeframes) }, period, from_most_recent),
    }
}

fn command_add_coin(config_path: &str, name: String, is_enabled: Option<bool>, is_favorite: Option<bool>, timeframes: Option<Vec<String>>) -> Result<()> {
    let mut config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(name.to_uppercase().as_str());
    let timeframes = timeframes.unwrap_or(config.default_timeframe_configs.iter().map(|c| c.timeframe.to_string()).collect_vec());

    // Add a new coin
    let mut new_coin = CoinConfig {
        name: ustr(symbol.as_str()),
        is_enabled: is_enabled.unwrap_or(true),
        is_favorite,
        timeframes: Some(timeframes.iter().map(|t| Timeframe::from_str(t.as_str()).unwrap()).collect_vec()),
        ..Default::default()
    };

    config.coins.push(new_coin.clone());
    config.save(config_path)?;

    println!("Coin added successfully:");
    println!("Name: {}", symbol);
    println!("Is Enabled: {:?}", new_coin.is_enabled);
    println!("Is Favorite: {:?}", new_coin.is_favorite.unwrap_or_default());
    println!("Timeframes: {:?}", new_coin.timeframes.unwrap_or_default());

    Ok(())
}

fn command_delete_coin(config_path: &str, name: String) -> Result<()> {
    let mut config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(name.to_uppercase().as_str());

    // Find and remove the coin
    if let Some(index) = config.coins.iter().position(|c| c.name == symbol) {
        config.coins.remove(index);
    } else {
        return Err(anyhow::anyhow!("Coin {} not found in the configuration", symbol));
    }

    // TODO: remove the coin from the database

    config.save(config_path)?;

    println!("Coin {} deleted successfully.", symbol);

    Ok(())
}

fn command_reset_coin(config_path: &str, symbol: String, timeframes: Option<Vec<Timeframe>>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(symbol.to_uppercase().as_str());

    let coin_config = match config.get_coin(&symbol) {
        None => {
            bail!("No config found for coin {}", symbol);
        }
        Some(coin_config) => coin_config,
    };

    let timeframes = match timeframes {
        None => config
            .default_timeframe_configs
            .iter()
            .map(|c| c.timeframe)
            .sorted_by(|a, b| a.duration().cmp(&b.duration()))
            .rev()
            .collect_vec(),

        Some(timeframes) => {
            if timeframes.is_empty() {
                bail!("No timeframes specified");
            }

            timeframes
        }
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(num_cpus::get())
        .build()?;

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());

    let coin_config = match config.get_coin(&symbol) {
        None => {
            bail!("No config found for coin {}", symbol);
        }
        Some(coin_config) => coin_config,
    };

    let mut cm = CandleManager::new(coin_config, config.defaults, timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

    info!("Deleting candles for {} on {:?}", symbol, timeframes);

    for timeframe in timeframes {
        cm.reset(timeframe)?;
    }

    Ok(())
}

fn command_edit_coin(config_path: &str, name: String, is_enabled: Option<bool>, is_favorite: Option<bool>, timeframes: Option<Vec<String>>) -> Result<()> {
    let mut config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(name.to_uppercase().as_str());

    // Find and edit the coin
    if let Some(coin) = config.coins.iter_mut().find(|c| c.name == symbol) {
        if let Some(is_enabled) = is_enabled {
            coin.is_enabled = is_enabled;
        }
        if let Some(is_favorite) = is_favorite {
            coin.is_favorite = Some(is_favorite);
        }
        if let Some(timeframes) = timeframes {
            coin.timeframes = Some(timeframes.iter().map(|t| Timeframe::from_str(t.as_str()).unwrap()).collect_vec());
        }
    } else {
        return Err(anyhow::anyhow!("Coin {} not found in the configuration", symbol));
    }

    config.save(config_path)?;

    println!("Coin {} edited successfully.", symbol);

    Ok(())
}

fn command_run(config_path: &str, graphical_mode: bool, is_restricted: bool, favorite_only: bool) -> Result<()> {
    // Initializing the runtime
    let tokio_runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(num_cpus::get())
        .build()
        .expect("Failed to initialize tokio runtime");

    // Loading the configuration
    let config = Config::from_file_with_env("BABYLON", config_path)?;

    // Channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    // Initializing the API context
    let mut exchange_api = if is_restricted {
        ExchangeApi::new(Some(config.exchange.restricted_key.clone()), Some(config.exchange.restricted_secret.clone()))
    } else {
        ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()))
    };

    // ----------------------------------------------------------------------------
    // Initializing the user stream before starting the main loop and
    // passing the runtime and the exchange API to the main context
    // ----------------------------------------------------------------------------

    let (userstream_sender, userstream_receiver) = unbounded::<WebsocketUserMessage>();
    let listen_key = exchange_api.start_user_stream().unwrap();

    info!("Subscribing to user stream...");
    tokio_runtime.spawn({
        let exchange_api = exchange_api.clone();

        async move {
            // Establish connection
            let (mut conn, _) = BinanceWebSocketClient::connect_async_default().await.expect("Failed to connect");
            let mut timestamp_countdown = Countdown::new(5u8);

            // Subscribe to streams
            conn.subscribe(vec![&UserDataStream::new(listen_key.as_str()).into()]).await;

            // Read messages
            while let Some(message) = conn.as_mut().next().await {
                match message {
                    Ok(message) => {
                        let raw_text = message.to_text().unwrap();

                        match serde_json::from_str::<WebsocketUserStreamResponse>(raw_text) {
                            Ok(response) => {
                                /*
                                // FIXME: Remove after debugging
                                if let WebsocketUserMessage::OrderUpdate(update) = &response.data {
                                    println!("{:#?}", update);
                                }
                                 */

                                userstream_sender.send(response.data).unwrap();
                            }

                            Err(err) => {
                                let Ok(maybe_timestamp) = raw_text.parse::<i64>() else {
                                    println!("Failed to parse message as timestamp: {}", raw_text);
                                    continue;
                                };

                                // NOTE: Binance sends a message with the server time on a 3-minute interval
                                match NaiveDateTime::from_timestamp_millis(s2i64(raw_text)) {
                                    None => {
                                        println!("Failed to parse message: {}", raw_text);
                                        continue;
                                    }
                                    Some(timestamp) => {
                                        // If the countdown has expired, send a keepalive message and reset the countdown
                                        if !timestamp_countdown.checkin() {
                                            println!("Server time: {}", timestamp.and_utc());
                                            exchange_api.keepalive_user_stream().unwrap();
                                            timestamp_countdown.reset();
                                        }
                                    }
                                }
                            }
                        }
                    }

                    Err(err) => {
                        error!("Error reading message: {:?}", err);
                    }
                }
            }

            // Disconnect
            conn.close().await.expect("Failed to disconnect");
        }
    });

    // Initializing the main structure
    let mut cx = Arc::new(RwLock::new(ContextInstance::new(tokio_runtime, config.clone(), exchange_api.clone())?));
    let coin_manager = cx.read().coin_manager();

    // TODO: propagate shutdown signal and runtime to all workers
    // starting a worker instance for every configured coin
    cx.write().run(favorite_only, graphical_mode)?;

    info!("Running main user stream loop...");
    thread::spawn({
        let cx = cx.clone();

        move || loop {
            match userstream_receiver.recv() {
                Ok(event) => match event {
                    // A message that is sent when an order is created, updated, or canceled
                    WebsocketUserMessage::OrderUpdate(update) => {
                        cx.write().process_order_update(update);
                    }

                    // A message that is sent when the balance of an asset changes
                    WebsocketUserMessage::AccountUpdate(update) => {
                        cx.write().process_account_update(update);
                    }

                    // A message that is sent when the account balance changes
                    // due to a trade, deposit, withdrawal, or other reasons
                    WebsocketUserMessage::BalanceUpdate(update) => {
                        cx.write().process_balance_update(update);
                    }
                },

                Err(err) => {
                    panic!("failed to receive websocket event: {}", err);
                }
            }
        }
    });

    // NOTE: This flag designates whether the bot is running in a graphical mode or not
    if graphical_mode {
        // initializing and running the UI
        let native_options = eframe::NativeOptions {
            resizable: true,

            hardware_acceleration: HardwareAcceleration::Required,
            ..Default::default()
        };

        eframe::run_native("Babylon", native_options, Box::new(|cc| Box::new(App::new(cc, config, cx, coin_manager, ustr("BTCUSDT")))));
    } else {
        shutdown_receiver.recv()?;
    }

    /*
    // waiting for signal to perform a graceful shutdown
    rt.block_on(tokio::select! {
        _ = signal::ctrl_c() => {},
        _ = shutdown_receiver.recv() => {},
    });
     */

    Ok(())
}

fn command_run_playground(config_path: &str) -> Result<()> {
    // Initializing the runtime
    let tokio_runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(num_cpus::get())
        .build()
        .expect("Failed to initialize tokio runtime");

    let config = Config::from_file_with_env("BABYLON", config_path)?;

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    // There is no need to communicate with the exchange in the simulator
    let exchange_api = ExchangeApi::default();

    // TODO: Maybe add a section to the config to specify the simulator config (I'm not sure if it's necessary)
    let mut simulator = Simulator::new(SimulatorConfig {}, GeneratorConfig::default());

    let base_timeframe = simulator.generator().config.base_timeframe;
    let series_capacity = simulator.generator().config.series_capacity;

    simulator.generate_candles(base_timeframe, series_capacity).expect("failed to generate candles");

    // initializing the main structure
    let mut main_context = PlaygroundContextInstance::new(tokio_runtime, config.clone(), exchange_api.clone(), simulator.clone(), true)?;
    let coin_manager = main_context.coin_manager();

    // initializing and running the UI
    let native_options = eframe::NativeOptions {
        resizable: true,
        icon_data: Some(IconData {
            rgba:   vec![],
            width:  0,
            height: 0,
        }),
        ..Default::default()
    };

    eframe::run_native("Babylon Playground", native_options, Box::new(|cc| Box::new(PlaygroundApp::new(cc, config, simulator, main_context, coin_manager))));

    Ok(())
}

fn command_fetch_initial(config_path: &str, symbol: String, timeframes: Option<Vec<Timeframe>>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(symbol.to_uppercase().as_str());

    let coin_config = match config.get_coin(&symbol) {
        None => {
            bail!("No config found for coin {}", symbol);
        }
        Some(coin_config) => coin_config,
    };

    let default_timeframes = config
        .default_timeframe_configs
        .iter()
        .map(|c| c.timeframe)
        .sorted_by(|a, b| a.duration().cmp(&b.duration()))
        .rev()
        .collect_vec();

    let timeframes = match timeframes {
        None => default_timeframes,
        Some(timeframes) if timeframes.is_empty() => default_timeframes,
        Some(timeframes) => timeframes,

        _ => unreachable!(),
    };

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let coin_specific_timeframes = coin_specific_timeframe_configs(&config, coin_config.name);
    let mut cm = CandleManager::new(coin_config, config.defaults.clone(), coin_specific_timeframes.clone(), db, exchange_api, UnifiedRateLimiter::default());

    let it = Instant::now();
    for (timeframe, timeframe_config) in coin_specific_timeframes.iter() {
        let num_candles = timeframe_config.initial_number_of_candles.unwrap_or(config.defaults.initial_number_of_candles);
        let it = Instant::now();
        cm.fetch_initial(num_candles)?;
        info!("{}: Fetched n={} tf={:?} elapsed={:?}", symbol, num_candles, timeframe, it.elapsed());
    }
    info!("{}: Fetched all candles in {:?}", symbol, it.elapsed());

    Ok(())
}

fn command_fetch_updates(config_path: &str, symbol: String, timeframes: Option<Vec<Timeframe>>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(symbol.to_uppercase().as_str());

    let coin_config = match config.get_coin(&symbol) {
        None => {
            bail!("No config found for coin {}", symbol);
        }
        Some(coin_config) => coin_config,
    };

    let default_timeframes = config
        .default_timeframe_configs
        .iter()
        .map(|c| c.timeframe)
        .sorted_by(|a, b| a.duration().cmp(&b.duration()))
        .rev()
        .collect_vec();

    let timeframes = match timeframes {
        None => default_timeframes,
        Some(timeframes) if timeframes.is_empty() => default_timeframes,
        Some(timeframes) => timeframes,

        _ => unreachable!(),
    };

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let coin_specific_timeframes = coin_specific_timeframe_configs(&config, coin_config.name);
    let mut cm = CandleManager::new(coin_config, config.defaults.clone(), coin_specific_timeframes.clone(), db, exchange_api, UnifiedRateLimiter::default());

    let it = Instant::now();
    for (timeframe, timeframe_config) in coin_specific_timeframes.iter() {
        let it = Instant::now();
        cm.fetch_updates(timeframe)?;
        info!("{}: Fetched tf={:?} elapsed={:?}", symbol, timeframe, it.elapsed());
    }
    info!("{}: Fetched all candles in {:?}", symbol, it.elapsed());

    Ok(())
}

fn command_update(config_path: &str, symbol: String, timeframes: Option<Vec<Timeframe>>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(symbol.to_uppercase().as_str());

    let coin_config = match config.get_coin(&symbol) {
        None => {
            bail!("No config found for coin {}", symbol);
        }
        Some(coin_config) => coin_config,
    };

    let timeframes = match timeframes {
        None => config
            .default_timeframe_configs
            .iter()
            .map(|c| c.timeframe)
            .sorted_by(|a, b| a.duration().cmp(&b.duration()))
            .rev()
            .collect_vec(),

        Some(timeframes) => {
            if timeframes.is_empty() {
                bail!("No timeframes specified");
            }

            timeframes
        }
    };

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let coin_specific_timeframes = coin_specific_timeframe_configs(&config, coin_config.name);
    let mut cm = CandleManager::new(coin_config, config.defaults.clone(), coin_specific_timeframes.clone(), db, exchange_api, UnifiedRateLimiter::default());

    info!("Loading candles for {} on {:?}", symbol, coin_specific_timeframes);

    for (timeframe, timeframe_config) in coin_specific_timeframes.iter() {
        // FIXME: fix `update` method
        // cm.update(timeframe)?;
    }

    Ok(())
}

fn command_update_all(config_path: &str, timeframes: Option<Vec<Timeframe>>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(num_cpus::get())
        .build()?;

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let ratelimiter = UnifiedRateLimiter::default();
    let mut handles = vec![];
    let mut semaphore = Semaphore::new(15);

    for coin_config in config.coins.iter().cloned() {
        if !coin_config.is_enabled {
            warn!("{}: Skipping disabled coin", coin_config.name);
            continue;
        }

        let coin_specific_timeframes = coin_specific_timeframe_configs(&config, coin_config.name);

        info!("{}: Loading candles for: {}", coin_config.name, coin_specific_timeframes.iter().map(|(tf, _)| tf.as_str()).join(", "));

        // IMPORTANT: a call to `iter()` is important because TimeSet has a custom implementation that filters out missing timeframes
        for (timeframe, timeframe_config) in coin_specific_timeframes.iter() {
            let semaphore = semaphore.clone();
            info!("Updating candles for {} on {:?}", coin_config.name, timeframe);

            ratelimiter.wait_for_request_weight_limit(2);

            handles.push(thread::spawn({
                let ratelimiter = ratelimiter.clone();
                semaphore.acquire();

                let mut cm = CandleManager::new(
                    coin_config.clone(),
                    config.defaults.clone(),
                    coin_specific_timeframes.clone(),
                    db.clone(),
                    exchange_api.clone(),
                    ratelimiter.clone(),
                );

                move || {
                    // FIXME: fix `update` method
                    /*
                    match cm.update(timeframe) {
                        Ok(_) => {
                            semaphore.release();
                            info!("Finished updating candles for {} on {:?}", coin_config.name, timeframe);
                        }
                        Err(e) => {
                            semaphore.release();
                            error!("Error updating candles for {} on {:?}: {}", coin_config.name, timeframe, e);
                        }
                    }
                     */
                }

                /*
                move || {
                    ratelimiter.wait_for_raw_request_limit(1);
                    ratelimiter.wait_for_request_weight_limit(1);
                    sleep(Duration::from_millis(100));
                    semaphore.release();
                }
                 */
            }));
        }
    }

    for handle in handles {
        handle.join().unwrap();
    }

    Ok(())
}

fn command_rollback(config_path: &str, symbol: Option<String>, timeframes: Option<Vec<Timeframe>>, period: String, from_most_recent: bool) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = symbol.map(|s| ustr(s.to_uppercase().as_str()));

    let period = match humantime::parse_duration(period.as_str()) {
        Ok(d) => d,
        Err(e) => {
            bail!("Invalid duration: {}", e);
        }
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(num_cpus::get())
        .build()?;

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let ratelimiter = UnifiedRateLimiter::default();
    let mut handles = vec![];

    for coin_config in config.coins.iter().cloned() {
        if let Some(symbol) = symbol.clone() {
            if coin_config.name != symbol {
                continue;
            }
        }

        if !coin_config.is_enabled {
            warn!("{}: Skipping disabled coin", coin_config.name);
            continue;
        }

        // FIXME: respect symbol argument
        let coin_specific_timeframes = coin_specific_timeframe_configs(&config, coin_config.name);

        info!("{}: Rolling back by {:?}: {}", coin_config.name, period, coin_specific_timeframes.iter().map(|(tf, _)| tf.as_str()).join(", "));

        for (timeframe, timeframe_config) in coin_specific_timeframes.iter() {
            handles.push(thread::spawn({
                let mut cm = CandleManager::new(
                    coin_config.clone(),
                    config.defaults.clone(),
                    coin_specific_timeframes.clone(),
                    db.clone(),
                    exchange_api.clone(),
                    ratelimiter.clone(),
                );

                move || match cm.rollback(
                    coin_config.name,
                    timeframe,
                    chrono::Duration::from_std(period).expect("failed to convert standard duration to chrono"),
                    from_most_recent,
                ) {
                    Ok(_) => {
                        info!("{}: Finished rolling back on {:?}", coin_config.name, timeframe);
                    }
                    Err(e) => {
                        error!("{}: Error updating candles on {:?}: {}", coin_config.name, timeframe, e);
                    }
                }
            }));
        }
    }

    for handle in handles {
        handle.join().unwrap();
    }

    Ok(())
}

fn command_inspect(config_path: &str, symbol: Option<String>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = symbol.map(|s| ustr(s.to_uppercase().as_str()));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(num_cpus::get())
        .build()?;

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let ratelimiter = UnifiedRateLimiter::default();

    let symbols = match symbol {
        None => config.coins.iter().map(|c| c.name.clone()).collect_vec(),
        Some(symbol) => vec![ustr(symbol.as_str())],
    };

    for symbol in symbols {
        let coin_config = match config.coins.iter().find(|c| c.name == symbol) {
            None => {
                bail!("No config found for coin {}", symbol);
            }
            Some(coin_config) => coin_config,
        };

        // get coin specific timeframes or use global timeframes
        let timeframes = match &coin_config.timeframes {
            None => config
                .default_timeframe_configs
                .iter()
                .map(|c| c.timeframe)
                .sorted_by(|a, b| a.duration().cmp(&b.duration()))
                .rev()
                .collect_vec(),
            Some(timeframes) => timeframes.clone(),
        };

        let coin_specific_timeframe_configs = coin_specific_timeframe_configs(&config, coin_config.name);
        let mut cm = CandleManager::new(
            coin_config.clone(),
            config.defaults.clone(),
            coin_specific_timeframe_configs,
            db.clone(),
            exchange_api.clone(),
            ratelimiter.clone(),
        );

        println!("{}:", coin_config.name);
        for timeframe in timeframes {
            let name = coin_config.name.clone();

            match cm.inspect(timeframe) {
                Ok(state) => match state {
                    CandleManagerState::Empty => {
                        warn!("{}: Empty", timeframe.to_string());
                    }
                    CandleManagerState::Actual => {
                        info!("{}: Actual", timeframe.to_string());
                    }
                    CandleManagerState::Outdated {
                        latest_candle_open_time_utc,
                        time_passed,
                    } => {
                        warn!("{}: Outdated", timeframe.to_string(),);
                    }
                },
                Err(e) => {
                    error!("Error inspecting candles for {} on {:?}: {}", name, timeframe, e);
                }
            }
        }
    }

    Ok(())
}

fn command_list(config_path: &str, simple: bool, favorite_only: bool) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;

    let mut db = Database::new(config.database.clone())?;
    let coins = if favorite_only { config.coins.iter().filter(|c| c.is_favorite.unwrap_or_default()).cloned().collect_vec() } else { config.coins.clone() };

    // Determine timeframes assuming all coins have the same set of timeframes
    let (_, example_counts) = db.count_candles_per_timeframe(coins[0].name)?;
    let timeframes = example_counts.iter().map(|(timeframe, _)| timeframe.as_str()).collect::<Vec<&str>>();
    let coin_count = coins.len();
    let mut header;

    if !simple {
        header = format!("Symbol | {: <15}", timeframes.join(" | "));
    }

    // Prepare counts
    for coin_config in coins.iter() {
        if !simple {
            let (coin_name, counts) = db.count_candles_per_timeframe(coin_config.name)?;
            let counts_str: Vec<String> = counts.iter().map(|(_, count)| count.to_string()).collect();
            println!("{0: <15} | {1: <15}", coin_name, counts_str.join(" | "));
        } else {
            println!("{}", coin_config.name.as_str());
        }
    }

    println!("\n{} coins", coin_count);

    Ok(())
}

fn command_fix_gaps(config_path: &str, symbol: String, timeframes: Option<Vec<Timeframe>>) -> Result<()> {
    let config = Config::from_file_with_env("BABYLON", config_path)?;
    let symbol = ustr(symbol.to_uppercase().as_str());

    let coin_config = match config.get_coin(&symbol) {
        None => {
            bail!("No config found for coin {}", symbol);
        }
        Some(coin_config) => coin_config,
    };

    let timeframes = match timeframes {
        None => config
            .default_timeframe_configs
            .iter()
            .map(|c| c.timeframe)
            .sorted_by(|a, b| a.duration().cmp(&b.duration()))
            .rev()
            .collect_vec(),

        Some(timeframes) => {
            if timeframes.is_empty() {
                bail!("No timeframes specified");
            }

            timeframes
        }
    };

    // channel to handle graceful shutdown
    let (shutdown_sender, mut shutdown_receiver) = bounded::<()>(1);

    let exchange_api = ExchangeApi::new(Some(config.exchange.key.clone()), Some(config.exchange.secret.clone()));
    let db = Database::new(config.database.clone())?;
    let coin_specific_timeframe_configs = coin_specific_timeframe_configs(&config, coin_config.name);
    let mut cm = CandleManager::new(coin_config, config.defaults, coin_specific_timeframe_configs, db, exchange_api, UnifiedRateLimiter::default());

    info!("Fixing gaps for {} on {:?}", symbol, timeframes);

    for timeframe in timeframes {
        cm.fix_gaps(timeframe)?;
    }

    Ok(())
}

fn command_run_experiment(config_path: &str) -> Result<()> {
    use binance_spot_connector_rust::{market::klines::KlineInterval, market_stream::kline::KlineStream, tokio_tungstenite::BinanceWebSocketClient};
    use env_logger::Builder;
    use futures_util::StreamExt;

    let rt = Runtime::new()?;

    rt.block_on(async {
        // Establish connection
        let (mut conn, _) = BinanceWebSocketClient::connect_async_default().await.expect("Failed to connect");

        // Subscribe to streams
        conn.subscribe(vec![
            &KlineStream::new("BTCUSDT", KlineInterval::Seconds1).into(),
            &TradeStream::new("BTCUSDT").into(),
            &DiffDepthStream::from_100ms("BTCUSDT").into(),
            &PartialDepthStream::from_100ms("BTCUSDT", 20).into(),
        ])
        .await;

        // Read messages
        while let Some(message) = conn.as_mut().next().await {
            match message {
                Ok(message) => {
                    let Ok(WebsocketResponse { stream, data }) = serde_json::from_str(message.to_text().unwrap()) else {
                        println!("Failed to parse message: {}", message.to_text().unwrap());
                        continue;
                    };

                    match data {
                        WebsocketMessage::Kline(c) => {
                            println!("{:#?}", c);
                        }
                        WebsocketMessage::ExchangeTrade(t) => {
                            println!("{:#?}", t);
                        }

                        WebsocketMessage::DiffDepth(d) => {
                            println!("{:#?}", d);
                        }

                        WebsocketMessage::PartialDepth(p) => {
                            println!("{:#?}", p);
                        }

                        _ => {
                            println!("{:?}", data);
                        }
                    }
                }
                Err(err) => {
                    error!("Error reading message: {:?}", err);
                }
            }
        }

        // Disconnect
        conn.close().await.expect("Failed to disconnect");
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_coin() {
        let original_config_path = "config.toml";
        let copied_config_path = "config.test.toml";

        // Copying the default configuration file to a temporary file
        fs::copy(original_config_path, copied_config_path).unwrap();

        // Loading the configuration from the file
        let mut original_config = Config::from_file_with_env("BABYLON", copied_config_path).unwrap();

        let name = "TESTUSDT".to_string();
        let is_enabled = Some(true);
        let is_favorite = Some(true);
        let timeframes = Some(vec!["1m".to_string(), "5m".to_string(), "15m".to_string()]);

        command_add_coin(copied_config_path, name.clone(), is_enabled, is_favorite, timeframes.clone()).unwrap();

        // Reloading the configuration from the newly created test config file
        let reloaded_config = Config::from_file_with_env("BABYLON", copied_config_path).unwrap();
        let coin_config = reloaded_config.get_coin(&name).unwrap();

        assert_eq!(coin_config.name, name);
        assert_eq!(coin_config.is_enabled, true);
        assert_eq!(coin_config.is_favorite, is_favorite);
        assert_eq!(coin_config.timeframes, Some(timeframes.unwrap().into_iter().map(|s| Timeframe::from(s)).collect_vec()));
    }

    #[test]
    fn test_delete_coin() {
        let original_config_path = "config.toml";
        let copied_config_path = "config.test.toml";

        // Copying the default configuration file to a temporary file
        fs::copy(original_config_path, copied_config_path).unwrap();

        let coin_name = "TESTUSDT";

        // Add a coin first to ensure it exists
        command_add_coin(copied_config_path, coin_name.to_string(), Some(true), Some(true), Some(vec!["1m".to_string()])).unwrap();

        // Now delete the coin
        command_delete_coin(copied_config_path, coin_name.to_string()).unwrap();

        // Load the config file and check if the coin was deleted
        let config = Config::from_file_with_env("BABYLON", copied_config_path).unwrap();
        assert!(config.coins.iter().find(|c| c.name == coin_name).is_none());
    }

    #[test]
    fn test_edit_coin() {
        let original_config_path = "config.toml";
        let copied_config_path = "config.test.toml";

        // Copying the default configuration file to a temporary file
        fs::copy(original_config_path, copied_config_path).unwrap();

        let coin_name = "TESTUSDT";

        // Add a coin first to ensure it exists
        command_add_coin(copied_config_path, coin_name.to_string(), Some(true), Some(true), Some(vec!["1m".to_string()])).unwrap();

        // Now edit the coin
        command_edit_coin(copied_config_path, coin_name.to_string(), Some(false), Some(false), Some(vec!["5m".to_string()])).unwrap();

        // Load the config file and check if the coin was edited
        let config = Config::from_file_with_env("BABYLON", copied_config_path).unwrap();
        let coin = config.coins.iter().find(|c| c.name == coin_name).unwrap();
        assert_eq!(coin.is_enabled, false);
        assert_eq!(coin.is_favorite, Some(false));
        assert_eq!(coin.timeframes, Some(vec![Timeframe::from_str("5m").unwrap()]));
    }
}
