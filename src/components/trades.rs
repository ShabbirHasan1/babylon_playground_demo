use crate::{
    app::App,
    appraiser::Appraiser,
    candle::Candle,
    coin::Coin,
    components::util::UiTimeframeConfig,
    plot_action::{PlotActionContext, PlotActionResult, PlotActionState},
    plot_action_ruler::RulerAction,
    support_resistance::SRLevel,
    timeframe::Timeframe,
    timeset::TimeSet,
    util::{calc_ratio_x_to_y, get_hour_f64, get_minute_f64, round_float_to_precision},
};
use chrono::{Datelike, NaiveDateTime, TimeZone, Timelike, Utc};
use egui::{
    plot::{
        Candle as CandleStick, CandleElem, ChartPlot, CoordinatesFormatter, Corner, FilledRange, HLine, Legend, Line, LineStyle, LinkedAxisGroup, Orientation,
        Plot, PlotBounds, PlotPoint, PlotPoints, PlotUi, Text,
    },
    Align, Align2, Color32, CursorIcon, Key, Layout, Rgba, Stroke, TextStyle, Ui, WidgetText,
};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;
use log::{info, warn};
use parking_lot::{MappedMutexGuard, MutexGuard, RwLock, RwLockReadGuard};
use rand::{rngs::OsRng, RngCore};
use std::{collections::hash_map::Entry, ops::RangeInclusive, sync::Arc};
use ustr::{ustr, Ustr, UstrMap};
use yata::core::{PeriodType, ValueType, Window};

impl App {
    pub fn ui_trades(&mut self, ui: &mut Ui, symbol: Ustr) {
        let coin_manager = self.coin_manager.read();
        let coins = coin_manager.coins();

        let coin = match coins.get(&symbol) {
            None => panic!("{}: Coin not found", symbol),
            Some(coin) => RwLockReadGuard::map(coin.read(), |m| m),
        };

        let quote_precision = coin.appraiser().true_quote_precision as usize;
        let base_precision = coin.appraiser().true_base_precision as usize;

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = [0.0, 0.0].into();
            ui.set_height(ui.available_height());
            ui.set_width(ui.available_width());
            ui.push_id("trades", |ui| {
                TableBuilder::new(ui)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .vscroll(true)
                    .columns(Column::auto(), 3)
                    .body(|mut body| {
                        for t in coin.trades.iter() {
                            body.row(18.0, |mut row| {
                                let color = if t.is_buyer_maker { Color32::LIGHT_RED } else { Color32::GREEN };

                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::LEFT), |ui| {
                                        ui.label(WidgetText::from(format!("{:0.p$?}", t.quantity.0, p = base_precision)).color(color));
                                    });
                                });

                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(WidgetText::from(format!("{:0.p$?}", t.price.0, p = quote_precision)).color(color));
                                    });
                                });

                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::RIGHT), |ui| {
                                        ui.label(WidgetText::from(format!("{:0.p$?}", (t.quantity * t.price).0, p = quote_precision)).color(color));
                                    });
                                });
                            });
                        }
                    });
            });
        });
    }
}
