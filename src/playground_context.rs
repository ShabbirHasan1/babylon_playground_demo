use std::{
    borrow::Borrow,
    collections::hash_map::Entry,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::{DateTime, NaiveDateTime, Utc};
use crossbeam_channel::{bounded, internal::SelectHandle, unbounded, Receiver, Sender};
use indexmap::IndexMap;
use itertools::Itertools;
use log::{error, info};
use ordered_float::OrderedFloat;
use parking_lot::RwLock;
use rand::{rngs::OsRng, Rng};
use tokio::{runtime, runtime::Runtime};
use yata::prelude::*;

use crate::{
    balance::BalanceSheet,
    coin_manager::CoinManager,
    coin_worker::CoinWorker,
    f,
    types::{DataPoint, NormalizedExecutionType, NormalizedOrderEvent, NormalizedOrderType, NormalizedTimeInForce},
    util::s2of,
    Config,
};

use crate::{
    appraiser::Appraiser,
    balance::BalanceError,
    candle_manager::CandleManager,
    candle_series::CandleSeries,
    coin::{Coin, CoinContext},
    coin_worker::CoinWorkerMessage,
    config::CoinConfig,
    context::{Context, ContextError},
    cooldown::Cooldown,
    database::{Database, DatabaseBackend},
    exchange_client::{BinanceExchangeClient, ExchangeClientMessage},
    helpers::{balance_for_testing, symbol_for_testing},
    id::SmartId,
    instruction::{ExecutionOutcome, Instruction, InstructionExecutionResult},
    model::{
        AccountInformation, AccountUpdateEvent, Balance, BalanceUpdateEvent, CommissionRates, ExchangeInformation, ExchangeTradeEvent, OrderUpdateEvent, Rules,
        Symbol,
    },
    order::{OrderFill, OrderFillStatus, OrderState},
    orderbook::OrderBook,
    position::Position,
    ratelimiter::UnifiedRateLimiter,
    simulator::{Simulator, SimulatorError, SimulatorMessage, SimulatorOrderEvent, SimulatorRequestMessage},
    simulator_order::{SimulatorOrder, SimulatorOrderMode},
    sliding_window::SlidingWindow,
    timeframe::Timeframe,
    timeset::TimeSet,
    trade::TradeError,
    trader::{Trader, TraderError, TraderSymbolContext},
    types::{ExchangeApi, NormalizedCommission, NormalizedOrderEventSource, NormalizedOrderStatus, NormalizedSide},
    util::{coin_specific_timeframe_configs, new_id_u64, timeframe_configs_to_timeset},
};
use ustr::{ustr, Ustr, UstrMap};
use yata::core::Window;

pub struct PlaygroundContextInstance {
    tokio_runtime:          Runtime,
    config:                 Config,
    exchange_api:           ExchangeApi,
    exchange_information:   Arc<ExchangeInformation>,
    account:                Arc<AccountInformation>,
    balance:                BalanceSheet,
    ratelimiter:            UnifiedRateLimiter,
    coins:                  UstrMap<CoinWorker>,
    coin_manager:           Arc<RwLock<CoinManager>>,
    trader:                 Trader,
    db:                     Database,
    coin_worker_sender:     Sender<CoinWorkerMessage>,
    exchange_client_sender: Sender<ExchangeClientMessage>,
    datapoint_sender:       Sender<DataPoint>,
}

/// Playground is a context that is used for testing and debugging purposes.
impl PlaygroundContextInstance {
    pub fn new(tokio_runtime: Runtime, config: Config, exchange_api: ExchangeApi, simulator: Simulator, graphical_mode: bool) -> Result<Self> {
        let symbol = ustr("TESTUSDT");
        let config = Config::from_file_with_env("BABYLON", "config.toml").unwrap();
        let db = Database::new(config.database.clone()).expect("Failed to initialize database object");
        let timeframe_configs = timeframe_configs_to_timeset(config.default_timeframe_configs.clone());
        let timeframe = Timeframe::M1;
        let coin_config = config.coins.iter().next().unwrap().clone();
        let mut cm =
            CandleManager::new(coin_config, config.defaults.clone(), timeframe_configs, db.clone(), exchange_api.clone(), UnifiedRateLimiter::default());
        let ratelimiter = UnifiedRateLimiter::default();
        let mut coin_manager = CoinManager::new();
        let mut balance = balance_for_testing();
        let market_price = simulator.atomic_market_price();

        // Initializing rules and appraiser
        let symbol_metadata = symbol_for_testing();
        let appraiser = Appraiser::new(symbol_metadata.rules.clone());

        // WARNING: Dummy commission
        let commission = NormalizedCommission {
            maker:  Some(OrderedFloat(0.001)),
            taker:  Some(OrderedFloat(0.001)),
            buyer:  None,
            seller: None,
        };

        // Initializing channels outside their respective workers
        let (exchange_client_sender, exchange_client_receiver) = unbounded::<ExchangeClientMessage>();
        let (datapoint_sender, datapoint_receiver) = unbounded::<DataPoint>();
        let (coin_worker_sender, coin_worker_receiver) = unbounded::<CoinWorkerMessage>();
        let (simulator_request_sender, simulator_request_receiver) = bounded::<SimulatorRequestMessage>(16);
        let (simulator_feedback_sender, simulator_feedback_receiver) = bounded::<SimulatorMessage>(16);

        // Initializing a global trader mechanism
        let trader = Trader::new(balance.clone(), exchange_client_sender.clone(), datapoint_sender.clone());

        // Initializing workers for the simulation
        Self::start_simulator_exchange_client_worker(exchange_client_receiver, simulator_request_sender, coin_worker_sender.clone());
        Self::start_simulator_worker(simulator.clone(), simulator_request_receiver, simulator_feedback_sender.clone());
        Self::start_simulator_datapoint_worker(config.clone(), db.clone(), datapoint_receiver);
        Self::start_simulator_coin_worker(coin_manager.clone(), trader.clone(), simulator_feedback_receiver, coin_worker_receiver);

        // Creating a dummy trader's internal coin context
        trader.create_context(TraderSymbolContext {
            symbol,
            quote_asset: ustr("USDT"),
            base_asset: ustr("TEST"),
            atomic_market_price: simulator.atomic_market_price().clone(),
            entry_cooldown: Cooldown::new(Duration::from_millis(100), 1),
            exit_cooldown: Cooldown::new(Duration::from_millis(100), 1),
            appraiser,
            commission,
            max_quote_allocated: f!(1000.0),
            min_quote_per_grid: OrderedFloat(100.0),
            max_quote_per_grid: OrderedFloat(300.0),
            max_orders: 1,
            max_orders_per_cooldown: 1,
            max_retry_attempts: 1,
            retry_delay: Duration::from_millis(300),
            entry_grid_orders: 5,
            exit_grid_orders: 5,
            initial_stop_ratio: OrderedFloat(0.05),
            stop_limit_offset_ratio: OrderedFloat(0.0015),
            min_order_spacing_ratio: OrderedFloat(0.005),
            initial_trailing_step_ratio: OrderedFloat(0.005),
            trailing_step_ratio: OrderedFloat(0.001),
            baseline_risk_ratio: Default::default(),
            position: Position::new(ustr("TESTUSDT"), balance.clone()),
            orders: Default::default(),
            trades: IndexMap::new(),
            is_paused: false,
            is_zero_maker_fee: false,
            is_zero_taker_fee: false,
            is_iceberg_allowed_by_exchange: true,
            is_iceberg_trading_enabled: false,
            is_spot_trading_allowed_by_exchange: true,
            is_margin_trading_allowed_by_exchange: true,
            is_long_trading_enabled: false,
            is_short_trading_enabled: false,
            stats: Default::default(),
            max_orders_per_trade: 1,
        });

        // This is the client that will be used to communicate with the exchange
        // FIXME: Implement a mock exchange client for simulation
        let exchange_client = BinanceExchangeClient::new(exchange_api.clone(), ratelimiter.clone(), trader.clone());

        // WARNING: running the exchange client in a separate thread
        // thread::spawn(move || exchange_client_worker(exchange_client, exchange_client_receiver));

        /*
        // Dummy account information
        let account_info = Arc::new(AccountInformation {
            maker_commission:  0.0,
            taker_commission:  0.0,
            buyer_commission:  0.0,
            seller_commission: 0.0,
            can_trade:         true,
            can_withdraw:      false,
            can_deposit:       false,
            balances:          vec![binance::model::Balance {
                asset:  "TESTUSDT".to_string(),
                free:   "10000.0".to_string(),
                locked: "0.0".to_string(),
            }],
        });
         */

        let account = Arc::new(AccountInformation {
            maker_commission:              0,
            taker_commission:              0,
            buyer_commission:              0,
            seller_commission:             0,
            commission_rates:              CommissionRates {
                maker:  f!(0.0),
                taker:  f!(0.0),
                buyer:  f!(0.0),
                seller: f!(0.0),
            },
            can_trade:                     true,
            can_withdraw:                  false,
            can_deposit:                   false,
            brokered:                      false,
            require_self_trade_prevention: false,
            prevent_sor:                   false,
            update_time:                   Default::default(),
            account_type:                  Default::default(),
            balances:                      vec![
                Balance::new(ustr("TEST"), f!(10000.0), f!(0.0)),
                Balance::new(ustr("USDT"), f!(10000.0), f!(0.0)),
            ],
            permissions:                   vec![],
            uid:                           0,
        });

        // Pushing dummy balance
        balance.set_free(ustr("USDT"), f!(10000.0));

        // Initializing aggregated account balance
        account.balances.iter().for_each(|b| {
            balance.set_free(b.asset, b.free);
            balance.set_locked(b.asset, b.locked);
        });

        // ----------------------------------------------------------------------------
        // WARNING: Registering a dummy coin for the simulation
        // ----------------------------------------------------------------------------
        let symbol = ustr("TESTUSDT");

        // Initializing the series
        let mut multiseries = TimeSet::<CandleSeries>::new();

        // Populating the series with candles from the simulator
        for (timeframe, generator_state) in simulator.generator().timeframes.iter() {
            let capacity = config.main.series_capacity;
            let num_candles = generator_state.candles.num_candles();

            for (i, c) in generator_state.candles.iter_candles_back_to_front().enumerate() {
                if i == 0 {
                    let mut series = CandleSeries::new(symbol, timeframe, None, capacity, num_candles, false, false, graphical_mode);
                    series.push_candle(*c);
                    multiseries.set(timeframe, series);
                } else {
                    multiseries.get_mut(timeframe)?.push_candle(*c);
                }
            }
        }

        coin_manager.register_coin(
            symbol,
            Arc::new(RwLock::new(Coin {
                coin_context:           Arc::new(RwLock::new(CoinContext {
                    symbol: symbol,
                    account_info: account.clone(),
                    config: CoinConfig::dummy(),
                    timeframe_configs: Default::default(),
                    symbol_metadata: Symbol {
                        symbol,
                        status: ustr("TRADING"),
                        base_asset: ustr("TEST"),
                        base_asset_precision: 8,
                        quote_asset: ustr("USDT"),
                        quote_precision: 8,
                        quote_asset_precision: 8,
                        base_commission_precision: 8,
                        quote_commission_precision: 8,
                        order_types: vec![
                            NormalizedOrderType::Limit,
                            NormalizedOrderType::LimitMaker,
                            NormalizedOrderType::Market,
                            NormalizedOrderType::StopLossLimit,
                            NormalizedOrderType::TakeProfitLimit,
                        ],
                        iceberg_allowed: true,
                        oco_allowed: true,
                        quote_order_qty_market_allowed: true,
                        allow_trailing_stop: true,
                        cancel_replace_allowed: true,
                        is_spot_trading_allowed: true,
                        is_margin_trading_allowed: true,
                        rules: Rules::default(),
                        permissions: vec![],
                        default_self_trade_prevention_mode: Default::default(),
                        allowed_self_trade_prevention_modes: vec![],
                    },
                    rules: symbol_metadata.rules.clone(),
                    appraiser,
                    atomic_price: Arc::new(Default::default()),
                    balance: balance.clone(),
                })),
                orderbook:              OrderBook::new(symbol, appraiser, 0, Default::default(), Default::default()),
                trades:                 SlidingWindow::<ExchangeTradeEvent>::new(0),
                candle_manager:         cm,
                quote_asset:            ustr("USDT"),
                base_asset:             ustr("TEST"),
                series:                 multiseries,
                trade_timeframe:        Default::default(),
                ratelimiter:            ratelimiter.clone(),
                manager:                Arc::new(Default::default()),
                trader:                 trader.clone(),
                exchange_client_sender: exchange_client_sender.clone(),
                coin_worker_sender:     coin_worker_sender.clone(),
                datapoint_sender:       datapoint_sender.clone(),
                buy_cooldown:           Cooldown::new(Duration::from_secs(1), 1),
                sell_cooldown:          Cooldown::new(Duration::from_secs(1), 1),
                uptime:                 Instant::now(),
                is_loaded:              false,
            })),
        );

        // Load exchange information snapshot from JSON file
        let exchange_info = Arc::new(
            serde_json::from_str::<ExchangeInformation>(
                std::fs::read_to_string("testdata/exchange_information.json")
                    .expect("Failed to read exchange info file")
                    .as_str(),
            )
            .expect("Failed to parse exchange info file"),
        );

        /*
        // Initializing a global trader mechanism
        let trader = Trader::new(exchange_client_sender.clone(), datapoint_sender.clone());

        // This is the client that will be used to communicate with the exchange
        let exchange_client = ExchangeClient::new(exchange_api.clone(), ratelimiter.clone(), trader.clone());
         */

        Ok(Self {
            tokio_runtime,
            exchange_information: exchange_info,
            account: account,
            coins: Default::default(),
            ratelimiter: ratelimiter,
            db,
            config,
            coin_manager: Arc::new(RwLock::new(coin_manager)),
            exchange_api,
            datapoint_sender,
            balance,
            exchange_client_sender,
            trader,
            coin_worker_sender,
        })
    }

    fn start_simulator_worker(
        mut simulator: Simulator,
        simulator_request_receiver: Receiver<SimulatorRequestMessage>,
        simulator_feedback_sender: Sender<SimulatorMessage>,
    ) {
        // ----------------------------------------------------------------------------
        // This thread is simulating the exchange and is responsible for handling
        // incoming requests from the application.
        //
        // It responds in 2 ways:
        // 1. By sending a response message to the application (via `response_sender`)
        // 2. By sending a message to the coin worker (via `simulator_feedback_sender`)
        // ----------------------------------------------------------------------------

        thread::spawn({
            let mut simulator = simulator.clone();

            info!("Simulator worker started");

            move || loop {
                match simulator_request_receiver.recv_timeout(Duration::from_millis(1)) {
                    Ok(message) => match message {
                        SimulatorRequestMessage::CreateOrder {
                            symbol,
                            client_order_id,
                            client_limit_order_id,
                            order_type,
                            side,
                            price,
                            limit_price,
                            quantity,
                            response_sender,
                        } => {
                            info!("SIMULATOR_RECEIVER: Received CreateOrder request");

                            let new_order = SimulatorOrder {
                                id: new_id_u64(),
                                other_order_id: None,
                                is_cancelled_by_other_order: false,
                                mode: SimulatorOrderMode::Regular,
                                client_order_id,
                                client_limit_order_id,
                                client_list_order_id: None,
                                client_stop_order_id: None,
                                client_cancel_order_id: None,
                                client_original_order_id: None,
                                symbol,
                                side,
                                order_type,
                                time_in_force: NormalizedTimeInForce::GTC,
                                price,
                                limit_price,
                                initial_quantity: quantity,
                                filled_quantity: f!(0.0),
                                fills: vec![],
                                state: OrderState::Idle,
                                atomic_market_price: simulator.atomic_market_price(),
                                replace_with_on_cancel: None,
                                events: vec![],
                            };

                            response_sender.send(simulator.create_order(new_order)).unwrap();
                        }

                        SimulatorRequestMessage::CreateOcoOrder {
                            symbol,
                            list_client_order_id,
                            stop_client_order_id,
                            limit_client_order_id,
                            side,
                            quantity,
                            price,
                            stop_price,
                            stop_limit_price,
                            stop_limit_time_in_force,
                            response_sender,
                        } => {
                            info!("SIMULATOR_RECEIVER: Received CreateOcoOrder request");
                            response_sender
                                .send(simulator.create_oco_order(
                                    symbol,
                                    list_client_order_id,
                                    stop_client_order_id,
                                    limit_client_order_id,
                                    side,
                                    quantity,
                                    price,
                                    stop_price,
                                    stop_limit_price,
                                    stop_limit_time_in_force,
                                ))
                                .unwrap();
                        }

                        SimulatorRequestMessage::CancelReplaceOrder {
                            symbol,
                            exchange_order_id,
                            cancel_order_id,
                            original_client_order_id,
                            new_client_order_id,
                            new_order_type,
                            new_time_in_force,
                            new_price,
                            new_limit_price,
                            new_quantity,
                            response_sender,
                        } => {
                            info!("SIMULATOR_RECEIVER: Received CancelReplaceOrder request");

                            response_sender
                                .send(simulator.cancel_replace_order(
                                    symbol,
                                    exchange_order_id,
                                    cancel_order_id,
                                    original_client_order_id,
                                    new_client_order_id,
                                    new_order_type,
                                    new_time_in_force,
                                    new_quantity,
                                    new_price,
                                    new_limit_price,
                                ))
                                .unwrap();
                        }

                        SimulatorRequestMessage::CancelOrder {
                            symbol,
                            client_order_id: order_id,
                            response_sender,
                        } => {
                            info!("SIMULATOR_RECEIVER: Received CancelOrder request");
                            response_sender.send(simulator.cancel_order(symbol, order_id, false)).unwrap();
                        }

                        SimulatorRequestMessage::CancelAllOpenOrders { .. } => {
                            todo!("CancelAllOrders message type is not implemented");
                        }

                        SimulatorRequestMessage::GenerateCandles {
                            timeframe,
                            count,
                            response_sender,
                        } => {
                            info!("SIMULATOR_RECEIVER: Received GenerateCandles request: {:?} {}", timeframe, count);
                            let it = Instant::now();
                            response_sender.send(simulator.generate_candles(timeframe, count)).unwrap();
                            info!("SIMULATOR_RECEIVER: Generated {} candles in {:?}", count, it.elapsed());
                        }
                        SimulatorRequestMessage::CancelAllOrdersForSymbol { .. } => {}
                    },

                    Err(err) => {
                        // error!("SIMULATOR_RECEIVER ERROR: {:#?}", err);
                        // return;
                    }
                }

                // ----------------------------------------------------------------------------
                // Computing simulator state and handling events
                // ----------------------------------------------------------------------------
                let (events, candles) = simulator.compute().expect("failed to compute simulator state");

                // TODO: Consider using a single response type for all events when anything is changed in the order
                for event in events {
                    match event {
                        SimulatorOrderEvent::OrderEvent(event) => {
                            info!("SIMULATOR EVENT: OrderEvent: {:#?}", event);
                            simulator_feedback_sender.send(SimulatorMessage::OrderEvent(event)).unwrap();
                        }
                        SimulatorOrderEvent::Error { code, msg, local_description } => {
                            simulator_feedback_sender
                                .send(SimulatorMessage::Error { code, msg, local_description })
                                .unwrap();
                        }
                    }
                }

                if simulator_feedback_sender.is_ready() {
                    for candle in candles {
                        simulator_feedback_sender.send(SimulatorMessage::CandleEvent(candle)).unwrap();
                    }
                }
            }
        });
    }

    fn start_simulator_exchange_client_worker(
        exchange_client_receiver: Receiver<ExchangeClientMessage>,
        simulator_request_sender: Sender<SimulatorRequestMessage>,
        coin_worker_sender: Sender<CoinWorkerMessage>,
    ) {
        // ----------------------------------------------------------------------------
        // NOTE: This is a mock exchange client worker that is used for testing
        // This thread is responsible for receiving instructions from the application
        // and sending requests to the simulator worker (which is a mock of the exchange)
        // IMPORTANT: The simulator worker response messages are received by the coin worker
        // ----------------------------------------------------------------------------

        thread::spawn({
            info!("Exchange client worker started (SIMULATION)");

            move || loop {
                // FIXME: simulate latency
                // Simulating latency
                let latency =
                    if OsRng.gen_ratio(1, 10) { Duration::from_millis(OsRng.gen_range(1..=30)) } else { Duration::from_micros(OsRng.gen_range(300..=1000)) };

                // Receiving instructions from the application
                match exchange_client_receiver.recv() {
                    Ok(message) => match message {
                        ExchangeClientMessage::Instruction(instruction_id, metadata, instruction, priority) => match instruction {
                            Instruction::LimitBuy {
                                symbol,
                                local_order_id,
                                order_type,
                                quantity,
                                price,
                                is_post_only,
                            } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing LimitBuy instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CreateOrder {
                                        symbol,
                                        client_order_id: local_order_id,
                                        client_limit_order_id: None,
                                        order_type: if is_post_only { NormalizedOrderType::LimitMaker } else { NormalizedOrderType::Limit },
                                        side: NormalizedSide::Buy,
                                        price,
                                        limit_price: None,
                                        quantity,
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::LimitBuy): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            Instruction::LimitSell {
                                symbol,
                                local_order_id,
                                order_type,
                                quantity,
                                price,
                                is_post_only,
                            } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing LimitSell instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CreateOrder {
                                        symbol,
                                        client_order_id: local_order_id,
                                        client_limit_order_id: None,
                                        order_type: if is_post_only { NormalizedOrderType::LimitMaker } else { NormalizedOrderType::Limit },
                                        side: NormalizedSide::Sell,
                                        price,
                                        limit_price: None,
                                        quantity,
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::LimitSell): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            Instruction::MarketBuy {
                                symbol,
                                local_order_id,
                                quantity,
                            } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing MarketBuy instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CreateOrder {
                                        symbol,
                                        client_order_id: local_order_id,
                                        client_limit_order_id: None,
                                        order_type: NormalizedOrderType::Market,
                                        side: NormalizedSide::Buy,
                                        price: f!(0.0),
                                        limit_price: None,
                                        quantity,
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::MarketBuy): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            Instruction::CreateStop {
                                symbol,
                                order_type,
                                side,
                                local_order_id: local_stop_order_id,
                                local_stop_order_id: local_stop_limit_order_id,
                                stop_price,
                                price: limit_price,
                                quantity,
                            } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing CreateStop instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CreateOrder {
                                        symbol,
                                        client_order_id: local_stop_order_id,
                                        client_limit_order_id: local_stop_limit_order_id,
                                        order_type,
                                        side,
                                        price: stop_price,
                                        limit_price: limit_price,
                                        quantity,
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::CreateStop): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            Instruction::CreateOcoOrder {
                                symbol,
                                list_client_order_id,
                                stop_client_order_id,
                                limit_client_order_id,
                                side,
                                quantity,
                                price,
                                stop_price,
                                stop_limit_price,
                                stop_limit_time_in_force,
                            } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing CreateOcoOrder instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CreateOcoOrder {
                                        symbol,
                                        list_client_order_id,
                                        stop_client_order_id,
                                        limit_client_order_id,
                                        side,
                                        quantity,
                                        price,
                                        stop_price,
                                        stop_limit_price,
                                        stop_limit_time_in_force: Some(stop_limit_time_in_force),
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::CreateOcoOrder): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            Instruction::CancelOrder { symbol, local_order_id } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing CancelOrder instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CancelOrder {
                                        symbol,
                                        client_order_id: local_order_id,
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::CancelOrder): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            Instruction::CancelReplaceOrder {
                                symbol,
                                exchange_order_id,
                                cancel_order_id,
                                new_client_order_id,
                                new_order_type,
                                local_original_order_id,
                                new_time_in_force,
                                new_quantity,
                                new_price,
                                new_limit_price,
                            } => {
                                info!("EXCHANGE_CLIENT_RECEIVER: Executing CancelReplaceOrder instruction");
                                let (response_sender, response_receiver) = unbounded::<Result<(), SimulatorError>>();

                                simulator_request_sender
                                    .send(SimulatorRequestMessage::CancelReplaceOrder {
                                        symbol,
                                        exchange_order_id,
                                        cancel_order_id,
                                        original_client_order_id: Some(local_original_order_id),
                                        new_client_order_id,
                                        new_order_type,
                                        new_time_in_force,
                                        new_price,
                                        new_limit_price,
                                        new_quantity,
                                        response_sender,
                                    })
                                    .unwrap();

                                match response_receiver.recv().unwrap() {
                                    Ok(()) => {
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Success),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                    Err(err) => {
                                        error!("EXCHANGE_CLIENT_RECEIVER (Instruction::CancelReplaceOrder): Request failed: {:#?}", err);
                                        coin_worker_sender
                                            .send(CoinWorkerMessage::ExecutedInstruction(InstructionExecutionResult {
                                                instruction_id: instruction_id,
                                                metadata:       metadata,
                                                symbol:         Some(symbol),
                                                outcome:        Some(ExecutionOutcome::Failure(err.to_string())),
                                                executed_at:    Some(Utc::now()),
                                                execution_time: Some(Duration::from_micros(10)),
                                            }))
                                            .unwrap();
                                    }
                                }
                            }

                            _ => {
                                info!("EXCHANGE_CLIENT_RECEIVER_ERROR: Unexpected instruction type");
                                println!("{:#?}", instruction);
                                return;
                            }
                        },

                        _ => {
                            info!("EXCHANGE_CLIENT_RECEIVER_ERROR: Unexpected message type");
                            return;
                        }
                    },
                    Err(err) => {
                        error!("EXCHANGE_CLIENT_RECEIVER ERROR: {:#?}", err);
                        return;
                    }
                }
            }
        });
    }

    fn start_simulator_coin_worker(
        mut coin_manager: CoinManager,
        mut trader: Trader,
        simulator_feedback_receiver: Receiver<SimulatorMessage>,
        coin_worker_receiver: Receiver<CoinWorkerMessage>,
    ) {
        // ----------------------------------------------------------------------------
        // IMPORTANT:
        // Mocking the coin worker message processor
        // This thread is responsible for processing messages from the simulator worker
        // and reflecting the changes in the trader's internal state (orders, trades, etc.)
        // NOTE: Simulator is a mock of the exchange
        // ----------------------------------------------------------------------------
        thread::spawn({
            info!("Coin worker started (SIMULATION)");

            move || loop {
                match simulator_feedback_receiver.recv_timeout(Duration::from_micros(100)) {
                    Ok(message) => {
                        match message {
                            SimulatorMessage::OrderEvent(event) => {
                                info!("COIN_WORKER: Received OrderEvent message: {:#?}", event);
                                /*
                                println!(
                                    "--> {}, {}, {:?}, {:?}, {:?}, {:?}, {:?}, {}, {} <--",
                                    event.local_order_id,
                                    event.exchange_order_id,
                                    event.side,
                                    event.status,
                                    event.execution_type,
                                    event.order_type,
                                    event.side,
                                    event.price,
                                    event.quantity
                                );
                                 */

                                // trader.preprocess_order_event(&event).expect("failed to preprocess order event");
                                // eprintln!("PLAYGROUND_CONTEXT: event = {:#?}", event);
                                // trader.handle_order_event(event).expect("failed to apply order event");
                                if let Err(err) = trader.handle_order_event(event.clone()) {
                                    /*
                                    eprintln!("PLAYGROUND_CONTEXT: event = {:#?}", event);
                                    eprintln!("event.local_order_id = {:#?}", event.local_order_id);
                                    eprintln!("SmartId(event.local_order_id).decode() = {:#?}", SmartId(event.local_order_id).decode());

                                    if let TraderError::TradeError(TradeError::OrderNotFound(symbol, position_id, order_id)) = err {
                                        eprintln!("!!!!!!! symbol = {:#?}", symbol);
                                        eprintln!("!!!!!!! position_id = {:#?}", position_id);
                                        eprintln!("!!!!!!! order_id = {:#?}", order_id);
                                        eprintln!("!!!!!!! SmartId(order_id).decode() = {:#?}", SmartId(order_id).decode());
                                    }
                                     */

                                    error!("COIN_WORKER: ERROR: {:#?}", err);
                                    std::process::exit(1);
                                }
                            }

                            SimulatorMessage::BalanceUpdateEvent(_) => {
                                info!("COIN_WORKER: Received BalanceUpdateEvent message");
                                todo!("BalanceUpdateEvent message type is not implemented")
                            }

                            SimulatorMessage::Error { code, msg, local_description } => {
                                error!("COIN_WORKER: ERROR: code = {}, msg = {}, local_description = {}", code, msg, local_description);
                            }

                            SimulatorMessage::CandleEvent(candle) => match coin_manager.coins().get(&candle.symbol) {
                                None => {
                                    error!("COIN_WORKER: ERROR: Coin not found: {:#?}", candle.symbol);
                                    continue;
                                }

                                Some(coin) => {
                                    coin.write().push_candle(candle);
                                }
                            },
                        }
                    }

                    Err(err) => {
                        // error!("COIN_WORKER: ERROR: {:#?}", err);
                        // return;
                    }
                }

                // Handling messages from the application
                match coin_worker_receiver.recv_timeout(Duration::from_micros(100)) {
                    Ok(message) => match message {
                        CoinWorkerMessage::ExecutedInstruction(result) => {
                            info!("COIN_WORKER: Received ExecutedInstruction message: {:#?}", result);
                            /*
                            info!("COIN_WORKER: Received ExecutedInstruction message:");
                            println!(
                                "[--> {} {} {} {} <--]",
                                result.instruction_id,
                                result.metadata.position_id.unwrap_or_default(),
                                result.metadata.local_order_id.unwrap_or_default(),
                                result.outcome.as_ref().unwrap(),
                            );
                             */

                            trader
                                .confirm_instruction_execution(result)
                                .expect("failed to handle instruction execution result");
                        }

                        _ => {
                            panic!("Unexpected message: {:#?}", message);
                        }
                    },

                    Err(err) => {
                        // error!("COIN_WORKER: ERROR: {:#?}", err);
                        // return;
                    }
                }
            }
        });
    }

    fn start_simulator_datapoint_worker(config: Config, mut db: Database, receiver: Receiver<DataPoint>) {
        thread::spawn({
            move || loop {
                match receiver.recv() {
                    Ok(dp) => {
                        // handling datapoint messages
                        match dp {
                            DataPoint::Candle(candle) =>
                                if let Err(e) = db.upsert_candle(candle.symbol, candle) {
                                    panic!("failed to upsert candle: {:?}", e);
                                },

                            DataPoint::OrderClosed(order) => {
                                info!("Order finished: {:#?}", order);
                            }

                            DataPoint::TradeClosed(position) => {
                                info!("Trade finished: {:#?}", position);
                            }

                            _ => {
                                error!("unexpected datapoint type: {:#?}", dp);
                            }
                        }
                    }

                    Err(e) => {
                        panic!("failed to receive datapoint: {:#?}", e);
                    }
                }
            }
        });
    }
}

impl Context for PlaygroundContextInstance {
    fn get_database(&self) -> Database {
        self.db.clone()
    }

    fn coin_manager(&self) -> Arc<RwLock<CoinManager>> {
        self.coin_manager.clone()
    }

    fn run_user_stream(&mut self) -> std::result::Result<(), ContextError> {
        todo!()
    }

    fn run(&mut self, favorite_only: bool, graphical_mode: bool) -> Result<(), ContextError> {
        info!("Loading configs...");

        if favorite_only {
            info!("Running in favorite only mode");
        }

        let mut configs = self
            .config
            .coins
            .iter()
            .filter_map(|coin_config| {
                if favorite_only {
                    if !(coin_config.is_favorite.unwrap_or_default() && coin_config.is_enabled) {
                        return None;
                    }
                } else {
                    if !coin_config.is_enabled {
                        return None;
                    }
                }

                match self.exchange_information.symbols.iter().find(|s| s.symbol == coin_config.name.as_str()) {
                    None => None,
                    Some(symbol) => Some((coin_config.clone(), symbol.clone())),
                }
            })
            .collect_vec();

        for (coin_config, symbol_metadata) in configs.iter_mut() {
            self.ratelimiter.wait_for_websocket_message_limit(1);

            let symbol = ustr(symbol_metadata.symbol.as_str());
            let coin_specific_timeframe_configs = coin_specific_timeframe_configs(&self.config, symbol);

            if let Entry::Vacant(e) = self.coins.entry(symbol) {
                e.insert(CoinWorker::new(
                    graphical_mode,
                    self.tokio_runtime.handle().clone(),
                    self.config.clone(),
                    self.config.defaults.clone(),
                    coin_config.clone(),
                    coin_specific_timeframe_configs,
                    symbol,
                    symbol_metadata.to_owned(),
                    self.account.clone(),
                    self.db.clone(),
                    self.exchange_api.clone(),
                    self.balance.clone(),
                    self.ratelimiter.clone(),
                    self.coin_manager.clone(),
                    self.trader.clone(),
                    self.exchange_client_sender.clone(),
                    self.datapoint_sender.clone(),
                )?);
            }
        }

        Ok(())
    }

    fn process_order_update(&mut self, e: OrderUpdateEvent) {
        let symbol = ustr(e.symbol.as_str());

        // This is actually the order id that is generated by the exchange and is used to identify
        // the order in the exchange's system. This is not the same as the client order id that
        // is generated by the client and is used to identify the order in the client's system.
        let exchange_order_id = e.order_id;

        // NOTE:
        // Quite a legacy misnomer from the original implementation of `binance-rs` crate.
        // It's actually just a "client order id", a bit confusing, but that's how it is.
        // TODO: parse the client order id as a u64
        // let local_order_id = parseint_trick_u64(event.new_client_order_id.as_str());
        let local_order_id = match e.client_order_id.parse::<u64>() {
            Ok(local_order_id) => local_order_id,
            Err(err) => {
                error!("Failed to parse client order id: {:?}", err);
                std::process::exit(1);
            }
        };

        // Constructing metadata for the latest order fill (if possible)
        let latest_fill = match e.current_order_status {
            NormalizedOrderStatus::PartiallyFilled => Some(OrderFill::new(
                OrderFillStatus::PartiallyFilled,
                e.last_executed_price,
                e.last_executed_quantity,
                e.cumulative_filled_quantity,
                e.cumulative_quote_asset_transacted_quantity,
                e.transaction_time,
            )),

            NormalizedOrderStatus::Filled => Some(OrderFill::new(
                OrderFillStatus::Filled,
                e.last_executed_price,
                e.last_executed_quantity,
                e.cumulative_filled_quantity,
                e.cumulative_quote_asset_transacted_quantity,
                e.transaction_time,
            )),

            _ => None,
        };

        let normalized_trade_event = NormalizedOrderEvent {
            symbol,
            exchange_order_id: exchange_order_id as u64,
            local_order_id,
            trade_id: e.trade_id,
            time_in_force: e.time_in_force,
            side: e.side,
            order_type: e.order_type,
            current_execution_type: e.current_execution_type,
            current_order_status: e.current_order_status,
            order_price: e.order_price,
            last_filled_price: e.last_executed_price,
            quantity: e.order_quantity,
            last_executed_quantity: e.last_executed_quantity,
            last_quote_asset_transacted_quantity: e.last_quote_asset_transacted_quantity,
            cumulative_filled_quantity: e.cumulative_filled_quantity,
            cumulative_quote_asset_transacted_quantity: e.cumulative_quote_asset_transacted_quantity,
            quote_order_quantity: e.quote_order_quantity,
            commission: e.commission_amount,
            commission_asset: e.commission_asset,
            latest_fill,
            is_trade_the_maker_side: e.is_trade_the_maker_side,
            order_reject_reason: e.order_reject_reason.to_string(),
            source: NormalizedOrderEventSource::UserTradeEvent,
            transaction_time: e.transaction_time,
            event_time: e.event_time,
            avg_filled_price: e.cumulative_quote_asset_transacted_quantity / e.cumulative_filled_quantity,
        };

        panic!();
        /*
        // FIXME: I don't remember when and why I did this but this is wrong in the latest version of the code
        // FIXME: The received, normalized trade event must be handled by the Trader mechanism
        if let Err(err) = self
            .exchange_client_sender
            .send(ExchangeClientMessage::NormalizedUserTradeEvent(Box::new(normalized_trade_event)))
        {
            error!("failed to send trade event to the processor");
        }
         */
    }

    fn process_account_update(&mut self, update: AccountUpdateEvent) {
        update.balances.into_iter().for_each(|b| {
            self.balance.set_free(b.asset, b.free);
            self.balance.set_locked(b.asset, b.locked);
        });
    }

    fn process_balance_update(&mut self, e: BalanceUpdateEvent) {
        unimplemented!("process_balance_update_event");
    }

    fn get_balance(&self, symbol: Ustr) -> Result<Balance, ContextError> {
        Ok(self.balance.get(symbol)?)
    }

    fn get_trader(&self) -> Trader {
        self.trader.clone()
    }
}
