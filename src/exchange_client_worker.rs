use crate::{
    coin_worker::CoinWorkerMessage,
    exchange_client::{BinanceExchangeClient, ExchangeClientError, ExchangeClientMessage},
    instruction::{ExecutionOutcome, InstructionExecutionResult},
    order::OrderError,
    simulator::{SimulatorError, SimulatorRequestMessage},
    types::{NormalizedOrderStatus, NormalizedOrderType, NormalizedSide},
    util::s2u64,
};
use chrono::Utc;
use crossbeam::{channel, channel::Receiver};
use crossbeam_channel::{select, unbounded, Sender};
use log::{error, info, warn};
use num_traits::Zero;
use parking_lot::Mutex;
use std::{sync::Arc, thread, time::Duration};

pub(crate) fn exchange_client_worker(
    mut exchange_client: BinanceExchangeClient,
    receiver: Receiver<ExchangeClientMessage>,
    coin_worker_sender: Sender<CoinWorkerMessage>,
) {
    loop {
        select! {
            // TODO: Decide whether I need it here, because the confirmation is done in the exchange client
            recv(receiver) -> event => {
                match event {
                    Ok(ExchangeClientMessage::NormalizedUserTradeEvent(event)) => {
                        if !event.commission.is_zero() {
                            warn!("non-zero commission: {:#?}", event);
                        }

                        let result = match event.current_order_status {
                            NormalizedOrderStatus::New => exchange_client.confirm_opened(event.symbol, event.local_order_id, if event.exchange_order_id.is_zero() { None } else { Some(event.exchange_order_id) }),
                            NormalizedOrderStatus::PartiallyFilled | NormalizedOrderStatus::Filled =>
                                exchange_client.confirm_filled(event.symbol, event.local_order_id, event.latest_fill),
                            NormalizedOrderStatus::Cancelled => exchange_client.confirm_cancelled(event.symbol, event.local_order_id),
                            NormalizedOrderStatus::Expired => {
                                // FIXME: investigate whether this is causing an insufficient balance error response
                                exchange_client.confirm_cancelled(event.symbol, event.local_order_id)
                            }
                            NormalizedOrderStatus::Rejected => exchange_client.confirm_open_failed(event.symbol, event.local_order_id),
                            NormalizedOrderStatus::Replaced => {
                                unimplemented!("replaced order");
                                // exchange_client.confirm_replaced(event.symbol,  event.,event.local_order_id),
                            }
                            // NOTE: turns out Binance also has `PendingCancel` but it's unused atm
                            NormalizedOrderStatus::PendingCancel => Ok(()),
                        };

                        if let Err(err) = result {
                            match err {
                                ExchangeClientError::OrderError(OrderError::OrderNotFound(order_id)) => {
                                    warn!("{}: ignoring {:?} for untracked order {}", event.symbol, event.current_order_status, order_id);
                                    continue;
                                }

                                _ => {
                                    panic!("{}: unhandled error: {:#?}", event.symbol, err);
                                }
                            }
                        }
                    }

                    Ok(ExchangeClientMessage::Instruction(instruction_id, metadata, instruction, priority)) => {
                        if let Err(err) = exchange_client.push_instruction(instruction_id, metadata, instruction, priority) {
                            error!("failed to push instruction: {:#?}", err);
                        }
                    }

                Err(err) => {
                    error!("exchange client message event: {:#?}", err);
                    }};
            }

            recv(channel::after(Duration::from_micros(100))) -> _ => {
                match exchange_client.compute() {
                    Ok(_) => {}
                    Err(err) => {
                        dbg!(err);
                        panic!("compute error");
                    }
                }
            }
        }

        /*
        match receiver.recv_timeout(Duration::from_micros(10)).expect("exchange client message event") {
            ExchangeClientMessage::NormalizedUserTradeEvent(event) => {
                if !event.commission.is_zero() {
                    warn!("non-zero commission: {:#?}", event);
                }

                let result = match event.current_order_status {
                    NormalizedOrderStatus::New => exchange_client.lock().confirm_opened(event.symbol, event.local_order_id),
                    NormalizedOrderStatus::PartiallyFilled | NormalizedOrderStatus::Filled =>
                        exchange_client.lock().confirm_filled(event.symbol, event.local_order_id, event.latest_fill),
                    NormalizedOrderStatus::Cancelled => exchange_client.lock().confirm_cancelled(event.symbol, event.local_order_id),
                    NormalizedOrderStatus::Expired => {
                        // FIXME: investigate whether this is causing an insufficient balance error response
                        exchange_client.lock().confirm_cancelled(event.symbol, event.local_order_id)
                    }
                    NormalizedOrderStatus::Rejected => exchange_client.lock().confirm_open_failed(event.symbol, event.local_order_id),
                    NormalizedOrderStatus::Replaced => exchange_client.lock().confirm_replaced(event.symbol, event.local_order_id),
                    // NOTE: turns out Binance also has `PendingCancel` but it's unused atm
                    NormalizedOrderStatus::PendingCancel => Ok(()),
                };

                if let Err(err) = result {
                    match err {
                        ExchangeClientError::OrderError(OrderError::OrderNotFound(order_id)) => {
                            warn!("{}: ignoring {:?} for untracked order {}", event.symbol, event.current_order_status, order_id);
                            continue;
                        }

                        _ => {
                            panic!("{}: unhandled error: {:#?}", event.symbol, err);
                        }
                    }
                }
            }

            ExchangeClientMessage::Instruction(instruction_id, metadata, instruction, priority) => {
                if let Err(err) = exchange_client.lock().push_instruction(instruction_id, metadata, instruction, priority) {
                    error!("failed to push instruction: {:#?}", err);
                }
            }
        };

        // TODO: introduce a call on an timeframe as well
        match exchange_client.lock().compute() {
            Ok(_) => {}
            Err(err) => {
                dbg!(err);
                panic!("compute error");
            }
        }
         */
    }
}
