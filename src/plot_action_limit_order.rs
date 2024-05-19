/*
use crate::{
    app::AppConfig,
    order_metadata::OrderMetadata,
    plot_action::{PlotAction, PlotActionContext, PlotActionError, PlotActionResult, PlotActionState},
    stop_metadata::StopMetadata,
    types::{Amount, NormalizedOrderType, NormalizedSide, OrderAction, OrderConfig, OrderStrategy},
    util::calc_x_is_percentage_of_y,
};
use eframe::epaint::Color32;
use egui::{
    mutex::RwLockReadGuard,
    plot::{FilledRange, HLine, Orientation, PlotPoint, PlotUi, Text},
    CursorIcon, Key, TextStyle, WidgetText,
};
use parking_lot::MappedRwLockReadGuard;
use portable_atomic::AtomicF64;
use std::sync::{atomic::Ordering, Arc, Mutex};
use ustr::Ustr;

#[derive(Debug, Clone)]
pub struct LimitOrderAction {
    primary_keybind: Key,
    context:         PlotActionContext,
    market_price:    Arc<AtomicF64>,
    order:           OrderMetadata,
    start_point:     PlotPoint,
    end_point:       PlotPoint,
    stop:            Option<StopMetadata>,
    state:           PlotActionState,
}

impl LimitOrderAction {
    pub fn new(primary_keybind: Key, symbol: Ustr, ui_config: Arc<Mutex<AppConfig>>, market_price: Arc<AtomicF64>, start_point: PlotPoint) -> Option<Self> {
        Some(Self {
            primary_keybind,
            context: PlotActionContext::new(symbol, ui_config.clone()),
            order_config,
            market_price,
            order: OrderMetadata::new(
                NormalizedSide::Buy,
                NormalizedOrderType::Limit,
                order_config.activator,
                Some(start_point.y),
                order_config.quantity.to_value(),
                Some(0.001),
                None,
                None,
            ),
            start_point,
            end_point: start_point,
            stop: None,
            state: PlotActionState::Active,
        })
    }
}

impl PlotAction for LimitOrderAction {
    fn init(&mut self, cx: PlotActionContext) {
        self.context = cx;
    }

    fn compute(&mut self, plot_ui: &mut PlotUi) -> Result<Option<PlotActionResult>, PlotActionError> {
        // if the modifier is released then canceling this action
        if plot_ui.ctx().input(|i| !i.key_down(self.primary_keybind)) {
            self.cancel();
            return Ok(None);
        }

        if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
            self.order.price = Some(self.start_point.y);
            self.end_point = cursor_pos;

            if let Some(stop) = self.stop.as_mut() {
                // *stop = Stop::fixed_limit(self.end_point.y, self.end_point.y * (1.0 - self.order_config.stop.stop_limit_offset_pc))
                *stop = StopMetadata {
                    activator:  self.activator,
                    order_type: Default::default(),
                    price:      Default::default(),
                    limit:      None,
                    quantity:        Default::default(),
                };
            }

            if plot_ui.ctx().input(|i| i.pointer.primary_released()) {
                return match plot_ui.pointer_coordinate() {
                    None => {
                        self.cancel();
                        Ok(None)
                    }
                    Some(_) => {
                        if self.order_config.stop.is_enabled {
                            self.order.strategy = Some(OrderStrategy {
                                on_close: OrderAction::CreateStop(
                                    self.context.symbol,
                                    StopMetadata {
                                        symbol: self.order.symbol.clone(),
                                        price:  self.end_point.y,
                                        limit:  Some(self.end_point.y * (1.0 - self.order_config.stop.stop_limit_offset_pc)),
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        // FIXME: Quantity must be exact and taken from the closed order, instead of `All`
                                        quantity:    self.order.initial_quantity,
                                    },
                                ),
                            });
                        }

                        self.state = PlotActionState::Finished;
                        Ok(Some(PlotActionResult::LimitOrder(self.order.clone())))
                    }
                };
            }
        }

        Ok(None)
    }

    fn draw(&mut self, plot_ui: &mut PlotUi) {
        plot_ui.ctx().set_cursor_icon(CursorIcon::None);

        let Some(entry_price) = self.order.price else { return };

        if plot_ui.ctx().input(|i| i.key_down(self.primary_keybind)) {
            if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
                // highlighting entry price
                plot_ui.hline(HLine::new(self.start_point.y, None).color(Color32::WHITE).width(1.0).highlight(true));

                /*
                match self.stop {
                    Some(Stop::FixedLimit {
                        trigger_price: stop_price,
                        limit,
                    }) => {
                        plot_ui.hline(HLine::new(stop_price, None).color(Color32::DARK_RED).width(1.0).highlight(true));

                        /*
                        plot_ui.filled_range(
                            FilledRange::new(
                                Orientation::Horizontal,
                                true,
                                (false, false),
                                PlotPoint::new(0.0, entry_price),
                                PlotPoint::new(0.0, stop_price.clone()),
                            )
                            .color(Color32::DARK_GRAY)
                            .fill_alpha(0.7),
                        );
                         */

                        plot_ui.filled_range(
                            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, stop_price), PlotPoint::new(0.0, limit))
                                .color(Color32::DARK_RED)
                                .fill_alpha(0.7),
                        );
                    }

                    Some(Stop::FixedMarket { trigger_price: stop_price }) => {}

                    _ => {
                        // NOTE: only fixed limit and market stops are supported for this action
                    }
                }
                 */

                // ----------------------------------------------------------------------------
                // Stop line
                // ----------------------------------------------------------------------------
                if let Some(stop) = self.stop {
                    match stop.order_type {
                        NormalizedOrderType::Limit | NormalizedOrderType::Market => {
                            plot_ui.hline(HLine::new(stop.price, None).color(Color32::DARK_RED).width(1.0).highlight(true));

                            /*
                            plot_ui.filled_range(
                                FilledRange::new(
                                    Orientation::Horizontal,
                                    true,
                                    (false, false),
                                    PlotPoint::new(0.0, entry_price),
                                    PlotPoint::new(0.0, stop_price.clone()),
                                )
                                .color(Color32::DARK_GRAY)
                                .fill_alpha(0.7),
                            );
                             */

                            if let Some(limit) = stop.limit {
                                plot_ui.filled_range(
                                    FilledRange::new(
                                        Orientation::Horizontal,
                                        true,
                                        (false, false),
                                        PlotPoint::new(0.0, stop.price),
                                        PlotPoint::new(0.0, limit),
                                    )
                                    .color(Color32::DARK_RED)
                                    .fill_alpha(0.7),
                                );
                            }
                        }

                        _ => {
                            // IMPORTANT: only fixed limit and market stops are supported for this action
                        }
                    }
                }

                // ----------------------------------------------------------------------------
                // stop middle text

                let delta = (self.start_point.y - self.end_point.y).abs();
                let delta_pc = calc_x_is_percentage_of_y(delta, self.market_price.load(Ordering::SeqCst));

                let text_point = PlotPoint::new((self.start_point.x + cursor_pos.x) / 2.0, (self.end_point.y + cursor_pos.y) / 2.0);
                let text = format!("{:+0.2?} {:+0.2?}%", delta, delta_pc);
                plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Body).strong()).color(Color32::LIGHT_RED));
            }
        }
    }

    fn state(&self) -> PlotActionState {
        self.state
    }

    fn context(&self) -> &PlotActionContext {
        &self.context
    }

    fn cancel(&mut self) {
        self.state = PlotActionState::Cancelled;
    }

    fn finish(&mut self) {
        self.state = PlotActionState::Finished;
    }
}
*/
