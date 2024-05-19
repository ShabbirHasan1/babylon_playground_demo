use crate::timeframe::Timeframe;
use egui::Color32;
use yata::core::ValueType;

use crate::util::calc_relative_position_between;

#[derive(Copy, Clone, Debug)]
pub struct UiTimeframeConfig {
    pub timeframe: Timeframe,
}

#[derive(Copy, Clone)]
pub enum OrderBookLevelType {
    Normal,
    Entry(bool),
    Exit(bool),
    Stop(bool),
    StopLimit(bool),
}

pub fn redgreen(value: ValueType, min: ValueType, max: ValueType) -> Color32 {
    let x = calc_relative_position_between(value, min, max).clamp(0.0, 1.0);
    let r = 255.0 * x;
    let g = 255.0 - r;

    Color32::from_rgb(r as u8, g as u8, 0)
}

pub fn redgreen_with_bg(value: ValueType, min: ValueType, max: ValueType, lower_threshold: ValueType, upper_threshold: ValueType) -> (Color32, Color32) {
    let x = calc_relative_position_between(value, min, max).clamp(0.0, 1.0);

    if x <= lower_threshold {
        (Color32::WHITE, Color32::DARK_GREEN)
    } else if x >= upper_threshold {
        (Color32::WHITE, Color32::DARK_RED)
    } else {
        let r = 255.0 * x;
        let g = 255.0 - r;

        (Color32::from_rgb(r as u8, g as u8, 0), Color32::default())
    }
}
