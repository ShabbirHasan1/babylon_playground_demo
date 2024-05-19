use crate::{
    appraiser::Appraiser,
    balance::BalanceSheet,
    coin_worker::CoinWorkerMessage,
    cooldown::Cooldown,
    exchange_client::ExchangeClientMessage,
    instruction::{ExecutionOutcome, Instruction, InstructionExecutionResult},
    model::{
        IcebergPartsRule, LotSizeRule, MarketLotSizeRule, MaxNumAlgoOrdersRule, MaxNumOrdersRule, NotionalRule, PriceFilterRule, Rules, Symbol,
        TrailingDeltaRule,
    },
    order::Order,
    position::Position,
    trader::{Trader, TraderSymbolContext},
    types::{NormalizedCommission, NormalizedOrderType},
};
use chrono::Utc;
use indexmap::IndexMap;
use ordered_float::OrderedFloat;
use portable_atomic::AtomicF64;
use std::{sync::Arc, time::Duration};
use ustr::{ustr, Ustr};
#[macro_export]
macro_rules! f {
    ($value: expr) => {
        OrderedFloat($value)
    };
}

#[macro_export]
macro_rules! d {
    ($e:expr) => {
        rust_decimal::Decimal::from_f64($e).unwrap()
    };
}

#[macro_export]
macro_rules! atomic_f {
    ($value: expr) => {
        Arc::new(AtomicF64::new($value))
    };
}

pub fn symbol_for_testing() -> Symbol {
    Symbol {
        symbol:                              ustr("TESTUSDT"),
        status:                              ustr("TRADING"),
        base_asset:                          ustr("TEST"),
        base_asset_precision:                8,
        quote_asset:                         ustr("USDT"),
        quote_precision:                     8,
        quote_asset_precision:               8,
        base_commission_precision:           8,
        quote_commission_precision:          8,
        order_types:                         vec![
            NormalizedOrderType::Limit,
            NormalizedOrderType::LimitMaker,
            NormalizedOrderType::Market,
            NormalizedOrderType::StopLossLimit,
            NormalizedOrderType::TakeProfitLimit,
        ],
        iceberg_allowed:                     true,
        oco_allowed:                         true,
        quote_order_qty_market_allowed:      true,
        allow_trailing_stop:                 true,
        cancel_replace_allowed:              true,
        rules:                               Rules {
            price_filter:           Some(PriceFilterRule {
                min_price: OrderedFloat(0.01),
                max_price: OrderedFloat(1000000.0),
                tick_size: OrderedFloat(0.01),
            }),
            percent_price:          None,
            lot_size:               Some(LotSizeRule {
                min_qty:   OrderedFloat(1e-5),
                max_qty:   OrderedFloat(9000.0),
                step_size: OrderedFloat(1e-5),
            }),
            market_lot_size:        Some(MarketLotSizeRule {
                min_qty:   OrderedFloat(0.0),
                max_qty:   OrderedFloat(39.53598857),
                step_size: OrderedFloat(0.0),
            }),
            min_notional:           None,
            notional:               Some(NotionalRule {
                min_notional:        OrderedFloat(5.0),
                apply_min_to_market: true,
                max_notional:        OrderedFloat(9000000.0),
                apply_max_to_market: false,
                avg_price_mins:      5,
            }),
            iceberg_parts:          Some(IcebergPartsRule { limit: 10 }),
            max_num_iceberg_orders: None,
            max_num_orders:         Some(MaxNumOrdersRule { max_num_orders: 200 }),
            max_num_algo_orders:    Some(MaxNumAlgoOrdersRule { max_num_algo_orders: 5 }),
            max_position:           None,
            trailing_delta:         Some(TrailingDeltaRule {
                min_trailing_above_delta: 10,
                max_trailing_above_delta: 2000,
                min_trailing_below_delta: 10,
                max_trailing_below_delta: 2000,
            }),
        },
        is_spot_trading_allowed:             true,
        is_margin_trading_allowed:           true,
        permissions:                         vec![
            ustr("SPOT"),
            ustr("MARGIN"),
            ustr("TRD_GRP_004"),
            ustr("TRD_GRP_005"),
            ustr("TRD_GRP_009"),
            ustr("TRD_GRP_010"),
            ustr("TRD_GRP_011"),
            ustr("TRD_GRP_012"),
            ustr("TRD_GRP_013"),
            ustr("TRD_GRP_014"),
            ustr("TRD_GRP_015"),
            ustr("TRD_GRP_016"),
            ustr("TRD_GRP_017"),
            ustr("TRD_GRP_018"),
            ustr("TRD_GRP_019"),
            ustr("TRD_GRP_020"),
            ustr("TRD_GRP_021"),
            ustr("TRD_GRP_022"),
            ustr("TRD_GRP_023"),
            ustr("TRD_GRP_024"),
            ustr("TRD_GRP_025"),
        ],
        default_self_trade_prevention_mode:  ustr("EXPIRE_MAKER"),
        allowed_self_trade_prevention_modes: vec![ustr("EXPIRE_TAKER"), ustr("EXPIRE_MAKER"), ustr("EXPIRE_BOTH")],
    }
}

pub fn balance_for_testing() -> BalanceSheet {
    let mut balance = BalanceSheet::default();
    balance.set_free(ustr("TEST"), f!(1000.0));
    balance.set_free(ustr("USDT"), f!(1000.0));
    balance
}

pub fn trader_for_testing(symbol: Ustr, appraiser: Appraiser, commission: NormalizedCommission, debug_messages: bool) -> (Ustr, Trader, Arc<AtomicF64>) {
    let (datapoint_sender, datapoint_receiver) = crossbeam::channel::unbounded();
    let (exchange_client_sender, exchange_client_receiver) = crossbeam::channel::unbounded();
    let (coin_worker_sender, coin_worker_receiver) = crossbeam::channel::unbounded();

    let price = Arc::new(AtomicF64::new(100.0));

    // NOTE: This is a test trader, so we don't care about the result of the messages.

    std::thread::spawn({
        // let exchange_client_receiver = exchange_client_receiver.clone();

        // ----------------------------------------------------------------------------
        // Mocking the exchange client message processor
        // ----------------------------------------------------------------------------
        move || loop {
            match exchange_client_receiver.recv() {
                Ok(message) => {
                    match message {
                        // println!("EXCHANGE_CLIENT_RECEIVER: Received message: {:#?}", message);
                        ExchangeClientMessage::Instruction(instruction_id, metadata, instruction, priority) => match instruction {
                            Instruction::LimitBuy { symbol, .. } => {
                                if debug_messages {
                                    println!("EXCHANGE_CLIENT_RECEIVER: Confirming LimitBuy instruction");
                                }

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

                            _ => {
                                println!("EXCHANGE_CLIENT_RECEIVER: TEST TRADER ERROR: Unexpected instruction type");
                                return;
                            }
                        },

                        _ => {
                            println!("EXCHANGE_CLIENT_RECEIVER: TEST TRADER ERROR: Unexpected message type");
                            return;
                        }
                    }
                }
                Err(err) => {
                    // println!("TEST EXCHANGE_CLIENT_RECEIVER ERROR: {:#?}", err);
                    return;
                }
            }
        }
    });

    // ----------------------------------------------------------------------------
    // Mocking the datapoint processor
    // ----------------------------------------------------------------------------
    std::thread::spawn({
        move || loop {
            match datapoint_receiver.recv() {
                Ok(datapoint) => match datapoint {
                    _ =>
                        if debug_messages {
                            println!("DATAPOINT_RECEIVER: {:#?}", datapoint);
                        },
                },
                Err(err) => {
                    // println!("TEST DATAPOINT_RECEIVER ERROR: {:#?}", err);
                    return;
                }
            }
        }
    });

    // ----------------------------------------------------------------------------
    // Mocking the coin worker
    // ----------------------------------------------------------------------------
    std::thread::spawn({
        move || loop {
            match coin_worker_receiver.recv() {
                Ok(message) => match message {
                    _ =>
                        if debug_messages {
                            println!("COIN_WORKER: {:#?}", message);
                        },
                },
                Err(err) => {
                    // println!("TEST COIN_WORKER ERROR: {:#?}", err);
                    return;
                }
            }
        }
    });

    let balance = balance_for_testing();
    let trader = Trader::new(balance.clone(), exchange_client_sender, datapoint_sender);

    trader.create_context(TraderSymbolContext {
        symbol,
        quote_asset: ustr("USDT"),
        base_asset: ustr("TEST"),
        atomic_market_price: price.clone(),
        entry_cooldown: Cooldown::new(Duration::from_millis(100), 1),
        exit_cooldown: Cooldown::new(Duration::from_millis(100), 1),
        appraiser,
        commission,
        max_quote_allocated: OrderedFloat(1000.0),
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
        position: Position::new(ustr("TESTUSDT"), balance),
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
        max_orders_per_trade: 0,
    });

    (symbol, trader, price)
}

/// Prints the explanation of the changes between two stops, comparing field values.
#[allow(dead_code)]
pub fn explain_order_changes(new: &Order, old_snapshot: Order) {
    todo!("fix explain_order_changes");
    /*
    // ID check
    if new.id != old_snapshot.id {
        println!("!--> ID changed from {} to {}", old_snapshot.id, new.id);
    }

    // Symbol check
    if new.symbol != old_snapshot.symbol {
        println!("!--> Symbol changed from {} to {}", old_snapshot.symbol, new.symbol);
    }

    // is_enabled check
    if new.is_enabled != old_snapshot.is_enabled {
        println!("!--> Is enabled changed from {} to {}", old_snapshot.is_enabled, new.is_enabled);
    }

    // is_virtual check
    if new.is_virtual != old_snapshot.is_virtual {
        println!("!--> Is virtual changed from {} to {}", old_snapshot.is_virtual, new.is_virtual);
    }

    // order_type check
    if new.order_type != old_snapshot.order_type {
        println!("!--> Order type changed from {:?} to {:?}", old_snapshot.order_type, new.order_type);
    }

    // contingency_order_type check
    if new.contingency_order_type != old_snapshot.contingency_order_type {
        println!("!--> Contingency order type changed from {:?} to {:?}", old_snapshot.contingency_order_type, new.contingency_order_type);
    }

    // Check if activators are different. This would require the activators to implement PartialEq or similar.
    // println!("!--> Activator changed from {} to {}", old.activator, new.activator);

    // is_created_on_exchange check
    if new.is_created_on_exchange != old_snapshot.is_created_on_exchange {
        println!("!--> Is created on exchange changed from {} to {}", old_snapshot.is_created_on_exchange, new.is_created_on_exchange);
    }

    // local_stop_order_id check
    if new.local_stop_order_id != old_snapshot.local_stop_order_id {
        println!("!--> Local stop order ID changed from {} to {}", old_snapshot.local_stop_order_id, new.local_stop_order_id);
    }

    // local_order_id check
    if new.local_stop_limit_order_id != old_snapshot.local_stop_limit_order_id {
        println!("!--> Local order ID changed from {} to {}", old_snapshot.local_stop_limit_order_id, new.local_stop_limit_order_id);
    }

    // side check
    if new.side != old_snapshot.side {
        println!("!--> Side changed from {:?} to {:?}", old_snapshot.side, new.side);
    }

    // quantity check
    if new.quantity != old_snapshot.quantity {
        println!("!--> Quantity changed from {} to {}", old_snapshot.quantity, new.quantity);
    }

    // reference_price check
    if new.reference_price != old_snapshot.reference_price {
        println!("!--> Reference price changed from {} to {}", old_snapshot.reference_price, new.reference_price);
    }

    // initial_stop_price check
    if new.initial_stop_price != old_snapshot.initial_stop_price {
        println!("!--> Initial stop price changed from {} to {}", old_snapshot.initial_stop_price, new.initial_stop_price);
    }

    // stop_price check
    if new.stop_price != old_snapshot.stop_price {
        println!("!--> Stop price changed from {} to {}", old_snapshot.stop_price, new.stop_price);
    }

    // limit_price check
    if new.limit_price != old_snapshot.limit_price {
        println!("!--> Limit price changed from {:?} to {:?}", old_snapshot.limit_price, new.limit_price);
    }

    // limit_price_offset_ratio check
    if new.limit_price_offset_ratio != old_snapshot.limit_price_offset_ratio {
        println!("!--> Limit price offset ratio changed from {} to {}", old_snapshot.limit_price_offset_ratio, new.limit_price_offset_ratio);
    }

    // is_trailing_active check
    if new.is_trailing != old_snapshot.is_trailing {
        println!("!--> Is trailing active changed from {} to {}", old_snapshot.is_trailing, new.is_trailing);
    }

    // initial_step_ratio check
    if new.initial_step_ratio != old_snapshot.initial_step_ratio {
        println!("!--> Initial step ratio changed from {} to {}", old_snapshot.initial_step_ratio, new.initial_step_ratio);
    }

    // trailing_step_ratio check
    if new.trailing_step_ratio != old_snapshot.trailing_step_ratio {
        println!("!--> Trailing step ratio changed from {} to {}", old_snapshot.trailing_step_ratio, new.trailing_step_ratio);
    }

    // pending_instruction check
    // Again, would need PartialEq or a custom comparison function
    if new.pending_instruction != old_snapshot.pending_instruction {
        println!("!--> Pending instruction changed from {:#?} to {:#?}", old_snapshot.pending_instruction, new.pending_instruction);
    }
     */
}

#[macro_export]
macro_rules! add_ratio {
    ($value: expr, $ratio: expr, $precision: expr) => {
        round_float_to_ordered_precision($value.0 * (1.0 + $ratio), $precision)
    };
}

#[macro_export]
macro_rules! with_order_explanation {
    ($stop: expr, $callback: block) => {
        let old_snapshot = $stop.clone();
        $callback();
        explain_stop_changes(&$stop, old_snapshot);
    };
}

#[macro_export]
macro_rules! quote {
    ($value: expr) => {
        Quantity::Quote(Amount::Exact(OrderedFloat($value)))
    };
}

#[macro_export]
macro_rules! quote_ratio_of_available {
    ($value: expr) => {
        Quantity::Quote(Amount::RatioOfAvailable(OrderedFloat($value)))
    };
}

#[macro_export]
macro_rules! quote_ratio_of_total {
    ($value: expr) => {
        Quantity::Quote(Amount::RatioOfTotal(OrderedFloat($value)))
    };
}

#[macro_export]
macro_rules! quote_all_available {
    () => {
        Quantity::Quote(Amount::AllAvailable)
    };
}

#[macro_export]
macro_rules! base {
    ($value: expr) => {
        Quantity::Base(Amount::Exact(OrderedFloat($value)))
    };
}

#[macro_export]
macro_rules! base_ratio_of_available {
    ($value: expr) => {
        Quantity::Base(Amount::RatioOfAvailable(OrderedFloat($value)))
    };
}

#[macro_export]
macro_rules! base_ratio_of_total {
    ($value: expr) => {
        Quantity::Base(Amount::RatioOfTotal(OrderedFloat($value)))
    };
}

#[macro_export]
macro_rules! base_all_available {
    () => {
        Quantity::Base(Amount::AllAvailable)
    };
}

#[macro_export]
macro_rules! create_test_setup {
    ($symbol:expr, $debug_messages:expr) => {{
        let symbol = ustr($symbol);
        let appraiser = Appraiser::new(symbol_for_testing().rules);
        let commission = NormalizedCommission {
            maker:  Some(OrderedFloat(0.001)),
            taker:  Some(OrderedFloat(0.001)),
            buyer:  None,
            seller: None,
        };

        let (symbol, mut trader, price) = trader_for_testing(symbol, appraiser, commission, $debug_messages);

        (symbol, trader, price)
    }};
}

#[macro_export]
macro_rules! buy_order_template {
    ($trader:expr, $symbol:expr, $trade_id:expr, $price:expr, $quantity:expr, $order_type:ident) => {{
        let order = $trader
            .buy_order_template(ustr($symbol), $trade_id, f!($price), f!($quantity), NormalizedOrderType::$order_type)
            .unwrap();
        (order.local_id, order)
    }};
}

#[macro_export]
macro_rules! sell_order_template {
    ($trader:expr, $symbol:expr, $trade_id:expr, $price:expr, $quantity:expr, $order_type:expr) => {{
        let order = $trader
            .sell_order_template(ustr($symbol), $trade_id, f!($price), f!($quantity), NormalizedOrderType::$order_type)
            .unwrap();
        (order.local_id, order)
    }};
}

#[macro_export]
macro_rules! trader_limit_buy {
    ($trader:expr, $symbol:expr, $timeframe:expr, $trade_id:expr, $price:expr, $quantity:expr, $stop_metadata:expr, $activator_metadata:expr, $deactivator_metadata:expr, $is_post_only:expr) => {
        $trader
            .limit_buy(
                ustr($symbol),
                $timeframe,
                $trade_id,
                f!($price),
                f!($quantity),
                $stop_metadata,
                $activator_metadata,
                $deactivator_metadata,
                $is_post_only,
            )
            .expect("failed to place test limit buy order")
    };
}

#[macro_export]
macro_rules! trader_get_managed_instruction {
    ($trader:expr, $symbol:expr, $local_order_id:expr) => {{
        $trader.get_order(ustr($symbol), $local_order_id).unwrap().managed_instruction.clone()
    }};
}

#[macro_export]
macro_rules! trader_print_managed_instruction {
    ($trader:expr, $symbol:expr, $local_order_id:expr) => {{
        let order = $trader.get_order(ustr($symbol), $local_order_id).unwrap();
        println!("DEBUG: {:#?}", order.managed_instruction);
    }};
}

#[macro_export]
macro_rules! trader_confirm_managed_instruction {
    ($trader:expr, $symbol:expr, $local_order_id:expr, $outcome:expr) => {{
        let result = {
            let order = $trader.get_order(ustr($symbol), $local_order_id).unwrap();
            let pending_instruction = order.pending_instruction.clone().unwrap();

            InstructionExecutionResult {
                instruction_id: pending_instruction.id,
                symbol:         Some(ustr($symbol)),
                metadata:       pending_instruction.metadata,
                outcome:        $outcome,
                executed_at:    Some(Utc::now()),
                execution_time: Some(Duration::from_millis(100)),
            }
        };

        $trader.confirm_instruction_execution(result).expect("failed to confirm pending instruction");
    }};
}

#[macro_export]
macro_rules! trader_compute_price {
    ($trader:expr, $symbol:expr, $new_price:expr) => {{
        $trader.set_latest_known_market_price(ustr($symbol), f!($new_price)).unwrap();
        $trader.compute().unwrap();
    }};
}

#[macro_export]
macro_rules! confirm_partial_order_fill {
    ($trader:expr, $symbol:expr, $local_order_id:expr, $price:expr, $quantity:expr) => {
        let (accumulated_filled_quantity, accumulated_filled_quote) = {
            let order = $trader.get_order(ustr($symbol), $local_order_id).unwrap();
            (order.accumulated_filled_quantity, order.accumulated_filled_quote)
        };

        $trader
            .confirm_order_filled(
                ustr($symbol),
                $local_order_id,
                OrderFill::new_partial(
                    f!($price),
                    f!($quantity),
                    accumulated_filled_quantity + $quantity,
                    accumulated_filled_quote + $price * $quantity,
                    Utc::now(),
                ),
            )
            .expect("failed to confirm order filled");
    };
}

#[macro_export]
macro_rules! confirm_full_order_fill {
    ($trader:expr, $symbol:expr, $local_order_id:expr, $price:expr, $quantity:expr) => {
        let (accumulated_filled_quantity, accumulated_filled_quote) = {
            let order = $trader.get_order(ustr($symbol), $local_order_id).unwrap();
            (order.accumulated_filled_quantity, order.accumulated_filled_quote)
        };

        $trader
            .confirm_order_filled(
                ustr($symbol),
                $local_order_id,
                OrderFill::new_filled(
                    f!($price),
                    f!($quantity),
                    accumulated_filled_quantity + $quantity,
                    accumulated_filled_quote + $price * $quantity,
                    Utc::now(),
                ),
            )
            .expect("failed to confirm order filled");
    };
}

#[macro_export]
macro_rules! roundtrip_init_test {
    ($mode: ident, $quote: expr, $entry_side: ident,
    $entry_price: expr, $exit_price: expr, $stop_price: expr) => {{
        let (tx, rx) = channel::bounded(1);

        let appraiser = Appraiser::new(symbol_for_testing());

        let order_id = "test_order_id".to_string();
        let symbol = "BTCUSDT".to_string();
        let mode = RoundtripMode::$mode;
        let entry_side = NormalizedSide::$entry_side;
        let allocated_quote = Currency::Quote($quote);
        let entry_price = OrderedFloat($entry_price);
        let exit_price = OrderedFloat($exit_price);
        let stop_price = OrderedFloat($stop_price);
        let stop_limit_price = None;
        let price_step = OrderedFloat(0.15);
        let entry_order_type = NormalizedOrderType::LimitMaker;
        let exit_order_type = NormalizedOrderType::LimitMaker;
        let stop_order_type = NormalizedOrderType::StopLossLimit;
        let entry_retries = 2;
        let exit_retries = 2;

        let mut r = Roundtrip::new(
            order_id.clone(),
            symbol.clone(),
            appraiser,
            mode,
            entry_side,
            allocated_quote,
            entry_price,
            exit_price,
            stop_price,
            stop_limit_price,
            price_step,
            None,
            entry_order_type,
            exit_order_type,
            stop_order_type,
            entry_retries,
            exit_retries,
            Some(tx),
        )
        .unwrap();

        assert_eq!(r.id, order_id);
        assert_eq!(r.symbol, symbol);
        assert_eq!(r.mode, mode);
        assert_eq!(r.entry_side, entry_side);
        assert_eq!(r.entry_price, OrderedFloat(appraiser.normalize_quote(entry_price.0)));
        assert_eq!(r.exit_price, OrderedFloat(appraiser.normalize_quote(exit_price.0)));
        assert_eq!(r.stop_price, OrderedFloat(appraiser.normalize_quote(stop_price.0)));
        assert_eq!(r.stop_limit_price, stop_limit_price);
        assert_eq!(r.entry_order_type, entry_order_type);
        assert_eq!(r.exit_order_type, exit_order_type);
        assert_eq!(r.stop_order_type, stop_order_type);
        assert_eq!(r.entry_retries_left, (entry_retries, entry_retries));
        assert_eq!(r.exit_retries_left, (exit_retries, exit_retries));

        // initial phase, first compute must return the instruction to enter trade
        assert_eq!(r.phase(), TradingPhase::Pending);

        (r, rx)
    }};
}

#[macro_export]
macro_rules! check_feedback {
    ($r: expr, $rx: expr, $status: ident) => {
        let CoinEvent::RoundtripFinalized { id, status } = $rx.recv().unwrap() else {
            panic!()
        };

        assert_eq!($r.id, id);
        assert_eq!(Status::$status, status);
    };
}

#[macro_export]
macro_rules! roundtrip_compute {
    ($r: expr, $phase: ident, $rstate: ident, $ostate: ident, $side: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::$phase);
        assert_eq!($r.roundtrip_state, RoundtripState::$rstate);
        assert_eq!($r.order_state, OrderState::$ostate);
        assert_eq!($r.current_side, $side);
    };

    ($r: expr, $instruction: expr, $phase: ident, $rstate: ident, $ostate: ident, $side: expr) => {
        assert_eq!($r.compute().unwrap().instructions.first(), Some(&$instruction));
        assert_eq!($r.phase(), TradingPhase::$phase);
        assert_eq!($r.roundtrip_state, RoundtripState::$rstate);
        assert_eq!($r.order_state, OrderState::$ostate);
        assert_eq!($r.current_side, $side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_nothing_changed {
    ($r: expr) => {{
        let phase_before_compute = $r.phase();
        let roundtrip_state_before_compute = $r.roundtrip_state;
        let order_state_before_compute = $r.order_state;
        let current_side_before_compute = $r.current_side;

        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), phase_before_compute);
        assert_eq!($r.roundtrip_state, roundtrip_state_before_compute);
        assert_eq!($r.order_state, order_state_before_compute);
        assert_eq!($r.current_side, current_side_before_compute);
    }};
}

#[macro_export]
macro_rules! roundtrip_compute_assert_entry_opened {
    ($r: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryOpened);
        assert_eq!($r.order_state, OrderState::Opened);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_exit_opened {
    ($r: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::Opened);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_entry_partially_filled {
    ($r: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryOpened);
        assert_eq!($r.order_state, OrderState::PartiallyFilled);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_exit_partially_filled {
    ($r: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::PartiallyFilled);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_entry_closed {
    ($r: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryClosed);
        assert_eq!($r.order_state, OrderState::Closed);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_exit_closed {
    ($r: expr) => {
        assert!($r.compute().unwrap().is_empty());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::Opened);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_normal_entry {
    ($r: expr) => {
        roundtrip_compute!($r, $r.instruction_normal_entry(), Entering, EntryPending, Pending, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_retry_entry {
    ($r: expr) => {
        let entry_retries_before_failing = $r.entry_retries_left.0;
        roundtrip_compute!($r, $r.instruction_normal_entry(), Entering, EntryPending, Pending, $r.entry_side);
        assert_eq!($r.entry_retries_left.0, entry_retries_before_failing - 1);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_internally_cancelled_entry {
    ($r: expr) => {
        roundtrip_compute!($r, Entering, Cancelled, Finished, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_cancelled_entry {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::CancelOrder {
                symbol:   $r.symbol.clone(),
                order_id: $r.id.clone(),
            },
            Entering,
            Cancelled,
            Pending,
            $r.entry_side.opposite()
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_entry_cancel_order {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::CancelOrder {
                symbol:   $r.symbol.clone(),
                order_id: $r.id.clone(),
            },
            Entering,
            PendingCancel,
            PendingCancel,
            $r.entry_side
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_exit_cancel_order {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::CancelOrder {
                symbol:   $r.symbol.clone(),
                order_id: $r.id.clone(),
            },
            Exiting,
            PendingCancel,
            PendingCancel,
            $r.entry_side.opposite()
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_entry_cancel_replace_order {
    ($r: expr) => {
        roundtrip_compute!($r, $r.instruction_cancel_and_exit(), Entering, PendingCancel, PendingCancel, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_exit_cancel_replace_order {
    ($r: expr) => {
        roundtrip_compute!($r, $r.instruction_cancel_and_exit(), Exiting, PendingCancel, PendingCancel, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_retry_exit {
    ($r: expr) => {
        let exit_retries_before_failing = $r.exit_retries_left.0;
        roundtrip_compute!($r, $r.instruction_normal_exit(), Exiting, ExitPending, Pending, $r.entry_side.opposite());
        assert_eq!($r.exit_retries_left.0, exit_retries_before_failing - 1);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_normal_exit {
    ($r: expr) => {
        roundtrip_compute!($r, $r.instruction_normal_exit(), Exiting, ExitPending, Pending, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_contingency_exit {
    ($r: expr) => {
        roundtrip_compute!($r, $r.instruction_contingency_exit(), Exiting, ExitPending, Pending, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_cancel_and_exit {
    ($r: expr) => {
        roundtrip_compute!($r, $r.instruction_cancel_and_exit(), Exiting, ExitPending, Pending, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_entry_finalized_with_failure {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         $r.is_cancelled_unexpectedly,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Failed,
            },
            Entering,
            Failed,
            OpenFailed,
            $r.entry_side
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_exit_successfully_finalized {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         $r.is_cancelled_unexpectedly,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Success,
            },
            Exiting,
            Finished,
            Finished,
            $r.entry_side.opposite()
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_entry_finalized_cancelled {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         $r.is_cancelled_unexpectedly,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Cancelled,
            },
            Entering,
            Cancelled,
            Finished,
            $r.entry_side
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_entry_finalized_cancelled_unexpectedly {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         true,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Cancelled,
            },
            Entering,
            Cancelled,
            Finished,
            $r.entry_side
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_exit_finalized_cancelled {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         $r.is_cancelled_unexpectedly,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Cancelled,
            },
            Exiting,
            Cancelled,
            Finished,
            $r.entry_side.opposite()
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_exit_finalized_cancelled_unexpectedly {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         true,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Cancelled,
            },
            Exiting,
            Cancelled,
            Finished,
            $r.entry_side.opposite()
        );
    };
}

#[macro_export]
macro_rules! roundtrip_compute_expecting_exit_finalized_with_failure {
    ($r: expr) => {
        roundtrip_compute!(
            $r,
            Instruction::Finalize {
                symbol:                            $r.symbol.clone(),
                order_id:                          $r.id.clone(),
                had_contingency_entry:             $r.had_contingency_entry,
                had_contingency_exit:              $r.had_contingency_exit,
                entry_retries_left:                $r.entry_retries_left.0,
                exit_retries_left:                 $r.exit_retries_left.0,
                is_cancelled_unexpectedly:         $r.is_cancelled_unexpectedly,
                do_emergency_exit_on_next_compute: $r.do_emergency_exit_on_next_compute,
                status:                            Status::Failed,
            },
            Exiting,
            Failed,
            OpenFailed,
            $r.entry_side.opposite()
        );
    };
}

#[macro_export]
macro_rules! roundtrip_assert {
    ($r: expr, $phase: ident, $rstate: ident, $ostate: ident, $side: expr) => {
        assert_eq!($r.phase(), TradingPhase::$phase);
        assert_eq!($r.roundtrip_state, RoundtripState::$rstate);
        assert_eq!($r.order_state, OrderState::$ostate);
        assert_eq!($r.current_side, $side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_entry_open_failed {
    ($r: expr) => {
        assert!($r.notify_open_failed().is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryPending);
        assert_eq!($r.order_state, OrderState::OpenFailed);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_exit_open_failed {
    ($r: expr) => {
        assert!($r.notify_open_failed().is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitPending);
        assert_eq!($r.order_state, OrderState::OpenFailed);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_entry_cancelled {
    ($r: expr) => {
        assert!($r.notify_cancelled().is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::PendingCancel);
        assert_eq!($r.order_state, OrderState::Cancelled);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_entry_cancelled_unexpectedly {
    ($r: expr) => {
        assert!($r.notify_cancelled().is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::Cancelled);
        assert_eq!($r.order_state, OrderState::Cancelled);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_cancelled {
    ($r: expr) => {
        assert!($r.notify_cancelled().is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::PendingCancel);
        assert_eq!($r.order_state, OrderState::Cancelled);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_cancelled_unexpectedly {
    ($r: expr) => {
        assert!($r.notify_cancelled().is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::Opened);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_entry_cancelled_and_pending_exit_replace {
    ($r: expr) => {
        roundtrip_assert!($r, Entering, PendingCancel, Cancelled, $r.entry_side);
        roundtrip_compute!($r, Exiting, PendingCancel, PendingReplace, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_entry_cancelled_but_pending_replace {
    ($r: expr) => {
        roundtrip_compute!($r, Entering, PendingCancel, PendingReplace, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_compute_assert_exit_cancelled_but_pending_replace {
    ($r: expr) => {
        roundtrip_compute!($r, Exiting, PendingCancel, PendingReplace, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_entry_replaced {
    ($r: expr) => {
        assert!($r.notify_replaced().is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        // assert_eq!($r.roundtrip_state, RoundtripState::PendingCancel);
        assert_eq!($r.order_state, OrderState::Replaced);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_replaced {
    ($r: expr) => {
        assert!($r.notify_replaced().is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        // assert_eq!($r.roundtrip_state, RoundtripState::PendingCancel);
        assert_eq!($r.order_state, OrderState::Replaced);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_entry_opened {
    ($r: expr) => {
        let entry_retries_before_failing = $r.entry_retries_left.0;

        assert!($r.notify_opened().is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryOpened);
        assert_eq!($r.order_state, OrderState::Opened);
        assert_eq!($r.current_side, $r.entry_side);
        assert_eq!($r.entry_retries_left.0, entry_retries_before_failing);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_opened {
    ($r: expr) => {
        let exit_retries_before_failing = $r.exit_retries_left.0;

        assert!($r.notify_opened().is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::Opened);
        assert_eq!($r.current_side, $r.entry_side.opposite());
        assert_eq!($r.exit_retries_left.0, exit_retries_before_failing);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_entry_partially_filled {
    ($r: expr) => {
        let partial_fill_quantity = OrderedFloat($r.entry_quantity.0 / 2.0);
        let partial_fill_quote = OrderedFloat(partial_fill_quantity.0 * $r.exit_price.0);

        assert!($r.notify_partially_filled(Some(partial_fill_quote), Some(partial_fill_quantity)).is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryOpened);
        assert_eq!($r.order_state, OrderState::PartiallyFilled);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_entry_filled {
    ($r: expr) => {
        let filled_quantity = $r.entry_quantity.0;
        let filled_quote = filled_quantity * $r.exit_price.0;

        assert!($r.notify_filled(Some(OrderedFloat(filled_quote)), Some(OrderedFloat(filled_quantity))).is_ok());
        assert_eq!($r.phase(), TradingPhase::Entering);
        assert_eq!($r.roundtrip_state, RoundtripState::EntryOpened);
        assert_eq!($r.order_state, OrderState::Filled);
        assert_eq!($r.current_side, $r.entry_side);
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_partially_filled {
    ($r: expr) => {
        let partial_fill_quantity = $r.filled_entry_quantity.0 / 2.0;
        let partial_fill_quote = partial_fill_quantity * $r.exit_price.0;

        assert!($r
            .notify_partially_filled(Some(OrderedFloat(partial_fill_quote)), Some(OrderedFloat(partial_fill_quantity)))
            .is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::PartiallyFilled);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_filled {
    ($r: expr) => {
        let filled_quantity = $r.filled_entry_quantity.0;
        let filled_quote = filled_quantity * $r.exit_price.0;

        assert!($r.notify_filled(Some(OrderedFloat(filled_quote)), Some(OrderedFloat(filled_quantity))).is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::ExitOpened);
        assert_eq!($r.order_state, OrderState::Filled);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}

#[macro_export]
macro_rules! roundtrip_notify_assert_exit_replaced_filled {
    ($r: expr) => {
        let filled_quantity = $r.filled_entry_qty.0;
        let filled_quote = filled_qty * $r.exit_price.0;

        assert!($r.notify_filled(Some(OrderedFloat(filled_quote)), Some(OrderedFloat(filled_qty))).is_ok());
        assert_eq!($r.phase(), TradingPhase::Exiting);
        assert_eq!($r.roundtrip_state, RoundtripState::PendingCancel);
        assert_eq!($r.order_state, OrderState::Filled);
        assert_eq!($r.current_side, $r.entry_side.opposite());
    };
}
