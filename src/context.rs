use binance_spot_connector_rust::{tokio_tungstenite::BinanceWebSocketClient, user_data_stream::UserDataStream};
use futures_util::StreamExt;
use std::{borrow::Borrow, collections::hash_map::Entry, num::ParseIntError, sync::Arc, thread};

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use crossbeam_channel::{unbounded, Receiver, Sender};
use itertools::Itertools;
use log::{debug, error, info, warn};
use parking_lot::{Mutex, RwLock};
use yata::prelude::*;

use crate::{
    balance::BalanceSheet,
    coin_manager::CoinManager,
    coin_worker::CoinWorker,
    types::{DataPoint, NormalizedExecutionType, NormalizedOrderEvent, NormalizedOrderType, NormalizedTimeInForce},
    util::s2of,
    Config,
};

use crate::{
    balance::BalanceError,
    coin_worker::CoinWorkerMessage,
    database::{Database, DatabaseBackend},
    exchange_client::{BinanceExchangeClient, ExchangeClientMessage},
    exchange_client_worker::exchange_client_worker,
    id::{HandleOrderKind, SmartId},
    model::{
        AccountInformation, AccountUpdateEvent, Balance, BalanceUpdateEvent, ExchangeInformation, OrderUpdateEvent, WebsocketUserMessage,
        WebsocketUserStreamResponse,
    },
    order::{OrderFill, OrderFillMetadata, OrderFillStatus},
    ratelimiter::UnifiedRateLimiter,
    trader::Trader,
    types::{ExchangeApi, ExchangeApiError, NormalizedOrderEventSource, NormalizedOrderStatus, Origin},
    util::{coin_specific_timeframe_configs, hash_id_u32, hash_id_u64, itoa_u64, parseint_trick_u64, s2i64},
};
use thiserror::Error;
use tokio::runtime;
use ustr::{ustr, Ustr, UstrMap};

#[derive(Error, Debug)]
pub enum ContextError {
    #[error("Coin not found: {0}")]
    CoinNotFound(u64),

    #[error("Balance error: {0:#?}")]
    BalanceError(#[from] BalanceError),

    #[error("Exchange API error: {0:#?}")]
    ExchangeApiError(#[from] ExchangeApiError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub trait Context {
    fn get_database(&self) -> Database;
    fn coin_manager(&self) -> Arc<RwLock<CoinManager>>;
    fn run_user_stream(&mut self) -> Result<(), ContextError>;
    fn run(&mut self, favorite_only: bool, graphical_mode: bool) -> Result<(), ContextError>;
    fn process_order_update(&mut self, e: OrderUpdateEvent);
    fn process_account_update(&mut self, e: AccountUpdateEvent);
    fn process_balance_update(&mut self, e: BalanceUpdateEvent);
    fn get_balance(&self, symbol: Ustr) -> Result<Balance, ContextError>;
    fn get_trader(&self) -> Trader;
}

pub struct ContextInstance {
    tokio_runtime:          runtime::Runtime,
    config:                 Config,
    exchange_api:           ExchangeApi,
    exchange_info:          Arc<ExchangeInformation>,
    account_info:           Arc<AccountInformation>,
    balance:                BalanceSheet,
    ratelimiter:            UnifiedRateLimiter,
    coins:                  UstrMap<CoinWorker>,
    coin_manager:           Arc<RwLock<CoinManager>>,
    trader:                 Trader,
    db:                     Database,
    exchange_client_sender: Sender<ExchangeClientMessage>,
    datapoint_sender:       Sender<DataPoint>,
    coin_worker_sender:     Sender<CoinWorkerMessage>,
}

impl ContextInstance {
    pub fn new(runtime: runtime::Runtime, config: Config, exchange_api: ExchangeApi) -> Result<Self, ContextError> {
        // Initializing the database
        let db = Database::new(config.database.clone()).expect("Failed to initialize database object");

        let ratelimiter = UnifiedRateLimiter::default();
        let coin_manager = CoinManager::new();
        let mut balance = BalanceSheet::default();

        let account_info = Arc::new(exchange_api.fetch_account()?);
        let exchange_info = Arc::new(exchange_api.fetch_exchange_information()?);

        // Initializing aggregated account balance
        account_info.balances.iter().for_each(|b| {
            balance.set_free(b.asset, b.free);
            balance.set_locked(b.asset, b.locked);
        });

        // Initializing channels outside their respective workers
        let (exchange_client_sender, exchange_client_receiver) = unbounded::<ExchangeClientMessage>();
        let (datapoint_sender, datapoint_receiver) = unbounded::<DataPoint>();
        let (coin_worker_sender, coin_worker_receiver) = unbounded::<CoinWorkerMessage>();

        Self::start_datapoint_worker(config.clone(), db.clone(), datapoint_receiver);

        // Initializing a global trader mechanism
        let trader = Trader::new(balance.clone(), exchange_client_sender.clone(), datapoint_sender.clone());

        // This is the client that will be used to communicate with the exchange
        let exchange_client = BinanceExchangeClient::new(exchange_api.clone(), ratelimiter.clone(), trader.clone());

        // WARNING: running the exchange client in a separate thread
        thread::spawn({
            let coin_worker_sender = coin_worker_sender.clone();
            move || exchange_client_worker(exchange_client, exchange_client_receiver, coin_worker_sender)
        });

        Ok(Self {
            tokio_runtime: runtime,
            exchange_info,
            account_info,
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

    fn start_datapoint_worker(config: Config, mut db: Database, receiver: Receiver<DataPoint>) {
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

                            _ => {}
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

impl Context for ContextInstance {
    fn get_database(&self) -> Database {
        self.db.clone()
    }

    fn coin_manager(&self) -> Arc<RwLock<CoinManager>> {
        self.coin_manager.clone()
    }

    fn run_user_stream(&mut self) -> Result<(), ContextError> {
        Ok(())
    }

    fn run(&mut self, favorite_only: bool, graphical_mode: bool) -> Result<(), ContextError> {
        // Running the userstream handler in a separate thread
        self.run_user_stream()?;

        info!("Loading coin configs...");

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

                match self.exchange_info.symbols.iter().find(|s| s.symbol == coin_config.name.as_str()) {
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
                    self.account_info.clone(),
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

        // Localizing the order id
        let local_order_id = match e.client_order_id.parse::<u64>() {
            // If the client order id is a valid u64, we will use it as the local order id
            Ok(local_order_id) => local_order_id,

            // If the client order id cannot be parsed as an u64, we will assume that this update is caused
            // by an external action, but we will still try to parse the original client order id before ignoring
            Err(err) => match e.original_client_order_id.parse::<u64>() {
                Ok(original_local_order_id) => {
                    debug!(
                        "Failed to parse client order id {}, assuming external order, using original client order id {}",
                        e.client_order_id, original_local_order_id
                    );
                    original_local_order_id
                }
                Err(err) => {
                    // WARNING:
                    // If the original client order id cannot be parsed as an u64, we will hash the client order id
                    // and use it as the local order id. This is for orders that are not placed by the client but are
                    // still updated by the exchange. This is a workaround to avoid using the same local order id for
                    // different orders.
                    debug!("Localizing non-numeric client order id: {}", e.client_order_id);
                    SmartId::new_order_id_with_unique_id(0, HandleOrderKind::UnexpectedOrder, hash_id_u32(e.client_order_id.as_str()))
                }
            },
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

        if let Err(err) = self.trader.handle_order_event(normalized_trade_event) {
            error!("failed to handle order event: {:#?}", err);
        }
    }

    fn process_account_update(&mut self, e: AccountUpdateEvent) {
        e.balances.into_iter().for_each(|b| {
            self.balance.set_free(b.asset, b.free);
            self.balance.set_locked(b.asset, b.locked);
        });
    }

    fn process_balance_update(&mut self, e: BalanceUpdateEvent) {
        unimplemented!("process_balance_update_event");
    }

    fn get_balance(&self, asset: Ustr) -> Result<Balance, ContextError> {
        Ok(self.balance.get(asset)?)
    }

    fn get_trader(&self) -> Trader {
        self.trader.clone()
    }
}
