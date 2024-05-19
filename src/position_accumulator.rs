use crate::{f, order::Order, state_analyzer::AnalyzerSummary, types::OrderedValueType};
use ordered_float::OrderedFloat;
use std::fmt::Debug;
use thiserror::Error;

#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum AccumulatorError {
    // TODO: Add specific errors related to accumulation
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd)]
pub enum AccumulatorKind {
    NoAccumulator,
    Incremental,
    VolumeBased,
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct AccumulatorMetadata {
    pub kind: AccumulatorKind,

    // General fields
    pub base_price: OrderedValueType,

    // Fields for Incremental
    pub increment_size: OrderedValueType,

    // Fields for VolumeBased
    pub volume_threshold: OrderedValueType,
}

impl Default for AccumulatorMetadata {
    fn default() -> Self {
        Self {
            kind:             AccumulatorKind::NoAccumulator,
            base_price:       f!(0.0),
            increment_size:   f!(0.0),
            volume_threshold: f!(0.0),
        }
    }
}

impl AccumulatorMetadata {
    pub fn into_accumulator(self) -> Box<dyn Accumulator> {
        match self.kind {
            AccumulatorKind::NoAccumulator => Box::new(NoAccumulator::default()),
            AccumulatorKind::Incremental => {
                unimplemented!("AccumulatorKind::Incremental is not implemented yet");
            }
            AccumulatorKind::VolumeBased => {
                unimplemented!("AccumulatorKind::VolumeBased is not implemented yet");
            }
        }
    }

    // Factory methods for different accumulator kinds
    // Example for an Incremental accumulator
    pub fn incremental(base_price: OrderedValueType, increment_size: OrderedValueType) -> Self {
        Self {
            kind: AccumulatorKind::Incremental,
            base_price,
            increment_size,
            ..Default::default()
        }
    }

    // Add more factory methods for other accumulator types
}

pub trait Accumulator: Debug + Send + Sync {
    fn validate(&self) -> Result<(), AccumulatorError>;
    fn compute(&mut self, market_conditions: &AnalyzerSummary) -> Result<Option<Order>, AccumulatorError>;
    fn should_accumulate(&self) -> bool;
    fn reset(&mut self);
    fn metadata(&self) -> AccumulatorMetadata;
}

// Implementations of different accumulator types

#[derive(Debug, Clone)]
pub struct NoAccumulator;

impl Default for NoAccumulator {
    fn default() -> Self {
        NoAccumulator {}
    }
}

impl Accumulator for NoAccumulator {
    fn validate(&self) -> Result<(), AccumulatorError> {
        todo!()
    }

    fn compute(&mut self, _market_conditions: &AnalyzerSummary) -> Result<Option<Order>, AccumulatorError> {
        Ok(None)
    }

    fn should_accumulate(&self) -> bool {
        false
    }

    fn reset(&mut self) {}

    fn metadata(&self) -> AccumulatorMetadata {
        AccumulatorMetadata::default()
    }
}
