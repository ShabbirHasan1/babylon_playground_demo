use crate::{
    app::App,
    candle::Candle,
    components::util::UiTimeframeConfig,
    plot_action::{PlotActionContext, PlotActionResult, PlotActionState},
    plot_action_ruler::RulerAction,
    support_resistance::SRLevel,
    util::round_float_to_precision,
};
use chrono::{Datelike, Duration, NaiveDateTime, TimeZone, Timelike};
use egui::{
    plot::{
        Candle as CandleStick, CandleElem, ChartPlot, CoordinatesFormatter, Corner, HLine, Legend, Line, LineStyle, Plot, PlotBounds, PlotPoint, PlotPoints,
        Text,
    },
    Align2, Color32, CursorIcon, Key, Stroke, TextStyle, Ui, WidgetText,
};
use itertools::Itertools;
use log::{info, warn};
use parking_lot::RwLockReadGuard;
use std::{
    collections::hash_map::Entry,
    ops::{Add, RangeInclusive},
};
use ustr::{ustr, Ustr, UstrMap};
use yata::core::{PeriodType, ValueType};

impl App {
    fn filter_values<'a>(&self, vec: &'a [SRLevel]) -> impl Iterator<Item = &'a SRLevel> {
        let len = vec.len();
        vec.iter()
            .enumerate()
            .filter(move |(i, item)| {
                let left = if *i > 0 { vec[i - 1].period } else { item.period };
                let right = if *i < len - 1 { vec[i + 1].period } else { item.period };
                item.period >= left && item.period >= right
            })
            .map(|(_, item)| item)
    }

    fn filter_values_opposite<'a>(&self, vec: &'a [SRLevel]) -> impl Iterator<Item = &'a SRLevel> {
        let len = vec.len();
        vec.iter()
            .enumerate()
            .filter(move |(i, item)| {
                let left = if *i > 0 { vec[*i - 1].period } else { item.period };
                let right = if *i < len - 1 { vec[*i + 1].period } else { item.period };
                let opposite = if left > right {
                    if *i < len - 1 {
                        vec[*i + 1].period
                    } else {
                        item.period
                    }
                } else {
                    if *i > 0 {
                        vec[*i - 1].period
                    } else {
                        item.period
                    }
                };
                item.period >= opposite
            })
            .map(|(_, item)| item)
    }

    /*
    pub fn plot_timeframe(
        &mut self,
        ui: &mut Ui,
        symbol: Ustr,
        ui_timeframe_config: UiTimeframeConfig,
        trailing_window_size: PeriodType,
        take_profit_pc: ValueType,
        fee_pc: ValueType,
        with_stop_loss: bool,
        data_aspect: f32,
    ) {
        let coin_manager = self.coin_manager.read();
        let coins = coin_manager.coins();

        let coin = match coins.get(&symbol) {
            None => panic!("{}: Coin not found", symbol),
            Some(coin) => RwLockReadGuard::map(coin.read(), |m| m),
        };

        let quote_precision = coin.appraiser().true_quote_precision;
        let base_precision = coin.appraiser().true_base_precision;
        let candle_width = ui_timeframe_config.timeframe as u64 as f64 * 0.80;
        let whisker_width = candle_width * 0.5;
        let series = coin.series.get(ui_timeframe_config.timeframe).unwrap();
        // let states = coin.states(ui_timeframe_config.timeframe, 0, length as PeriodType).unwrap();
        let states = coin.states(ui_timeframe_config.timeframe, 0, trailing_window_size).unwrap();
        let mut head_timestamp = 0i64;
        let mut head_candle = Candle::default();

        let mut candlesticks = states
            .iter()
            .map(|s| {
                head_timestamp = s.candle.open_time.timestamp();
                head_candle = s.candle;
                let color = if s.candle.close < s.candle.open { Color32::RED } else { Color32::GREEN };

                CandleElem {
                    x: s.candle.open_time.timestamp() as f64,
                    candle: CandleStick {
                        timeframe:                    s.candle.timeframe as u64,
                        open_time:                    s.candle.open_time.naive_utc(),
                        close_time:                   s.candle.close_time.naive_utc(),
                        open:                         s.candle.open,
                        high:                         s.candle.high,
                        low:                          s.candle.low,
                        close:                        s.candle.close,
                        volume:                       s.candle.volume,
                        number_of_trades:             s.candle.number_of_trades as u32,
                        quote_asset_volume:           s.candle.quote_asset_volume,
                        taker_buy_quote_asset_volume: s.candle.taker_buy_quote_asset_volume,
                        taker_buy_base_asset_volume:  s.candle.taker_buy_base_asset_volume,
                    },
                    candle_width,
                    whisker_width,
                    stroke: Stroke::new(1.0, color),
                    fill: color,
                }
            })
            .collect_vec();

        let timeframe = ui_timeframe_config.timeframe;
        let price = round_float_to_precision(head_candle.close, coin.appraiser().true_quote_precision);
        let mut mas: UstrMap<Vec<(PeriodType, Ustr, [f64; 2])>> = UstrMap::default();

        // obtaining historical values
        for cs in states.iter() {
            for sr in cs.moving_average.into_iter().filter_map(|sr| sr) {
                if sr.is_empty() {
                    continue;
                }

                let key = format!("{}-{}-{}", sr.name.as_str(), sr.ma_type.as_str(), timeframe.as_str());

                match mas.entry(ustr(key.as_str())) {
                    Entry::Occupied(mut e) => {
                        e.get_mut().push((sr.period, sr.ma_type, [cs.candle.open_time.timestamp() as f64, sr.value.0]));
                    }
                    Entry::Vacant(mut e) => {
                        e.insert(vec![(sr.period, sr.ma_type, [cs.candle.open_time.timestamp() as f64, sr.value.0])]);
                    }
                }
            }
        }

        // adding values for the current, head candle
        for sr in coin.series.get(timeframe).unwrap().0.moving_average.into_iter().filter_map(|sr| sr) {
            if sr.is_empty() {
                continue;
            }

            let key = format!("{}-{}-{}", sr.name.as_str(), sr.ma_type.as_str(), timeframe.as_str());

            match mas.entry(ustr(key.as_str())) {
                Entry::Occupied(mut e) => {
                    let len = e.get().len();
                    e.get_mut().push((sr.period, sr.ma_type, [head_timestamp as f64, sr.value.0]));
                }
                Entry::Vacant(mut e) => {
                    e.insert(vec![(sr.period, sr.ma_type, [head_timestamp as f64, sr.value.0])]);
                }
            }
        }

        let time_axis_formatter = move |x: f64, _range: &RangeInclusive<f64>| {
            let ts = match NaiveDateTime::from_timestamp_opt(x as i64, 0) {
                None => return String::new(),
                Some(ts) => ts.and_utc(),
            };

            format!(
                "{year:04}.{month:02}.{day:02} {h:02}:{m:02}:{s:02}",
                year = ts.year(),
                month = ts.month(),
                day = ts.day(),
                h = ts.hour(),
                m = ts.minute(),
                s = ts.second(),
            )
        };

        let price_axis_formatter = move |price: f64, _range: &RangeInclusive<f64>| format!("{:.d$}", price, d = quote_precision as usize);

        let coord_fmt = move |p: &PlotPoint, b: &PlotBounds| {
            let ts = match NaiveDateTime::from_timestamp_opt(p.x as i64, 0) {
                None => return String::new(),
                Some(ts) => ts.add(Duration::seconds(100000000000000)).and_utc(),
            };

            format!(
                "{y:.d$}\n{year:04}.{month:02}.{day:02} {h:02}:{m:02}:{s:02}",
                y = round_float_to_precision(p.y, quote_precision),
                year = ts.year(),
                month = ts.month(),
                day = ts.day(),
                h = ts.hour(),
                m = ts.minute(),
                s = ts.second(),
                d = quote_precision as usize
            )
        };

        let mut plot = Plot::new(coin.symbol())
            .allow_boxed_zoom(true)
            .allow_drag(self.app_context.lock().app_config.allow_plot_drag)
            .legend(Legend::default())
            .x_axis_formatter(time_axis_formatter)
            .y_axis_formatter(price_axis_formatter)
            .data_aspect(data_aspect)
            .show_axes([false, true])
            .show_background(true)
            .height(ui.available_height())
            .coordinates_formatter(Corner::RightBottom, CoordinatesFormatter::new(coord_fmt));

        plot.show(ui, |plot_ui| {
            plot_ui.chart_plot(ChartPlot::new(candlesticks).name(coin.symbol()).color(Color32::LIGHT_BLUE));

            for (name, entry) in mas {
                let period = entry.first().unwrap().0;
                let ma_type = entry.first().unwrap().1;
                let ma = entry.into_iter().map(|x| x.2).collect_vec();

                let color = match ma_type.as_str() {
                    "ema" => Color32::from_rgb(255, 255, 255),
                    "rma" => Color32::from_rgb(255, 165, 0),
                    _ => Color32::from_rgb(0, 0, 255),
                };

                plot_ui.line(
                    Line::new(PlotPoints::new(ma)).name(name).color(color), // .stroke(Stroke::new(1.0, Rgba::from(Color32::TEMPORARY_COLOR).to_opaque().multiply(1.0))),
                );
            }

            // ----------------------------------------------------------------------------
            // ----------------------------------------------------------------------------

            // NOTE: Interactive actions require a coin context
            let ccx = match self.coin_context {
                Some(ref coin_context) => coin_context,
                None => {
                    warn!("coin context is not available");
                    return;
                }
            };

            // NOTE: whichever action manages to initialize becomes the active event
            match self.action {
                None => {
                    // Ruler
                    if plot_ui.ctx().input(|i| i.modifiers.shift && i.pointer.primary_down()) {
                        if let Some(start_point) = plot_ui.pointer_coordinate() {
                            let action_context = PlotActionContext::new(None, self.app_context.clone(), ccx.clone(), Some(start_point));

                            let Some(ruler) = RulerAction::new(action_context) else {
                                return;
                            };

                            self.action = Some(Box::new(ruler));
                        }
                    }

                    /*
                    // New limit buy order
                    if plot_ui.ctx().input(|i| i.key_down(Key::B) && i.pointer.primary_down()) {
                        if let Some(start_point) = plot_ui.pointer_coordinate() {
                            let order_config =
                                if plot_ui.ctx().input(|i| i.modifiers.shift) { OrderConfig::limit_buy_then_stop() } else { OrderConfig::limit_buy() };

                            let Some(new_action) = LimitOrderAction::new(Key::B, self.symbol.clone(), order_config, self.market_price.clone(), start_point)
                            else {
                                return Ok(());
                            };

                            self.action = Some(Box::new(new_action));
                        }
                    }
                     */
                }
                Some(ref mut action) => {
                    self.app_context.lock().app_config.allow_plot_drag = false;

                    // NOTE: Escape is used to cancel and clear any action
                    if plot_ui.ctx().input(|i| i.key_released(Key::Escape)) {
                        action.cancel();
                    }

                    match action.compute(plot_ui).unwrap() {
                        None => {
                            match action.state() {
                                PlotActionState::PendingInit => {}

                                PlotActionState::Active => {
                                    action.draw(plot_ui);
                                }

                                PlotActionState::Cancelled => {
                                    info!("action is cancelled");
                                    self.app_context.lock().app_config.allow_plot_drag = true;
                                    self.action = None;
                                }

                                PlotActionState::Finished => {
                                    info!("action is finished");
                                    self.app_context.lock().app_config.allow_plot_drag = true;
                                    self.action = None;
                                }
                            }

                            // return Ok(());
                            return;
                        }

                        Some(PlotActionResult::Trade(trade)) => {
                            // self.orders.write().insert(OsRng.next_u64(), order);

                            println!("{:#?}", ccx.read().config);
                            println!("{:#?}", trade);
                        }

                        _ => {
                            warn!("unrecognized action response");
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
                    .style(LineStyle::Dashed { length: 1.0 })
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
     */
}
