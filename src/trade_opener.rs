use crate::{f, instruction::ManagedInstruction, types::OrderedValueType};
use dyn_clonable::clonable;
use ordered_float::OrderedFloat;
use std::fmt::Debug;
use thiserror::Error;

#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum OpenerError {
    //
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum OpenerKind {
    NoOpener,
    FixedPrice,
    MomentumBased,
    DipBuyer,
}

#[derive(Debug, Clone, Copy)]
pub struct OpenerMetadata {
    pub kind: OpenerKind,

    // General fields
    pub current_market_price: OrderedValueType,

    // Fields for FixedPrice
    pub target_price: OrderedValueType,

    // Fields for MomentumBased
    pub momentum_threshold: OrderedValueType,
}

impl Default for OpenerMetadata {
    fn default() -> Self {
        Self {
            kind:                 OpenerKind::NoOpener,
            current_market_price: f!(0.0),
            target_price:         f!(0.0),
            momentum_threshold:   f!(0.0),
        }
    }
}

impl OpenerMetadata {
    pub fn into_opener(self) -> Box<dyn Opener> {
        match self.kind {
            OpenerKind::NoOpener => Box::new(NoOpener::default()),
            _ => todo!(),
        }
    }

    pub fn fixed_price(target: OrderedValueType) -> Self {
        Self {
            kind: OpenerKind::FixedPrice,
            target_price: target,
            ..Default::default()
        }
    }
}

#[clonable]
pub trait Opener: Clone + Debug + Send + Sync {
    fn validate(&self) -> Result<(), OpenerError>;
    fn compute(&mut self, current_market_price: OrderedValueType) -> Result<Option<ManagedInstruction>, OpenerError>;
    fn should_open(&self) -> bool;
    fn reset(&mut self);
    fn metadata(&self) -> OpenerMetadata;
}

#[derive(Debug, Clone)]
pub struct NoOpener;

impl Default for NoOpener {
    fn default() -> Self {
        NoOpener {}
    }
}

impl Opener for NoOpener {
    fn validate(&self) -> Result<(), OpenerError> {
        todo!()
    }

    fn compute(&mut self, _current_price: OrderedValueType) -> Result<Option<ManagedInstruction>, OpenerError> {
        Ok(None)
    }

    fn should_open(&self) -> bool {
        false
    }

    fn reset(&mut self) {}

    fn metadata(&self) -> OpenerMetadata {
        OpenerMetadata::default()
    }
}
