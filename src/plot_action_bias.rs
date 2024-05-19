use crate::{
    app::{AppConfig, AppContext},
    coin::CoinContext,
    f,
    plot_action::{PlotAction, PlotActionContext, PlotActionError, PlotActionResult, PlotActionState},
    simulator::Simulator,
    simulator_generator::GeneratorConfigBias,
    util::calc_ratio_x_to_y,
};
use eframe::epaint::Color32;
use egui::{
    plot::{FilledRange, Orientation, PlotPoint, PlotUi, Text},
    CursorIcon, Key, TextStyle, Ui, WidgetText,
};
use log::info;
use ordered_float::OrderedFloat;
use std::sync::{atomic::Ordering, Arc, Mutex};

#[derive(Clone)]
pub struct PriceBiasAction {
    /// The context of this action.
    context: PlotActionContext,

    /// The simulator that is being used to generate the plot.
    simulator: Simulator,

    /// The current state of this action.
    state: PlotActionState,
}

impl PriceBiasAction {
    pub fn new(context: PlotActionContext, simulator: Simulator) -> Result<Self, PlotActionError> {
        let target_point = match context.start_point {
            None => return Err(PlotActionError::MissingStartingPoint),
            Some(target_point) => target_point,
        };

        Ok(Self {
            context,
            state: PlotActionState::Active,
            simulator,
        })
    }
}

impl PlotAction for PriceBiasAction {
    fn compute(&mut self, plot_ui: &mut PlotUi) -> Result<Option<PlotActionResult>, PlotActionError> {
        let Some(primary_keybind) = self.context.primary_keybind else {
            return Ok(None);
        };

        if !plot_ui.ctx().input(|i| i.key_down(primary_keybind)) {}

        if plot_ui.ctx().input(|i| i.pointer.primary_down()) {
            match plot_ui.pointer_coordinate() {
                None => {}
                Some(target_point) => {
                    self.simulator.generator_mut().config.bias = Some(GeneratorConfigBias {
                        // factor:       0.0005,
                        // max_factor:   0.001,
                        factor:       0.0005,
                        max_factor:   0.001,
                        target_price: target_point.y,
                    });
                }
            };

            Ok(None)
        } else {
            self.finish();
            return Ok(Some(PlotActionResult::PriceBias));
        }
    }

    fn draw(&mut self, plot_ui: &mut PlotUi) {}

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
