use crate::{
    f,
    instruction::{Instruction, ManagedInstruction},
    timeframe::Timeframe,
    types::OrderedValueType,
};
use dyn_clonable::clonable;
use ordered_float::OrderedFloat;
use std::fmt::{Debug, Formatter};
use thiserror::Error;

#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum CloserError {
    #[error("The closer is not valid")]
    InvalidCloser,

    #[error("The closer is missing the exit price")]
    MissingExitPrice,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum CloserKind {
    NoCloser,
    TakeProfit,
    TrendFollowing,
}

#[derive(Debug, Clone, Copy)]
pub struct CloserMetadata {
    pub kind: CloserKind,

    /// General fields
    pub entry_price: OrderedValueType,

    /// Fields for TakeProfit
    pub is_target_ratio: bool,
    pub target_price:    OrderedValueType,

    /// Fields for TrendFollowing
    pub timeframe: Timeframe,
}

impl Default for CloserMetadata {
    fn default() -> Self {
        Self {
            kind:            CloserKind::NoCloser,
            entry_price:     OrderedValueType::default(),
            is_target_ratio: false,
            target_price:    OrderedValueType::default(),
            timeframe:       Timeframe::NA,
        }
    }
}

impl CloserMetadata {
    pub fn into_closer(self) -> Box<dyn Closer> {
        match self.kind {
            CloserKind::NoCloser => Box::new(NoCloser::default()),
            CloserKind::TakeProfit => Box::new(TakeProfitCloser::new(self.entry_price, self.is_target_ratio, self.target_price)),
            CloserKind::TrendFollowing => Box::new(TrendFollowingCloser::new(self.entry_price, self.timeframe)),
        }
    }

    pub fn no_closer() -> Self {
        Self {
            kind:            CloserKind::NoCloser,
            entry_price:     OrderedValueType::default(),
            is_target_ratio: false,
            target_price:    OrderedValueType::default(),
            timeframe:       Timeframe::default(),
        }
    }

    pub fn take_profit(entry_price: OrderedValueType, is_target_ratio: bool, target: OrderedValueType) -> Self {
        Self {
            kind: CloserKind::TakeProfit,
            entry_price,
            is_target_ratio,
            target_price: target,
            ..Default::default()
        }
    }

    pub fn trend_following(entry_price: OrderedValueType, timeframe: Timeframe) -> Self {
        Self {
            kind: CloserKind::TrendFollowing,
            entry_price,
            timeframe,
            ..Default::default()
        }
    }
}

/// TODO: Add `highest_price_observed` and `lowest_price_observed`
#[clonable]
pub trait Closer: Clone + Debug + Send + Sync {
    fn validate(&self) -> Result<(), CloserError>;
    fn compute(&mut self, current_market_price: OrderedValueType) -> Result<Option<ManagedInstruction>, CloserError>;
    fn should_close(&self) -> bool;
    fn reset(&mut self);
    fn metadata(&self) -> CloserMetadata;
}

// ----------------------------------------------------------------------------
// This is a stub closer that does nothing.
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NoCloser;

impl Default for NoCloser {
    fn default() -> Self {
        NoCloser {}
    }
}

impl Closer for NoCloser {
    fn validate(&self) -> Result<(), CloserError> {
        todo!()
    }

    fn compute(&mut self, _current_price: OrderedValueType) -> Result<Option<ManagedInstruction>, CloserError> {
        Ok(None)
    }

    fn should_close(&self) -> bool {
        false
    }

    fn reset(&mut self) {}

    fn metadata(&self) -> CloserMetadata {
        CloserMetadata::no_closer()
    }
}

// ----------------------------------------------------------------------------
// This closer is used to close a trade when the price reaches a certain level.
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TakeProfitCloser {
    entry_price:       OrderedValueType,
    is_target_ratio:   bool,
    take_profit_price: OrderedValueType,
    close_signal:      bool,
}

impl TakeProfitCloser {
    pub fn new(entry_price: OrderedValueType, is_target_ratio: bool, target: OrderedValueType) -> Self {
        TakeProfitCloser {
            entry_price,
            is_target_ratio,
            take_profit_price: if is_target_ratio { entry_price * (f!(1.0) + target) } else { target },
            close_signal: false,
        }
    }
}

impl Closer for TakeProfitCloser {
    fn validate(&self) -> Result<(), CloserError> {
        todo!()
    }

    fn compute(&mut self, current_price: OrderedValueType) -> Result<Option<ManagedInstruction>, CloserError> {
        if current_price >= self.take_profit_price {
            self.close_signal = true;
        }

        Ok(None)
    }

    fn should_close(&self) -> bool {
        self.close_signal
    }

    fn reset(&mut self) {
        self.close_signal = false;
    }

    fn metadata(&self) -> CloserMetadata {
        CloserMetadata {
            kind: CloserKind::NoCloser,
            entry_price: self.entry_price,
            is_target_ratio: self.is_target_ratio,
            target_price: self.take_profit_price,
            ..Default::default()
        }
    }
}

// ----------------------------------------------------------------------------
// This closer is following the trend of the market and closes the
// trade when the trend is reversing.
// ----------------------------------------------------------------------------

// TODO: Implement this closer
// TODO: Add computed state analysis
// TODO: Add computed state analysis
// TODO: Add computed state analysis
// TODO: Add computed state analysis
// TODO: Add computed state analysis
// TODO: Add computed state analysis
// TODO: Add computed state analysis
#[derive(Debug, Clone, Default)]
pub struct TrendFollowingCloser {
    entry_price:  OrderedValueType,
    timeframe:    Timeframe,
    close_signal: bool,
}

impl TrendFollowingCloser {
    pub fn new(entry_price: OrderedValueType, timeframe: Timeframe) -> Self {
        TrendFollowingCloser {
            entry_price,
            timeframe,
            close_signal: false,
        }
    }
}

impl Closer for TrendFollowingCloser {
    fn validate(&self) -> Result<(), CloserError> {
        todo!()
    }

    fn compute(&mut self, current_price: OrderedValueType) -> Result<Option<ManagedInstruction>, CloserError> {
        Ok(None)
    }

    fn should_close(&self) -> bool {
        false
    }

    fn reset(&mut self) {
        self.close_signal = false;
    }

    fn metadata(&self) -> CloserMetadata {
        CloserMetadata {
            kind:            CloserKind::NoCloser,
            entry_price:     Default::default(),
            is_target_ratio: false,
            target_price:    Default::default(),
            timeframe:       Default::default(),
        }
    }
}
