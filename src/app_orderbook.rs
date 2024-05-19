use crate::{
    app::App,
    appraiser::Appraiser,
    components::util::OrderBookLevelType,
    egui_fill_rect,
    orderbook::{OrderBook, SIZED_ORDERBOOK_LENGTH},
    types::{NormalizedSide, OrderedValueType},
};
use egui::{Align, Color32, Layout, Ui, WidgetText};
use egui_extras::{Column, TableBody, TableBuilder};
use itertools::Itertools;
use ordered_float::OrderedFloat;
use ustr::Ustr;

impl App {
    pub fn ui_orderbook(&self, ui: &mut Ui, symbol: Ustr) {
        let coin_manager = self.coin_manager.read();
        let coin_map = coin_manager.coins();

        let coin = match coin_map.get(&symbol) {
            None => {
                return;
            }
            Some(coin) => coin.read(),
        };

        let ob = coin.orderbook.clone();
        let ob = ob.0.read();

        // let ob_height = ui.available_height();
        let ob_height = 760.0;

        let suggested_roundtrips = match ob.suggested_long_roundtrip_quote(OrderedFloat(200.0), OrderedFloat(2.5), OrderedFloat(0.70), OrderedFloat(0.70)) {
            None => vec![],
            Some(roundtrip) => vec![roundtrip],
        };

        let exits = suggested_roundtrips.iter().map(|r| r.exit).collect_vec();
        let entries = suggested_roundtrips.iter().map(|r| r.entry).collect_vec();
        let stops = suggested_roundtrips.iter().map(|r| r.stop).collect_vec();

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = [0.0, 0.0].into();
            ui.set_height(ob_height / 2.0);
            ui.push_id("asks", |ui| {
                TableBuilder::new(ui)
                    .stick_to_bottom(true)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .vscroll(true)
                    .columns(Column::auto(), 3)
                    .body(|mut body| {
                        for (i, ask) in ob.asks().into_iter().rev().enumerate() {
                            let mapped_index = SIZED_ORDERBOOK_LENGTH - i;
                            let suggested_exit = exits.iter().find(|x| x.anchor_position == mapped_index);

                            let is_better_price_used = if let Some(best_fit) = suggested_exit {
                                best_fit.anchor_position == mapped_index && best_fit.compute(&ob.appraiser).2
                            } else {
                                false
                            };

                            body.row(18.0, |mut row| {
                                row.col(|ui| {
                                    if let Some(best_fit) = suggested_exit {
                                        if best_fit.anchor_position == mapped_index && !is_better_price_used {
                                            egui_fill_rect!(ui, 0.0, Color32::DARK_RED);
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.label(WidgetText::from(format!("{:0.p$?}", ask.price.0, p = ob.quote_precision)).color(Color32::WHITE));
                                            });
                                        } else {
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.label(WidgetText::from(format!("{:0.p$?}", ask.price.0, p = ob.quote_precision)).color(Color32::LIGHT_RED));
                                            });
                                        }
                                    } else {
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            ui.label(WidgetText::from(format!("{:0.p$?}", ask.price.0, p = ob.quote_precision)).color(Color32::LIGHT_RED));
                                        });
                                    }
                                });
                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(WidgetText::from(format!("{:0.p$?}", ask.quantity.0, p = ob.base_precision)).color(Color32::LIGHT_RED));
                                    });
                                });
                                row.col(|ui| {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.add_space(5.0);
                                        ui.label(
                                            WidgetText::from(format!("{:0.p$?}", (ask.price * ask.quantity).0, p = ob.quote_precision))
                                                .color(Color32::LIGHT_RED),
                                        );
                                    });
                                });
                            });

                            if let Some(best_fit) = suggested_exit {
                                if best_fit.anchor_position == mapped_index && is_better_price_used {
                                    body.row(18.0, |mut row| {
                                        row.col(|ui| {
                                            egui_fill_rect!(ui, 0.0, Color32::WHITE);
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.label(WidgetText::from(format!("{:0.p$?}", ask.price.0, p = ob.quote_precision)).color(Color32::DARK_RED));
                                            });
                                        });
                                        row.col(|ui| {
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.label(
                                                    WidgetText::from(format!("{:0.p$?}", ask.quantity.0, p = ob.base_precision)).color(Color32::LIGHT_RED),
                                                );
                                            });
                                        });
                                        row.col(|ui| {
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.add_space(5.0);
                                                ui.label(
                                                    WidgetText::from(format!("{:0.p$?}", (ask.price * ask.quantity).0, p = ob.quote_precision))
                                                        .color(Color32::LIGHT_RED),
                                                );
                                            });
                                        });
                                    });
                                }
                            }
                        }
                    });
            });
        });

        ui.horizontal_top(|ui| {
            ui.set_height(ob_height / 2.0);
            ui.push_id("bids", |ui| {
                ui.spacing_mut().item_spacing = [0.0, 0.0].into();
                TableBuilder::new(ui)
                    .cell_layout(Layout::left_to_right(Align::Center))
                    .vscroll(true)
                    .columns(Column::auto(), 3)
                    .body(|mut body| {
                        for (mapped_index, lvl) in ob.bids().into_iter().enumerate() {
                            let suggested_entry = entries.iter().find(|x| x.anchor_position == mapped_index);
                            let suggested_stop = stops.iter().find(|x| x.anchor_position == mapped_index);
                            let mut is_normal_price_shown = false;

                            // if no suggestions then showing regular level
                            if suggested_entry.is_none() || suggested_stop.is_none() {
                                self.orderbook_level(
                                    &mut body,
                                    NormalizedSide::Buy,
                                    &ob.appraiser,
                                    (ob.quote_precision, ob.base_precision),
                                    18.0,
                                    mapped_index,
                                    OrderBookLevelType::Normal,
                                    lvl.price,
                                    lvl.quantity,
                                    lvl.price * lvl.quantity,
                                );

                                continue;
                            }

                            let (suggedted_entry, suggested_entry_price, expected_entry_return, has_better_entry_price) = {
                                let entry = suggested_entry.unwrap();
                                let (price, expected_return, has_better_price) = entry.compute(&ob.appraiser);
                                (entry, price, expected_return, has_better_price)
                            };

                            /*
                            let (suggested_stop, suggested_stop_price, expected_stop_return, has_better_stop_price) = {
                                let stop = suggested_stop.unwrap();
                                let (price, expected_return, has_better_price) = stop.compute(&ob.appraiser);
                                (stop, price, expected_return, has_better_price)
                            };
                             */

                            if has_better_entry_price {
                                self.orderbook_level(
                                    &mut body,
                                    NormalizedSide::Buy,
                                    &ob.appraiser,
                                    (ob.quote_precision, ob.base_precision),
                                    18.0,
                                    mapped_index,
                                    OrderBookLevelType::Entry(true),
                                    suggested_entry_price,
                                    expected_entry_return,
                                    suggested_entry_price * expected_entry_return,
                                );

                                if !is_normal_price_shown {
                                    is_normal_price_shown = true;
                                    self.orderbook_level(
                                        &mut body,
                                        NormalizedSide::Buy,
                                        &ob.appraiser,
                                        (ob.quote_precision, ob.base_precision),
                                        18.0,
                                        mapped_index,
                                        OrderBookLevelType::Normal,
                                        lvl.price,
                                        lvl.quantity,
                                        lvl.price * lvl.quantity,
                                    );
                                }
                            } else if !is_normal_price_shown {
                                is_normal_price_shown = true;
                                self.orderbook_level(
                                    &mut body,
                                    NormalizedSide::Buy,
                                    &ob.appraiser,
                                    (ob.quote_precision, ob.base_precision),
                                    18.0,
                                    mapped_index,
                                    OrderBookLevelType::Entry(false),
                                    suggested_entry_price,
                                    lvl.quantity + expected_entry_return,
                                    suggested_entry_price * (lvl.quantity + expected_entry_return),
                                );
                            }

                            /*
                            if has_better_stop_price {
                                if !is_normal_price_shown {
                                    is_normal_price_shown = true;
                                    self.orderbook_level(
                                        &mut body,
                                        Side::Buy,
                                        &ob.appraiser,
                                        (ob.quote_precision, ob.base_precision),
                                        18.0,
                                        mapped_index,
                                        OrderBookLevelType::Normal,
                                        lvl.price,
                                        lvl.quantity,
                                        lvl.price * lvl.quantity,
                                    );
                                }

                                self.orderbook_level(
                                    &mut body,
                                    Side::Buy,
                                    &ob.appraiser,
                                    (ob.quote_precision, ob.base_precision),
                                    18.0,
                                    mapped_index,
                                    OrderBookLevelType::Stop(true),
                                    suggested_stop_price,
                                    expected_stop_return / suggested_stop_price,
                                    suggested_stop_price * expected_stop_return,
                                );
                            } else if !is_normal_price_shown {
                                is_normal_price_shown = true;
                                self.orderbook_level(
                                    &mut body,
                                    Side::Buy,
                                    &ob.appraiser,
                                    (ob.quote_precision, ob.base_precision),
                                    18.0,
                                    mapped_index,
                                    OrderBookLevelType::Stop(false),
                                    suggested_stop_price,
                                    lvl.quantity + (expected_stop_return / suggested_stop_price),
                                    suggested_stop_price * (lvl.quantity + expected_stop_return),
                                );
                            }
                             */
                        }
                    });
            });
        });
    }

    fn orderbook_level(
        &self,
        table_body: &mut TableBody<'_>,
        side: NormalizedSide,
        appraiser: &Appraiser,
        precision: (usize, usize),
        height: f32,
        mapped_index: usize,
        lvl_type: OrderBookLevelType,
        price: OrderedValueType,
        quote_or_quantity: OrderedValueType,
        total_amount: OrderedValueType,
    ) {
        use NormalizedSide::{Buy, Sell};
        use OrderBookLevelType::{Entry, Exit, Normal, Stop, StopLimit};

        let (fg, bg) = match (side, lvl_type) {
            (Buy, Normal) => (Color32::default(), Color32::LIGHT_GREEN),
            (Buy, Entry(has_better_price)) =>
                if has_better_price {
                    (Color32::DARK_GREEN, Color32::WHITE)
                } else {
                    (Color32::DARK_GREEN, Color32::BLACK)
                },
            (Buy, Stop(has_better_price)) =>
                if has_better_price {
                    (Color32::DARK_RED, Color32::WHITE)
                } else {
                    (Color32::DARK_RED, Color32::BLACK)
                },
            (Sell, Normal) => (Color32::default(), Color32::LIGHT_RED),
            (Sell, Exit(has_better_price)) =>
                if has_better_price {
                    (Color32::BROWN, Color32::WHITE)
                } else {
                    (Color32::RED, Color32::BLACK)
                },
            _ => return,
        };

        table_body.row(height, |mut row| {
            row.col(|ui| {
                egui_fill_rect!(ui, 0.0, fg);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(WidgetText::from(format!("{:0.p$?}", price.0, p = precision.0)).color(bg));
                });
            });
            row.col(|ui| {
                egui_fill_rect!(ui, 0.0, fg);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(WidgetText::from(format!("{:0.p$?}", quote_or_quantity.0, p = precision.1)).color(bg));
                });
            });
            row.col(|ui| {
                egui_fill_rect!(ui, 0.0, fg);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(5.0);
                    ui.label(WidgetText::from(format!("{:0.p$?}", total_amount.0, p = precision.0)).color(bg));
                });
            });
        });
    }
}
