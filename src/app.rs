use chrono::{Datelike, Duration, NaiveDateTime, Timelike};
use egui::{
    hex_color,
    panel::{Side, TopBottomSide},
    plot::{CandleElem, ChartPlot, CoordinatesFormatter, Corner, HLine, Legend, Line, LineStyle, Plot, PlotBounds, PlotPoint, PlotPoints, PlotUi, Text},
    Align, Align2, Color32, CursorIcon, Key, Layout, Rgba, Stroke, TextBuffer, TextStyle, Ui, Visuals, Widget, WidgetText,
};
use egui_extras::{Column, Size, StripBuilder, TableBody, TableBuilder};
use indexmap::IndexMap;
use itertools::Itertools;
use log::{error, info, warn};
use num_traits::{Float, Zero};
use ordered_float::OrderedFloat;
use parking_lot::{lock_api::RawRwLockUpgrade, MappedMutexGuard, MappedRwLockReadGuard, Mutex, MutexGuard, RawRwLock, RwLock, RwLockReadGuard};
use portable_atomic::AtomicF64;
use rand::{rngs::OsRng, Rng, RngCore};
use std::{
    borrow::BorrowMut,
    collections::{BTreeMap, HashMap},
    mem::MaybeUninit,
    ops::RangeInclusive,
    sync::{atomic::Ordering, Arc},
};
use thiserror::Error;
use ustr::{ustr, Ustr, UstrMap};
use yata::core::{IndicatorResult, PeriodType, ValueType};

use crate::{
    appraiser::Appraiser,
    candle::Candle,
    coin::{Coin, CoinContext},
    coin_manager::CoinManager,
    components::util::{redgreen_with_bg, OrderBookLevelType, UiTimeframeConfig},
    config::Config,
    context::{Context, ContextInstance},
    egui_fill_rect, egui_formatted_value, egui_formatted_value_with_precision, egui_label, egui_labeled_string, egui_labeled_value, egui_labeled_widget,
    egui_string, egui_value, f,
    model::Balance,
    order::{Order, OrderState},
    orderbook::{OrderBook, OrderBookInner, Suggestion, SIZED_ORDERBOOK_LENGTH},
    orderbook_level::OrderBookLevel,
    plot_action::{PlotAction, PlotActionContext, PlotActionError, PlotActionResult, PlotActionState},
    plot_action_bias::PriceBiasAction,
    plot_action_ruler::RulerAction,
    plot_action_trade_builder::TradeBuilderAction,
    simulator::Simulator,
    state_analyzer::Analyzer,
    state_indicator::ComputedIndicatorState,
    support_resistance::MaType,
    timeframe::Timeframe,
    trade::{Trade, TradeBuilderConfig, TradeType},
    trade_closer::CloserKind,
    trader::Trader,
    trigger::{InstantTrigger, TriggerMetadata},
    types::{NormalizedOrderType, NormalizedSide, NormalizedTimeInForce, OrderedValueType, UiConfirmationResponse},
    util::{calc_relative_position_between, data_aspect_from_value, dump_to_file, hash_string, round_float_to_precision},
};

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Coin not found: {0}")]
    CoinNotFound(u64),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Copy, Clone, Default)]
pub struct AppConfig {
    pub show_cursor_hline: bool,
    pub allow_plot_drag:   bool,
    pub series_capacity:   usize,
}

impl AppConfig {
    pub fn new() -> Self {
        Self {
            show_cursor_hline: true,
            allow_plot_drag:   true,
            series_capacity:   0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppContext {
    pub symbol:                     Ustr,
    pub config:                     Config,
    pub app_config:                 AppConfig,
    pub trade_pending_confirmation: Option<Trade>,
}

pub struct App {
    pub title:             String,
    pub app_context:       Arc<Mutex<AppContext>>,
    pub coin_manager:      Arc<RwLock<CoinManager>>,
    pub coin_context:      Arc<RwLock<CoinContext>>,
    pub main_context:      Arc<RwLock<dyn Context>>,
    pub colormap:          IndexMap<&'static str, Color32>,
    pub timeframe:         Timeframe,
    pub timeframe_configs: HashMap<Timeframe, UiTimeframeConfig>,
    pub orders:            Arc<RwLock<IndexMap<u64, Order>>>,
    pub action:            Option<Box<dyn PlotAction>>,
}

impl App {
    /// Called once before the first frame.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Config,
        main_context: Arc<RwLock<ContextInstance>>,
        coin_manager: Arc<RwLock<CoinManager>>,
        symbol: Ustr,
    ) -> Self {
        cc.egui_ctx.set_visuals(Visuals::dark());
        cc.egui_ctx.memory_mut(|mem| *mem = Default::default());

        match cc.integration_info.native_pixels_per_point {
            None => cc.egui_ctx.set_pixels_per_point(1.25),
            Some(ppp) => cc.egui_ctx.set_pixels_per_point(ppp * 1.10),
        };

        let mut colormap = IndexMap::new();

        let coin_context = {
            match coin_manager.read().coins().get(&symbol) {
                None => panic!("{}: Coin not found", symbol),
                Some(coin) => coin.read().coin_context.clone(),
            }
        };

        Self {
            title: "Babylon".to_string(),
            coin_manager,
            coin_context: coin_context,
            app_context: Arc::new(Mutex::new(AppContext {
                config:                     config.clone(),
                symbol:                     Ustr::from("BTCUSDT"),
                app_config:                 AppConfig {
                    show_cursor_hline: true,
                    allow_plot_drag:   true,
                    series_capacity:   config.main.series_capacity,
                },
                trade_pending_confirmation: None,
            })),
            main_context,
            colormap,
            timeframe: Timeframe::M1,
            timeframe_configs: HashMap::from([
                (Timeframe::S1, UiTimeframeConfig { timeframe: Timeframe::S1 }),
                (Timeframe::M1, UiTimeframeConfig { timeframe: Timeframe::M1 }),
                (Timeframe::M3, UiTimeframeConfig { timeframe: Timeframe::M3 }),
                (Timeframe::M5, UiTimeframeConfig { timeframe: Timeframe::M5 }),
                (Timeframe::M15, UiTimeframeConfig { timeframe: Timeframe::M15 }),
                (Timeframe::M30, UiTimeframeConfig { timeframe: Timeframe::M30 }),
                (Timeframe::H1, UiTimeframeConfig { timeframe: Timeframe::H1 }),
                (Timeframe::H2, UiTimeframeConfig { timeframe: Timeframe::H2 }),
                (Timeframe::H4, UiTimeframeConfig { timeframe: Timeframe::H4 }),
                (Timeframe::H6, UiTimeframeConfig { timeframe: Timeframe::H6 }),
                (Timeframe::H8, UiTimeframeConfig { timeframe: Timeframe::H8 }),
                (Timeframe::H12, UiTimeframeConfig { timeframe: Timeframe::H12 }),
                (Timeframe::D1, UiTimeframeConfig { timeframe: Timeframe::D1 }),
                (Timeframe::D3, UiTimeframeConfig { timeframe: Timeframe::D3 }),
                (Timeframe::W1, UiTimeframeConfig { timeframe: Timeframe::W1 }),
                (Timeframe::MM1, UiTimeframeConfig { timeframe: Timeframe::MM1 }),
            ]),
            orders: Default::default(),
            action: None,
        }
    }

    pub fn load_coin_context(&mut self, symbol: Ustr) -> Result<(), AppError> {
        let coin_manager = self.coin_manager.read();
        let coins = coin_manager.coins();

        let coin = match coins.get(&symbol) {
            None => panic!("{}: Coin not found", symbol),
            Some(coin) => RwLockReadGuard::map(coin.read(), |m| m),
        };

        self.coin_context = coin.coin_context.clone();

        Ok(())
    }

    pub fn get_coin_names(&self) -> Result<Vec<Ustr>, AppError> {
        let coin_manager = self.coin_manager.read();
        let coins = coin_manager.coins().iter().map(|c| *c.0).collect_vec();
        Ok(coins)
    }

    pub fn ui_coinlist(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());

            let coin_manager = self.coin_manager.read();
            let coins = coin_manager.coins();

            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .columns(Column::auto(), 2)
                .resizable(false)
                .vscroll(true)
                .body(|mut body| {
                    for (&symbol, coin) in coins.iter() {
                        body.row(20.0, |mut row| {
                            row.col(|ui| {
                                if ui.link(symbol.as_str()).clicked() {
                                    // self.load_coin_context(symbol).unwrap();

                                    let coin = match coins.get(&symbol) {
                                        None => panic!("{}: Coin not found", symbol),
                                        Some(coin) => RwLockReadGuard::map(coin.read(), |m| m),
                                    };

                                    self.app_context.lock().symbol = symbol;
                                    self.coin_context = coin.coin_context.clone();
                                }
                            });
                            row.col(|ui| {
                                egui_label!(ui, coin.read().price().0, Color32::WHITE);
                            });
                        });
                    }
                });
        });
    }

    pub fn handle_input(&mut self, plot_ui: &mut PlotUi) {
        self.app_context.lock().app_config.show_cursor_hline = plot_ui.ctx().input(|i| !i.keys_down.is_empty());
    }

    // FIXME: this is horrible, fix this
    fn price(&self, symbol: Ustr) -> Option<OrderedValueType> {
        let coin_manager = self.coin_manager.read();
        let coin_map = coin_manager.coins();

        let coin = match coin_map.get(&symbol) {
            None => {
                error!("failed to get price, coin not found: {}", symbol);
                return None;
            }
            Some(coin) => coin.read(),
        };

        Some(coin.price())
    }

    fn ui_metadata(&mut self, ui: &mut Ui, symbol: Ustr) {
        ui.vertical_centered_justified(|ui| {
            ui.set_max_width(400.0);
            ui.heading("General");
            ui.separator();

            egui::Grid::new("metadata").num_columns(2).spacing([30.0, 4.0]).show(ui, |ui| {
                // egui_labeled_value!(ui, "Price", self.price(symbol).unwrap_or_default());
                ui.add(egui::Label::new("Price"));
                ui.add(egui::Label::new(format!("{}", self.price(symbol).unwrap_or_default())));
                ui.end_row();
            });
        });

        ui.add_space(20.0);

        // ...
    }

    pub fn ui_moving_averages(&self, ui: &mut Ui, symbol: Ustr) {
        let coin_manager = self.coin_manager.read();
        let coin_map = coin_manager.coins();

        let coin = match coin_map.get(&symbol) {
            None => {
                error!("failed to get price, coin not found: {}", symbol);
                return;
            }
            Some(coin) => coin.read(),
        };

        // println!("{:#?}", coin.head_state(self.timeframe).unwrap().moving_average.iter().collect_vec());

        /*
        ui.vertical_centered_justified(|ui| {
            ui.set_max_width(400.0);
            ui.separator();

            // header
            ui.heading("Moving Averages");
            ui.separator();

            egui::Grid::new("moving_averages").num_columns(2).spacing([30.0, 4.0]).show(ui, |ui| {
                egui_labeled_value!(ui, "Symbol", self.price(symbol).unwrap_or_default());
                ui.end_row();

                /*
                coin.head_state(self.timeframe).unwrap().moving_average.iter().for_each(|ma| {
                    egui_labeled_value!(ui, format!("{}-{}", ma.0, ma.1).as_str(), ma.2 .0);
                });
                 */
            });

                ui.end_row();
            });
         */

        ui.add_space(20.0);
    }

    pub fn show_cursor_line(&mut self, plot_ui: &mut PlotUi) {
        if self.action.is_none() {
            return;
        }

        let symbol = self.app_context.lock().symbol;

        let price = match self.price(symbol) {
            None => {
                error!("failed to get price, coin not found: {}", symbol);
                return;
            }
            Some(price) => price,
        };

        if self.app_context.lock().app_config.show_cursor_hline {
            if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
                plot_ui.ctx().set_cursor_icon(CursorIcon::None);

                let cursor_color = match cursor_pos.y.total_cmp(&price) {
                    std::cmp::Ordering::Less => Color32::RED,
                    std::cmp::Ordering::Equal => Color32::GRAY,
                    std::cmp::Ordering::Greater => Color32::GREEN,
                };

                plot_ui.text(
                    Text::new(
                        PlotPoint::new(cursor_pos.x, cursor_pos.y),
                        WidgetText::from(format!("{:0.2?}", cursor_pos.y)).text_style(TextStyle::Body).strong(),
                    )
                    .highlight(false)
                    .anchor(Align2::LEFT_BOTTOM)
                    .color(cursor_color),
                );

                plot_ui.hline(HLine::new(cursor_pos.y, None).color(cursor_color).width(1.0).highlight(true));
            }
        }
    }

    pub fn show_price_line(&mut self, plot_ui: &mut PlotUi, candle: Candle) {
        plot_ui.hline(
            HLine::new(candle.close, None)
                .color(if candle.close >= candle.open { Color32::GREEN } else { Color32::RED })
                .style(LineStyle::Dashed { length: 1.0 })
                .highlight(true),
        );
    }

    pub fn trade_form(&mut self, ui: &mut Ui, trade: &mut Trade) -> Result<(), AppError> {
        // Displaying basic order information
        ui.horizontal(|ui| {
            ui.label("Trade ID:");
            ui.label(format!("{}", trade.id));
        });

        // Editable field for initial_quantity
        ui.horizontal(|ui| {
            ui.label("Initial Quantity:");
            ui.add(egui::DragValue::new(&mut trade.entry.initial_quantity.0));
        });

        Ok(())
    }

    fn confirmation_dialog<T: FnOnce(&mut Ui)>(ui: &mut Ui, title: &str, content: T) -> Result<UiConfirmationResponse, AppError> {
        let mut response = UiConfirmationResponse::NoResponse;

        let mut window = egui::Window::new(title)
            .collapsible(false)
            .fixed_pos([ui.available_width() / 2.0, ui.available_height() / 2.0]);

        window.show(ui.ctx(), |ui| {
            content(ui);

            ui.vertical_centered_justified(|ui| {
                if ui.button("Confirm").clicked() {
                    response = UiConfirmationResponse::Confirmed;
                }

                if ui.button("Cancel").clicked() {
                    response = UiConfirmationResponse::Cancelled;
                }
            });

            ui.input(|i| {
                if i.key_pressed(Key::Escape) {
                    response = UiConfirmationResponse::Cancelled;
                    return;
                }

                if i.key_pressed(Key::Enter) {
                    response = UiConfirmationResponse::Confirmed;
                    return;
                }
            });
        });

        Ok(response)
    }

    pub fn plot_timeframe(&mut self, ui: &mut Ui, data_aspect: f32, symbol: Ustr, ui_timeframe_config: UiTimeframeConfig) {
        let coin_manager = self.coin_manager.read();
        let coins = coin_manager.coins();

        let coin = match coins.get(&symbol) {
            None => panic!("{}: Coin not found", symbol),
            Some(coin) => RwLockReadGuard::map(coin.read(), |m| m),
        };

        let symbol = coin.symbol();
        let appraiser = coin.appraiser();
        let quote_asset = coin.quote_asset;
        let base_asset = coin.base_asset;
        let quote_precision = appraiser.true_quote_precision;
        let base_precision = appraiser.true_base_precision;
        let current_market_price = coin.price();
        let quote_balance = coin.coin_context.read().balance.get(quote_asset).unwrap_or(Balance::default());
        let base_balance = coin.coin_context.read().balance.get(base_asset).unwrap_or(Balance::default());

        let candle_width = ui_timeframe_config.timeframe as u64 as f64 * 0.6;
        let whisker_width = ui_timeframe_config.timeframe as u64 as f64 * 0.3;

        let mut head_candle = coin.series.get(ui_timeframe_config.timeframe).unwrap().head_state().unwrap().candle.clone();
        let mut head_timestamp = head_candle.open_time.timestamp_millis();

        let timeframe = ui_timeframe_config.timeframe;
        // let timeframe = self.timeframe;
        let price = round_float_to_precision(head_candle.close, appraiser.true_quote_precision);
        let mut mas: UstrMap<Vec<(PeriodType, Ustr, [f64; 2])>> = UstrMap::default();

        let time_axis_formatter = move |x: f64, _range: &RangeInclusive<f64>| {
            let ts = match NaiveDateTime::from_timestamp_millis(x as i64) {
                None => return String::new(),
                Some(ts) => ts.and_utc(),
            };

            format!(
                "{year:04}.{month:02}.{day:02} {h:02}:{m:02}:{s:02}:{ms:03}",
                year = ts.year(),
                month = ts.month(),
                day = ts.day(),
                h = ts.hour(),
                m = ts.minute(),
                s = ts.second(),
                ms = ts.timestamp_subsec_millis()
            )
        };

        let price_axis_formatter = move |price: f64, _range: &RangeInclusive<f64>| format!("{:.d$}", price, d = quote_precision as usize);

        let coord_fmt = move |p: &PlotPoint, b: &PlotBounds| {
            let ts = match NaiveDateTime::from_timestamp_millis(p.x as i64) {
                None => return String::new(),
                Some(ts) => ts.and_utc(),
            };

            format!(
                "{y:.d$}\n{year:04}.{month:02}.{day:02} {h:02}:{m:02}:{s:02}:{ms:03}",
                y = round_float_to_precision(p.y, quote_precision),
                year = ts.year(),
                month = ts.month(),
                day = ts.day(),
                h = ts.hour(),
                m = ts.minute(),
                s = ts.second(),
                ms = ts.timestamp_subsec_millis(),
                d = quote_precision as usize
            )
        };

        let mut plot = Plot::new(symbol)
            .allow_boxed_zoom(true)
            .allow_drag(self.app_context.lock().app_config.allow_plot_drag)
            .legend(Legend::default())
            .x_axis_formatter(time_axis_formatter)
            .y_axis_formatter(price_axis_formatter)
            .data_aspect(data_aspect)
            .show_axes([false, true])
            .show_background(true)
            .height(ui.available_height())
            .show_y(false)
            .show_x(false)
            .coordinates_formatter(Corner::RightBottom, CoordinatesFormatter::new(coord_fmt));

        plot.show(ui, |plot_ui| {
            /*
            for (id, order) in self.simulator.orders().iter() {
                let mut order_line = HLine::new(order.price, Some(order.id.to_string()));
                let alpha = if order.state == OrderState::Opened { 1.0 } else { 0.1 };

                order_line = match (order.side, order.order_type) {
                    (NormalizedSide::Buy, NormalizedOrderType::Limit) =>
                        order_line.stroke(Stroke::new(1.0, Rgba::from(Color32::GREEN).to_opaque().multiply(alpha))),
                    (NormalizedSide::Buy, NormalizedOrderType::LimitMaker) =>
                        order_line.stroke(Stroke::new(2.0, Rgba::from(Color32::GREEN).to_opaque().multiply(alpha))),
                    (NormalizedSide::Sell, NormalizedOrderType::Limit) =>
                        order_line.stroke(Stroke::new(1.0, Rgba::from(Color32::RED).to_opaque().multiply(alpha))),
                    (NormalizedSide::Sell, NormalizedOrderType::LimitMaker) =>
                        order_line.stroke(Stroke::new(2.0, Rgba::from(Color32::RED).to_opaque().multiply(alpha))),
                    (NormalizedSide::Sell, NormalizedOrderType::StopLoss) => order_line
                        .style(LineStyle::dashed_dense())
                        .stroke(Stroke::new(1.0, Rgba::from(Color32::LIGHT_RED).to_opaque().multiply(alpha))),
                    (NormalizedSide::Sell, NormalizedOrderType::StopLossLimit) => order_line
                        .style(LineStyle::dashed_dense())
                        .stroke(Stroke::new(1.0, Rgba::from(Color32::LIGHT_RED).to_opaque().multiply(alpha))),

                    _ => order_line.stroke(Stroke::new(1.0, Rgba::from(Color32::WHITE).to_opaque().multiply(alpha))),
                };

                plot_ui.hline(order_line);
            }
             */

            /*
            let trader = self.main_context.read().get_trader();
            let ccx = trader.get_context(self.app_context.lock().symbol).expect("failed to get trader's coin context");
            let trades = trader
                .get_trades(ccx.symbol)
                .expect("failed to get trader's trades")
                .values()
                .cloned()
                .collect_vec();

            drop(ccx);
            drop(trader);

            for trade in trades {
                if trade.is_closed() {
                    continue;
                }

                for order in [trade.entry, trade.exit, trade.stoploss].iter() {
                    let mut order_line = HLine::new(order.price, Some(trade.id.to_string()));
                    let alpha = if order.state == OrderState::Opened { 1.0 } else { 0.1 };

                    order_line = match (order.side, order.order_type) {
                        (NormalizedSide::Buy, NormalizedOrderType::Limit) =>
                            order_line.stroke(Stroke::new(1.0, Rgba::from(Color32::GREEN).to_opaque().multiply(alpha))),
                        (NormalizedSide::Buy, NormalizedOrderType::LimitMaker) =>
                            order_line.stroke(Stroke::new(2.0, Rgba::from(Color32::GREEN).to_opaque().multiply(alpha))),
                        (NormalizedSide::Sell, NormalizedOrderType::Limit) =>
                            order_line.stroke(Stroke::new(1.0, Rgba::from(Color32::RED).to_opaque().multiply(alpha))),
                        (NormalizedSide::Sell, NormalizedOrderType::LimitMaker) =>
                            order_line.stroke(Stroke::new(2.0, Rgba::from(Color32::RED).to_opaque().multiply(alpha))),
                        (NormalizedSide::Sell, NormalizedOrderType::Stoploss) => order_line
                            .style(LineStyle::dashed_dense())
                            .stroke(Stroke::new(1.0, Rgba::from(Color32::LIGHT_RED).to_opaque().multiply(alpha))),
                        (NormalizedSide::Sell, NormalizedOrderType::StoplossLimit) => order_line
                            .style(LineStyle::dashed_dense())
                            .stroke(Stroke::new(1.0, Rgba::from(Color32::LIGHT_RED).to_opaque().multiply(alpha))),

                        _ => order_line.stroke(Stroke::new(1.0, Rgba::from(Color32::WHITE).to_opaque().multiply(alpha))),
                    };

                    plot_ui.hline(order_line);
                }
            }
             */

            /*
            plot_ui.chart_plot(
                ChartPlot::new(self.simulator.generator().timeframes.get(self.timeframe).unwrap().ui_candles.clone())
                    .name(symbol)
                    .color(Color32::LIGHT_BLUE),
            );
             */

            // ----------------------------------------------------------------------------
            // Preparing the candles
            // ----------------------------------------------------------------------------

            let ui_candles = coin
                .series
                .get(self.timeframe)
                .unwrap()
                .iter_candles_back_to_front()
                .map(|c| {
                    // IMPORTANT: We need to keep track of the head candle
                    head_timestamp = c.open_time.timestamp_millis();
                    head_candle = c.clone();

                    let color = if c.close < c.open { Color32::RED } else { Color32::GREEN };

                    CandleElem {
                        x: c.open_time.timestamp_millis() as f64,
                        candle: egui::widgets::plot::Candle {
                            timeframe:                    c.timeframe as u64,
                            open_time:                    c.open_time,
                            close_time:                   c.close_time,
                            open:                         c.open,
                            high:                         c.high,
                            low:                          c.low,
                            close:                        c.close,
                            volume:                       c.volume,
                            number_of_trades:             c.number_of_trades as u32,
                            quote_asset_volume:           c.quote_asset_volume,
                            taker_buy_quote_asset_volume: c.taker_buy_quote_asset_volume,
                            taker_buy_base_asset_volume:  c.taker_buy_base_asset_volume,
                        },
                        candle_width,
                        whisker_width,
                        stroke: Stroke::new(1.0, color),
                        fill: color,
                    }
                })
                .collect_vec();

            plot_ui.chart_plot(ChartPlot::new(ui_candles).name(symbol).color(Color32::LIGHT_BLUE));

            // ----------------------------------------------------------------------------
            // Moving Averages
            // ----------------------------------------------------------------------------

            let mut mas = BTreeMap::<(MaType, usize), Vec<[f64; 2]>>::new();

            let num_states = coin.series.get(self.timeframe).unwrap().num_states();

            for s in coin.series.get(self.timeframe).unwrap().iter_states_back_to_front() {
                for ma in s.ma.iter() {
                    match mas.entry((ma.0, ma.1)) {
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            entry.get_mut().push([s.candle.open_time.timestamp_millis() as ValueType, ma.2 .0]);
                        }
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(vec![]);
                        }
                    }
                }
            }

            for ma in mas {
                let color = match ma.0 {
                    (MaType::EMA, period) => Color32::RED,
                    (MaType::WMA, period) => Color32::GREEN,
                    (MaType::WSMA, period) => Color32::LIGHT_BLUE,
                    (MaType::WWMA, period) => Color32::GOLD,
                    _ => Color32::WHITE,
                };

                plot_ui.line(
                    Line::new(PlotPoints::new(ma.1))
                        .name(format!("{}-{}", ma.0 .0.to_string(), ma.0 .1))
                        .color(color)
                        .width(1.0),
                );
            }

            // NOTE: whichever action manages to initialize becomes the active event
            match self.action {
                None => {
                    // ruler
                    if plot_ui.ctx().input(|i| i.modifiers.shift && i.pointer.primary_down()) {
                        if let Some(start_point) = plot_ui.pointer_coordinate() {
                            let action_context = PlotActionContext::new(None, self.app_context.clone(), self.coin_context.clone(), Some(start_point));

                            let Some(ruler) = RulerAction::new(action_context) else {
                                return;
                            };

                            self.action = Some(Box::new(ruler));
                        }
                    }

                    // ----------------------------------------------------------------------------
                    // Build a new trade
                    // ENTRY: Buy LimitMaker Order
                    // EXIT: Sell LimitMaker Order
                    // STOP: Stoploss Limit Order
                    // CONTINGENCY: Market Order
                    // ----------------------------------------------------------------------------
                    if plot_ui.ctx().input(|i| i.key_down(Key::T)) {
                        if let Some(start_point) = plot_ui.pointer_coordinate() {
                            let action_context = PlotActionContext::new(Some(Key::T), self.app_context.clone(), coin.coin_context.clone(), Some(start_point));
                            let builder_config = TradeBuilderConfig {
                                specific_trade_id: None,
                                symbol,
                                timeframe: self.timeframe,
                                fee_ratio: Some(f!(0.001)),
                                trade_type: TradeType::Long,
                                quantity: (quote_balance.total() * f!(OsRng.gen_range(0.05..=0.25))) / current_market_price,
                                entry_order_type: NormalizedOrderType::LimitMaker,
                                entry_activator: TriggerMetadata::instant(),
                                entry_deactivator: TriggerMetadata::none(),
                                entry_time_in_force: NormalizedTimeInForce::GTC,
                                exit_order_type: NormalizedOrderType::Limit,
                                // exit_activator: TriggerMetadata::proximity(f!(0.0), f!(0.0), f!(0.10)),
                                exit_activator: TriggerMetadata::instant(),
                                // exit_activator: TriggerMetadata::instant(),
                                exit_deactivator: TriggerMetadata::none(),
                                exit_time_in_force: NormalizedTimeInForce::GTC,
                                stoploss_order_type: NormalizedOrderType::StopLossLimit,
                                stoploss_activator: TriggerMetadata::instant(),
                                atomic_market_price: self.coin_context.read().atomic_price.clone(),
                                is_closer_required: true,
                                is_stoploss_required: true,
                                closer_kind: CloserKind::TakeProfit,
                            };

                            let Ok(new_action) = TradeBuilderAction::new(action_context, self.main_context.read().get_trader(), builder_config) else {
                                return;
                            };

                            self.action = Some(Box::new(new_action));
                        }
                    }
                }

                Some(ref mut action) => {
                    self.app_context.lock().app_config.allow_plot_drag = false;

                    // NOTE: Escape is used to cancel and clear any active action
                    if plot_ui.ctx().input(|i| i.key_released(Key::Escape)) {
                        action.cancel();
                        self.app_context.lock().app_config.allow_plot_drag = true;
                        self.action = None;
                        return;
                    }

                    action.draw(plot_ui);

                    match action.compute(plot_ui) {
                        Ok(None) => {
                            // NOTE: action is still pending
                        }

                        Ok(Some(action_result)) => match (action.state(), action_result) {
                            (PlotActionState::Finished, result) => match result {
                                PlotActionResult::Trade(new_trade) => {
                                    eprintln!("new_trade = {:#?}", new_trade);

                                    self.app_context.lock().trade_pending_confirmation = Some(new_trade);
                                    self.app_context.lock().app_config.allow_plot_drag = true;
                                    self.action = None;
                                }

                                _ => {}
                            },

                            (PlotActionState::Cancelled, result) => {
                                match result {
                                    _ => {
                                        panic!("unhandled action cancelled: {:?}", result);
                                    }
                                }

                                self.app_context.lock().app_config.allow_plot_drag = true;
                                self.action = None;
                            }

                            _ => {}
                        },

                        Err(err) => {
                            error!("action error: {}", err);
                            self.app_context.lock().app_config.allow_plot_drag = true;
                            self.action = None;
                        }
                    }
                }
            };

            // ----------------------------------------------------------------------------
            // Price line
            // ----------------------------------------------------------------------------

            plot_ui.hline(
                HLine::new(head_candle.close, None)
                    .color(if head_candle.close >= head_candle.open { Color32::GREEN } else { Color32::RED })
                    .style(LineStyle::Dashed { length: 3.0 })
                    .highlight(true),
            );

            // ----------------------------------------------------------------------------
            // Cursor line
            // ----------------------------------------------------------------------------
            if self.action.is_some() {
                if self.app_context.lock().app_config.show_cursor_hline {
                    if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
                        plot_ui.ctx().set_cursor_icon(CursorIcon::None);

                        let cursor_color = match cursor_pos.y.total_cmp(&price) {
                            std::cmp::Ordering::Less => Color32::RED,
                            std::cmp::Ordering::Equal => Color32::GRAY,
                            std::cmp::Ordering::Greater => Color32::GREEN,
                        };

                        plot_ui.text(
                            Text::new(
                                PlotPoint::new(cursor_pos.x, cursor_pos.y),
                                WidgetText::from(format!("{:0.2?}", cursor_pos.y)).text_style(TextStyle::Body).strong(),
                            )
                            .highlight(false)
                            .anchor(Align2::LEFT_BOTTOM)
                            .color(cursor_color),
                        );

                        plot_ui.hline(HLine::new(cursor_pos.y, None).color(cursor_color).width(1.0).highlight(true));
                    }
                }
            }
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, cx: &egui::Context, frame: &mut eframe::Frame) {
        let symbol = self.app_context.lock().symbol;
        let trailing_window_size = self.app_context.lock().app_config.series_capacity;
        let mut trader = self.main_context.write().get_trader();

        // loading initial coin context
        self.load_coin_context(symbol).unwrap();

        if cx.input(|i| i.key_pressed(Key::F)) {
            frame.set_fullscreen(!frame.info().window_info.fullscreen);
        }

        egui::TopBottomPanel::new(TopBottomSide::Top, "timeframes").show(cx, |ui| {
            let coin_manager = self.coin_manager.read();
            let coin_map = coin_manager.coins();

            let coin = match coin_map.get(&symbol) {
                None => {
                    error!("coin not found: {}", symbol);
                    return;
                }
                Some(coin) => coin.read(),
            };

            let quote_asset = coin.quote_asset;
            let base_asset = coin.base_asset;
            let quote_balance = coin.coin_context.read().balance.get(quote_asset).unwrap_or(Balance::default());
            let base_balance = coin.coin_context.read().balance.get(base_asset).unwrap_or(Balance::default());
            let s = coin.series.get(self.timeframe).unwrap().head_state().unwrap();

            ui.horizontal(|ui| {
                let cx = coin.context();

                // TODO: display the current timeframe
                // TODO: display whether the series are live

                egui_labeled_string!(ui, "Symbol", coin.context().symbol.as_str());
                egui_formatted_value_with_precision!(ui, "Price", s.candle.close, cx.appraiser.true_quote_precision as usize);
                egui_formatted_value_with_precision!(ui, "RSI", s.i.rsi_14.value(0) * 100.0, 2);
                egui_formatted_value_with_precision!(ui, "RSI (MA)", s.i.rsi_14.value(1) * 100.0, 2);
                egui_formatted_value_with_precision!(ui, "EMA 3", s.i.ema_3, cx.appraiser.true_quote_precision as usize);
                egui_formatted_value_with_precision!(ui, "STD (EMA 3)", s.i.ema_3_stdev, cx.appraiser.true_quote_precision as usize);
                egui_formatted_value_with_precision!(ui, "STD (LOG(TR))", s.i.stdev_tr_ln, cx.appraiser.true_quote_precision as usize);
                egui_formatted_value_with_precision!(ui, "STD (HLD)", s.i.stdev_high_low, 4);
                egui_formatted_value_with_precision!(ui, "STD (OCD)", s.i.stdev_open_close, 4);
                egui_labeled_string!(ui, "Computation Time", format!("{:?}", s.total_computation_time));

                for (timeframe, timeframe_config) in coin.context().timeframe_configs.iter() {
                    ui.radio_value(&mut self.timeframe, timeframe, timeframe.to_string());
                }

                egui_formatted_value_with_precision!(ui, "Q", quote_balance.total(), cx.appraiser.true_quote_precision as usize);
                egui_formatted_value_with_precision!(ui, "B", base_balance.total(), cx.appraiser.true_base_precision as usize);
            });
        });

        egui::SidePanel::new(Side::Left, "left_sidebar")
            .max_width(350.0)
            .exact_width(350.0)
            .resizable(false)
            .show(cx, |ui| {
                self.ui_orderbook(ui, symbol);
                ui.end_row();

                self.ui_trades(ui, symbol);
                ui.end_row();
            });

        /*
        egui::SidePanel::new(Side::Right, "right_sidebar")
            .max_width(350.0)
            .resizable(false)
            .show(cx, |ui| {
                ui.vertical(|ui| {
                    if ui.button("Print 10 last candles").clicked() {
                        let coin_manager = self.coin_manager.read();
                        let coin_map = coin_manager.coins();

                        let coin = match coin_map.get(&symbol) {
                            None => {
                                error!("coin not found: {}", symbol);
                                return;
                            }
                            Some(coin) => coin.read(),
                        };

                        let series = coin.series.get(self.timeframe).unwrap();
                        let mut candles = series.iter_candles_front_to_back().take(10).collect_vec();
                        candles.reverse();

                        println!("{:#?}", candles);
                    }
                    ui.set_width(ui.available_width());

                    self.ui_metadata(ui, symbol);
                    self.ui_moving_averages(ui, symbol);
                });
                ui.end_row();
            });
         */

        egui::CentralPanel::default().show(cx, |ui| {
            if let Some(price) = self.price(symbol) {
                let data_aspect = (data_aspect_from_value(price.0) as f32);

                let adjusted_data_aspect = match self.timeframe {
                    Timeframe::NA => panic!("timeframe not set"),
                    Timeframe::MS100 => data_aspect * 5.0,
                    Timeframe::MS250 => data_aspect * 4.0,
                    Timeframe::MS500 => data_aspect * 2.0,
                    Timeframe::S1 => data_aspect,
                    Timeframe::S3 => data_aspect / 3.0,
                    Timeframe::S5 => data_aspect / 3.0,
                    Timeframe::S15 => data_aspect / 3.0,
                    Timeframe::S30 => data_aspect / 3.0,
                    Timeframe::M1 => data_aspect / 3.0,
                    Timeframe::M3 => data_aspect / 3.0,
                    Timeframe::M5 => data_aspect / 2.5,
                    Timeframe::M15 => data_aspect / 2.25,
                    Timeframe::M30 => data_aspect / 1.75,
                    Timeframe::H1 => data_aspect,
                    Timeframe::H2 => data_aspect * 1.5,
                    Timeframe::H4 => data_aspect * 1.75,
                    Timeframe::H6 => data_aspect * 2.0,
                    Timeframe::H8 => data_aspect * 2.25,
                    Timeframe::H12 => data_aspect * 2.5,
                    Timeframe::D1 => data_aspect * 3.0,
                    Timeframe::D3 => data_aspect * 5.0,
                    Timeframe::W1 => data_aspect * 6.0,
                    Timeframe::MM1 => data_aspect * 10.0,
                };

                ui.push_id("plot_name_change_later", |ui| {
                    self.plot_timeframe(
                        ui,
                        // adjusted_data_aspect,
                        1000.0,
                        symbol,
                        self.timeframe_configs[&self.timeframe],
                    );
                });
            }

            // ----------------------------------------------------------------------------
            // Handling pending trades (if there are any)
            // ----------------------------------------------------------------------------
            let new_pending_trade = self.app_context.lock().trade_pending_confirmation.clone();

            if let Some(mut new_trade) = new_pending_trade {
                let confirmation_response = Self::confirmation_dialog(ui, "Trade Confirmation", |ui| {
                    self.trade_form(ui, &mut new_trade).expect("failed to display trade form");

                    ui.set_max_width(300.0);
                    ui.vertical_centered_justified(|ui| {
                        ui.add_space(10.0);
                        ui.label("Are you sure you want to create this trade?");
                        ui.add_space(10.0);
                    });
                });

                match confirmation_response {
                    Ok(response) => match response {
                        UiConfirmationResponse::Confirmed => {
                            // if let Some(new_trade) = self.app_context.lock().trade_pending_confirmation.take() {
                            trader.create_trade(new_trade.clone()).expect("failed to add trade to trader");
                            self.app_context.lock().trade_pending_confirmation = None;

                            /*
                            self.simulator
                                .create_order(SimulatorOrder {
                                    id: new_order.local_id,
                                    parent_order_id: None,
                                    client_order_id: new_order.local_id,
                                    client_stop_limit_order_id: Some(new_order.stoploss.local_stop_limit_order_id),
                                    symbol,
                                    side: new_order.side,
                                    order_type: new_order.order_type,
                                    time_in_force: NormalizedTimeInForce::GTC,
                                    price: new_order.price,
                                    stop_price: Some(new_order.stoploss.stop_price),
                                    stop_limit_price: Some(new_order.stoploss.limit_price),
                                    initial_quantity: new_order.initial_quantity,
                                    filled_quantity: f!(0.0),
                                    fills: vec![],
                                    state: OrderState::Opened,
                                    is_stop: false,
                                    is_oco: false,
                                    events: vec![],
                                })
                                .expect("failed to add order to simulator");

                            // FIXME: this is a hack, we need to find a way to add orders to the trader
                            trader.create_raw_order(new_order).expect("failed to add order to local orders");
                             */
                            // }
                        }
                        UiConfirmationResponse::Cancelled => {
                            self.app_context.lock().trade_pending_confirmation = None;
                        }
                        UiConfirmationResponse::NoResponse => {}
                    },
                    Err(err) => {
                        error!("confirmation dialog error: {}", err);
                    }
                }
            }
        });

        // ctx.request_repaint_after(core::time::Duration::from_millis(100));
        cx.request_repaint();
    }
}
