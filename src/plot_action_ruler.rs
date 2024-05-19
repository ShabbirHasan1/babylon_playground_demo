use crate::{
    app::{AppConfig, AppContext},
    coin::CoinContext,
    plot_action::{PlotAction, PlotActionContext, PlotActionError, PlotActionResult, PlotActionState},
    util::calc_ratio_x_to_y,
};
use eframe::epaint::Color32;
use egui::{
    plot::{FilledRange, Orientation, PlotPoint, PlotUi, Text},
    CursorIcon, Key, TextStyle, Ui, WidgetText,
};
use std::sync::{atomic::Ordering, Arc, Mutex};

#[derive(Clone)]
pub struct RulerAction {
    /// The context of this action.
    context: PlotActionContext,

    /// This is the starting point of the action (on the plot).
    start_point: PlotPoint,

    /// The current state of this action.
    state: PlotActionState,
}

impl RulerAction {
    pub fn new(context: PlotActionContext) -> Option<Self> {
        let start_point = match context.start_point {
            None => return None,
            Some(start_point) => start_point,
        };

        Some(Self {
            context,
            start_point,
            state: PlotActionState::Active,
        })
    }
}

impl PlotAction for RulerAction {
    fn compute(&mut self, plot_ui: &mut PlotUi) -> Result<Option<PlotActionResult>, PlotActionError> {
        // if the modifier is released then canceling this action
        if plot_ui.ctx().input(|i| !i.modifiers.shift_only()) {
            self.cancel();
            return Ok(None);
        }

        if plot_ui.ctx().input(|i| i.pointer.primary_released()) {
            return match plot_ui.pointer_coordinate() {
                None => {
                    self.cancel();
                    Ok(None)
                }
                Some(end_point) => {
                    self.state = PlotActionState::Finished;
                    Ok(Some(PlotActionResult::Ruler {
                        start_point:  self.start_point,
                        target_point: end_point,
                    }))
                }
            };
        }

        Ok(None)
    }

    fn draw(&mut self, plot_ui: &mut PlotUi) {
        if let Some(cursor_pos) = plot_ui.pointer_coordinate() {
            let fill_color = if self.start_point.y < cursor_pos.y { Color32::DARK_GREEN } else { Color32::DARK_RED };

            plot_ui.filled_range(
                FilledRange::new(Orientation::Horizontal, false, (true, true), self.start_point, cursor_pos)
                    .color(fill_color)
                    .fill_alpha(0.3),
            );

            let price_delta = cursor_pos.y - self.start_point.y;
            let price_delta_pc = calc_ratio_x_to_y(price_delta, self.start_point.y) * 100.0;

            let text_point = PlotPoint::new((self.start_point.x + cursor_pos.x) / 2.0, (self.start_point.y + cursor_pos.y) / 2.0);
            let text = format!("{:+0.2?} {:+0.2?}%", price_delta, price_delta_pc);

            plot_ui.text(Text::new(text_point, WidgetText::from(text).text_style(TextStyle::Body)).color(Color32::WHITE));

            plot_ui.ctx().output_mut(|o| {
                o.cursor_icon = CursorIcon::None;
            });
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
