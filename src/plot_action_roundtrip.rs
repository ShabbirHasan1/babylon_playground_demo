use crate::{
    app::AppConfig,
    balance::BalanceError,
    coin::CoinContext,
    plot_action::{PlotAction, PlotActionContext, PlotActionError, PlotActionResult, PlotActionState},
    trigger::InstantTrigger,
    types::{Amount, NormalizedOrderType, NormalizedSide, OrderedValueType, Quantity},
    util::calc_ratio_x_to_y,
};
use eframe::epaint::Color32;
use egui::{
    plot::{FilledRange, HLine, Orientation, PlotPoint, PlotUi, Text},
    CursorIcon, Key, TextStyle, WidgetText,
};
use log::{error, warn};
use ordered_float::OrderedFloat;
use parking_lot::{lock_api::MutexGuard, MappedRwLockReadGuard, RawMutex, RwLockReadGuard};
use portable_atomic::AtomicF64;
use std::{
    sync::{atomic::Ordering, Arc, Mutex},
    time::Duration,
};
use ustr::Ustr;
use yata::core::ValueType;

/*
// TODO: add ability to change by resizing and moving after creation
// TODO: add support for trailing stop but more importantly is to display where it turns on and incremental steps
#[derive(Debug, Clone)]
pub struct RoundtripAction {
    /// The context of this action.
    context: PlotActionContext,

    /// The configuration of this roundtrip.
    config: RoundtripConfig,

    /// The metadata of this roundtrip. This is what is returned when the action is finished.
    /// This is also what is used to display the roundtrip on the plot.
    roundtrip: RoundtripMetadata,

    /// This is the starting point of the action (on the plot).
    start_point: PlotPoint,

    /// The current state of this action.
    state: PlotActionState,
}

// TODO: add ability to change by resizing and moving after creation
// TODO: add support for trailing stop but more importantly is to display where it turns on and incremental steps
impl RoundtripAction {
    pub fn new(context: PlotActionContext, roundtrip_config: RoundtripConfig) -> Option<Self> {
        // Entry price is Y value of the starting point (on the plot)
        let (entry_price, start_point) = match context.start_point {
            None => return None,
            Some(start_point) => (start_point.y, start_point),
        };

        // ----------------------------------------------------------------------------
        // Initializing the entry order metadata
        // ----------------------------------------------------------------------------

        let entry = OrderMetadata::new(
            context.coin_context.clone(),
            NormalizedSide::Buy,
            NormalizedOrderType::Limit,
            Box::new(InstantTrigger::new()),
            Some(entry_price),
            Some(roundtrip_config.entry.initial_quantity),
            Some(0.001),
            StopMetadata::default(),
            2,
            Some(Duration::from_millis(300)),
            false,
            None,
        );

        // ----------------------------------------------------------------------------
        // Initializing the exit order metadata
        // ----------------------------------------------------------------------------

        let exit = OrderMetadata::new(
            context.coin_context.clone(),
            NormalizedSide::Sell,
            NormalizedOrderType::Limit,
            Box::new(InstantTrigger::new()),
            None,
            None,
            Some(0.001),
            StopMetadata::default(),
            2,
            Some(Duration::from_millis(300)),
            roundtrip_config.exit_use_oco,
            None,
        );

        // ----------------------------------------------------------------------------
        // Initializing the Roundrip Action itself
        // ----------------------------------------------------------------------------

        let mut action = Self {
            context,
            config: Default::default(),
            roundtrip: RoundtripMetadata {
                tag: None,
                entry,
                exit,
                exit_ratio: 1.0,
                has_been_moved_or_replaced: false,
            },
            start_point,
            state: PlotActionState::Active,
        };

        // NOTE: Updating the entry quantity to set the initial quantity
        action.update_entry_quantity();

        Some(action)
    }
}

impl PlotAction for RoundtripAction {
    fn compute(&mut self, plot_ui: &mut PlotUi) -> Result<Option<PlotActionResult>, PlotActionError> {
        // if the modifier is released then canceling this action
        if plot_ui.ctx().input(|i| !i.key_down(Key::R)) {
            self.cancel();
            return Ok(None);
        }

        if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
            self.roundtrip.entry.price = Some(self.start_point.y);
            self.roundtrip.exit.price = Some(cursor_pos.y);

            if plot_ui.ctx().input(|i| i.pointer.primary_released()) {
                return match plot_ui.pointer_coordinate() {
                    None => {
                        self.cancel();
                        Ok(None)
                    }
                    Some(_) => {
                        self.state = PlotActionState::Finished;
                        Ok(Some(PlotActionResult::Roundtrip(self.roundtrip.clone())))
                    }
                };
            }
        }

        Ok(None)
    }

    fn draw(&mut self, plot_ui: &mut PlotUi) {
        plot_ui.ctx().set_cursor_icon(CursorIcon::None);

        let Some(entry_price) = self.roundtrip.entry.price else { return };
        let Some(exit_price) = self.roundtrip.exit.price else { return };
        let Some(net_price) = self.roundtrip.net_price() else { return };
        let Some(breakeven_price) = self.roundtrip.breakeven_price() else { return };

        if plot_ui.ctx().input(|i| i.key_down(Key::R)) {
            if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
                // highlighting entry price
                plot_ui.hline(HLine::new(self.start_point.y, None).color(Color32::WHITE).width(1.0).highlight(true));

                if self.roundtrip.exit.fee.is_some() {
                    plot_ui.filled_range(
                        FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, net_price), PlotPoint::new(0.0, exit_price))
                            .color(Color32::DARK_GRAY)
                            .fill_alpha(0.3),
                    );

                    if self.roundtrip.entry.fee.is_some() {
                        plot_ui.filled_range(
                            FilledRange::new(
                                Orientation::Horizontal,
                                true,
                                (false, false),
                                PlotPoint::new(0.0, breakeven_price),
                                PlotPoint::new(0.0, net_price),
                            )
                            .color(Color32::DARK_GREEN)
                            .fill_alpha(0.5),
                        );
                    } else {
                        plot_ui.filled_range(
                            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, entry_price), PlotPoint::new(0.0, net_price))
                                .color(Color32::DARK_GREEN)
                                .fill_alpha(0.5),
                        );
                    }

                    // ----------------------------------------------------------------------------
                    // exit fee

                    let price_delta = cursor_pos.y - net_price;
                    let price_delta_pc = calc_x_is_percentage_of_y(price_delta, self.market_price.load(Ordering::SeqCst));

                    let text_point = PlotPoint::new((self.start_point.x + cursor_pos.x) / 2.0, (net_price + cursor_pos.y) / 2.0);
                    let text = format!("-{:0.2?} -{:0.2?}%", price_delta, price_delta_pc);
                    plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Body).strong()).color(Color32::WHITE));
                } else {
                    if self.roundtrip.entry.fee.is_some() {
                        plot_ui.filled_range(
                            FilledRange::new(
                                Orientation::Horizontal,
                                true,
                                (false, false),
                                PlotPoint::new(0.0, breakeven_price),
                                PlotPoint::new(0.0, exit_price),
                            )
                            .color(Color32::DARK_GREEN)
                            .fill_alpha(0.5),
                        );
                    } else {
                        plot_ui.filled_range(
                            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, entry_price), PlotPoint::new(0.0, exit_price))
                                .color(Color32::DARK_GREEN)
                                .fill_alpha(0.5),
                        );
                    }
                }

                if self.roundtrip.entry.fee.is_some() {
                    plot_ui.filled_range(
                        FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, entry_price), PlotPoint::new(0.0, breakeven_price))
                            .color(Color32::DARK_GRAY)
                            .fill_alpha(0.3),
                    );

                    // ----------------------------------------------------------------------------
                    // entry fee

                    let price_delta = breakeven_price - self.start_point.y;
                    let price_delta_pc = calc_x_is_percentage_of_y(price_delta, self.market_price.load(Ordering::SeqCst));

                    let text_point = PlotPoint::new((self.start_point.x + cursor_pos.x) / 2.0, (breakeven_price + self.start_point.y) / 2.0);
                    let text = format!("-{:0.2?} -{:0.2?}%", price_delta, price_delta_pc);
                    plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Body).strong()).color(Color32::WHITE));
                }

                /*
                plot_ui.filled_range(
                    FilledRange::new(
                        Orientation::Horizontal,
                        true,
                        (false, false),
                        PlotPoint::new(0.0, entry_price),
                        PlotPoint::new(0.0, self.order.stop_price),
                    )
                    .color(Color32::DARK_RED)
                    .fill_alpha(0.7),
                );
                 */

                match self.roundtrip.stop_price_and_limit() {
                    Ok((stop_price, None)) => {
                        plot_ui.hline(HLine::new(stop_price, None).color(Color32::DARK_RED).width(1.0).highlight(true));
                    }

                    Ok((stop_price, Some(limit))) => {
                        plot_ui.filled_range(
                            FilledRange::new(Orientation::Horizontal, true, (false, false), PlotPoint::new(0.0, stop_price), PlotPoint::new(0.0, limit))
                                .color(Color32::DARK_RED)
                                .fill_alpha(0.7),
                        );
                    }

                    _ => {}
                }

                // highlighting exit price
                plot_ui.hline(HLine::new(exit_price, None).color(Color32::GOLD).width(1.0).highlight(true));

                // ----------------------------------------------------------------------------
                // take profit middle text

                let price_delta = cursor_pos.y - self.start_point.y;
                let price_delta_pc = calc_x_is_percentage_of_y(price_delta, self.market_price.load(Ordering::SeqCst));

                let text_point = PlotPoint::new((self.start_point.x + cursor_pos.x) / 2.0, (self.start_point.y + cursor_pos.y) / 2.0);
                let text = format!("{:+0.2?} {:+0.2?}%", price_delta, price_delta_pc);
                plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Body).strong()).color(Color32::WHITE));
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
