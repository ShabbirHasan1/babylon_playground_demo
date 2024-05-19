use crate::{
    app::{AppContext, AppError},
    coin::CoinContext,
    order::{Order, OrderConfigError},
    trade::{Trade, TradeError},
    trader::Trader,
    trigger::TriggerError,
    types::{NormalizedOrderType, OrderedValueType},
};
use egui::{
    plot::{PlotPoint, PlotUi},
    Key,
};
use parking_lot::{Mutex, RwLock};
use std::{fmt::Debug, sync::Arc};
use thiserror::Error;
use ustr::Ustr;

#[derive(Error, Debug)]
pub enum PlotActionError {
    #[error("Invalid pointer input")]
    InvalidPointerInput,

    #[error("Missing starting point")]
    MissingStartingPoint,

    #[error("Invalid key input, no primary keybind")]
    NoPrimaryKeybind,

    #[error("Invalid point index: {0}")]
    InvalidPointIndex(usize),

    #[error("Invalid input")]
    InvalidInput,

    #[error("No start point")]
    NoStartPoint,

    #[error("Invalid entry price")]
    InvalidEntryPrice,

    #[error("Invalid exit price")]
    InvalidExitPrice,

    #[error("Invalid stop price")]
    InvalidStopPrice,

    #[error("Invalid stop limit price")]
    InvalidStopLimitPrice,

    #[error("Invalid stop order type: {0:?}")]
    InvalidStopOrderType(NormalizedOrderType),

    #[error("Invalid trailing stoploss activation price")]
    InvalidTrailingStoplossActivationPrice,

    #[error("Missing stoploss limit offset ratio")]
    MissingStopLimitOffsetRatio,

    #[error(transparent)]
    AppError(#[from] AppError),

    #[error(transparent)]
    TradeError(#[from] TradeError),

    #[error(transparent)]
    TriggerError(#[from] TriggerError),

    #[error(transparent)]
    OrderConfigError(#[from] OrderConfigError),
}

#[derive(Clone)]
pub struct PlotActionContext {
    /// Application context.
    pub app_context: Arc<Mutex<AppContext>>,

    /// Coin context.
    pub coin_context: Arc<RwLock<CoinContext>>,

    /// Starting point of the action (on the plot).
    pub start_point: Option<PlotPoint>,

    /// Symbol that the action is being performed on.
    pub symbol: Ustr,

    /// Keybind that activated the action.
    pub primary_keybind: Option<Key>,
}

impl PlotActionContext {
    pub fn new(
        primary_keybind: Option<Key>,
        app_context: Arc<Mutex<AppContext>>,
        coin_context: Arc<RwLock<CoinContext>>,
        start_point: Option<PlotPoint>,
    ) -> Self {
        let symbol = coin_context.read().symbol;

        Self {
            app_context,
            coin_context,
            start_point,
            symbol,
            primary_keybind,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PlotActionInputType {
    Entry,
    Exit,
    Stop,
    StopLimit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlotActionInput {
    Entry(Option<PlotPoint>),
    Exit(Option<PlotPoint>),
    Stop(Option<PlotPoint>),
    StopLimit(Option<PlotPoint>),
}

// TODO: use traits for input and output (e.g. PlotActionInput, PlotActionOutput)
// TODO: use traits for state (e.g. PlotActionState)
#[derive(Debug, Clone)]
pub struct PlotActionInputSequence {
    sequence_order: Vec<PlotActionInputType>,
    inputs:         Vec<PlotActionInput>,
    index:          usize,
}

impl PlotActionInputSequence {
    pub fn new(sequence_order: Vec<PlotActionInputType>) -> Self {
        Self {
            index:          0,
            sequence_order: sequence_order.clone(),
            inputs:         sequence_order
                .into_iter()
                .map(|point| match point {
                    PlotActionInputType::Entry => PlotActionInput::Entry(None),
                    PlotActionInputType::Exit => PlotActionInput::Exit(None),
                    PlotActionInputType::Stop => PlotActionInput::Stop(None),
                    PlotActionInputType::StopLimit => PlotActionInput::StopLimit(None),
                })
                .collect(),
        }
    }

    pub fn sequence_order(&self) -> &[PlotActionInputType] {
        self.sequence_order.as_slice()
    }

    pub fn inputs(&self) -> &[PlotActionInput] {
        self.inputs.as_slice()
    }

    pub fn set_next_input(&mut self, point: PlotPoint) -> Result<(), PlotActionError> {
        if self.index > self.inputs.len() {
            return Err(PlotActionError::InvalidPointIndex(self.index));
        }

        if self.is_complete() {
            return Ok(());
        }

        self.inputs[self.index] = match self.sequence_order[self.index] {
            PlotActionInputType::Entry => PlotActionInput::Entry(Some(point)),
            PlotActionInputType::Exit => PlotActionInput::Exit(Some(point)),
            PlotActionInputType::Stop => PlotActionInput::Stop(Some(point)),
            PlotActionInputType::StopLimit => PlotActionInput::StopLimit(Some(point)),
        };

        self.index += 1;

        Ok(())
    }

    pub fn get_next_input_type(&self) -> Option<PlotActionInputType> {
        self.sequence_order.get(self.index).cloned()
    }

    pub fn is_complete(&self) -> bool {
        self.index >= self.inputs.len()
    }

    pub fn get_point(&self, point_type: PlotActionInputType) -> Option<PlotActionInput> {
        self.inputs
            .iter()
            .find(|point| match point_type {
                PlotActionInputType::Entry => matches!(point, PlotActionInput::Entry(_)),
                PlotActionInputType::Exit => matches!(point, PlotActionInput::Exit(_)),
                PlotActionInputType::Stop => matches!(point, PlotActionInput::Stop(_)),
                PlotActionInputType::StopLimit => matches!(point, PlotActionInput::StopLimit(_)),
            })
            .cloned()
    }
}

#[derive(Debug, Copy, Clone, Default, PartialEq)]
pub enum PlotActionState {
    #[default]
    PendingInit,
    Active,
    Cancelled,
    Finished,
}

#[derive(Debug)]
pub enum PlotActionResult {
    Ruler { start_point: PlotPoint, target_point: PlotPoint },
    Order(Order),
    Trade(Trade),
    PriceBias,
}

// TODO: use traits for input and output
pub trait PlotAction {
    fn compute(&mut self, plot_ui: &mut PlotUi) -> Result<Option<PlotActionResult>, PlotActionError>;
    fn draw(&mut self, plot_ui: &mut PlotUi);
    fn state(&self) -> PlotActionState;
    fn context(&self) -> &PlotActionContext;
    fn cancel(&mut self);
    fn finish(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_points() {
        let mut points = PlotActionInputSequence::new(vec![PlotActionInputType::Entry, PlotActionInputType::Exit, PlotActionInputType::Stop]);

        assert_eq!(points.inputs().len(), 3);
        assert_eq!(points.inputs()[0], PlotActionInput::Entry(None));
        assert_eq!(points.inputs()[1], PlotActionInput::Exit(None));
        assert_eq!(points.inputs()[2], PlotActionInput::Stop(None));
        assert!(!points.is_complete());

        assert!(points.set_next_input(PlotPoint::new(1.0, 1.0)).is_ok());
        assert_eq!(points.inputs()[0], PlotActionInput::Entry(Some(PlotPoint::new(1.0, 1.0))));
        assert_eq!(points.inputs()[1], PlotActionInput::Exit(None));
        assert_eq!(points.inputs()[2], PlotActionInput::Stop(None));
        assert!(!points.is_complete());

        assert!(points.set_next_input(PlotPoint::new(2.0, 2.0)).is_ok());
        assert_eq!(points.inputs()[0], PlotActionInput::Entry(Some(PlotPoint::new(1.0, 1.0))));
        assert_eq!(points.inputs()[1], PlotActionInput::Exit(Some(PlotPoint::new(2.0, 2.0))));
        assert_eq!(points.inputs()[2], PlotActionInput::Stop(None));
        assert!(!points.is_complete());

        assert!(points.set_next_input(PlotPoint::new(3.0, 3.0)).is_ok());
        assert_eq!(points.inputs()[0], PlotActionInput::Entry(Some(PlotPoint::new(1.0, 1.0))));
        assert_eq!(points.inputs()[1], PlotActionInput::Exit(Some(PlotPoint::new(2.0, 2.0))));
        assert_eq!(points.inputs()[2], PlotActionInput::Stop(Some(PlotPoint::new(3.0, 3.0))));
        assert!(points.is_complete());

        assert!(points.set_next_input(PlotPoint::new(4.0, 4.0)).is_ok());
        assert_eq!(points.inputs()[0], PlotActionInput::Entry(Some(PlotPoint::new(1.0, 1.0))));
        assert_eq!(points.inputs()[1], PlotActionInput::Exit(Some(PlotPoint::new(2.0, 2.0))));
        assert_eq!(points.inputs()[2], PlotActionInput::Stop(Some(PlotPoint::new(3.0, 3.0))));
        assert!(points.is_complete());

        assert_eq!(points.get_point(PlotActionInputType::Entry), Some(PlotActionInput::Entry(Some(PlotPoint::new(1.0, 1.0)))));
        assert_eq!(points.get_point(PlotActionInputType::Exit), Some(PlotActionInput::Exit(Some(PlotPoint::new(2.0, 2.0)))));
        assert_eq!(points.get_point(PlotActionInputType::Stop), Some(PlotActionInput::Stop(Some(PlotPoint::new(3.0, 3.0)))));
    }
}
