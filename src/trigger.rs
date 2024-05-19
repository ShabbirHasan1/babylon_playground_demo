use crate::{
    trigger_delayed::DelayedTrigger, trigger_delayed_zone::DelayedZoneTrigger, trigger_distance::DistanceTrigger, trigger_proximity::ProximityTrigger,
    types::OrderedValueType,
};
use dyn_clonable::clonable;
use pausable_clock::PausableClock;
use std::{
    fmt::Debug,
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TriggerError {
    #[error("Trigger initialization error: {0}")]
    InitializationError(String),

    #[error("Trigger is already activated")]
    AlreadyActivated,

    #[error("Trigger is not activated")]
    NotActivated,

    #[error("Trigger is negative distance ratio: {0}")]
    NegativeDistanceRatio(OrderedValueType),

    #[error("Trigger has invalid proximity: {0}")]
    InvalidProximity(OrderedValueType),

    #[error("Trigger has invalid target price: {0}")]
    MissingParameter(&'static str),

    #[error("Trigger is not initialized")]
    NotInitialized,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum TriggerKind {
    #[default]
    None,
    Instant,
    Delayed,
    DelayedZone,
    Distance,
    Proximity,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct TriggerMetadata {
    pub kind: TriggerKind,

    // Fields for Delayed
    pub delay: Option<Duration>,

    // Fields for DelayedZone
    pub target_price:           Option<OrderedValueType>,
    pub threshold_price:        Option<OrderedValueType>,
    pub reset_timer_on_reentry: Option<bool>,

    // Fields for Distance
    pub anchor_price:   Option<OrderedValueType>,
    pub distance_ratio: Option<OrderedValueType>,

    // Fields for Proximity
    pub proximity: Option<OrderedValueType>,
}

impl TriggerMetadata {
    pub fn instant() -> Self {
        Self {
            kind: TriggerKind::Instant,
            ..Default::default()
        }
    }

    pub fn delayed(delay: Duration) -> Self {
        Self {
            kind: TriggerKind::Delayed,
            delay: Some(delay),
            ..Default::default()
        }
    }

    pub fn delayed_zone(target_price: OrderedValueType, threshold_price: OrderedValueType, delay: Duration, reset_timer_on_reentry: bool) -> Self {
        Self {
            kind: TriggerKind::DelayedZone,
            target_price: Some(target_price),
            threshold_price: Some(threshold_price),
            delay: Some(delay),
            reset_timer_on_reentry: Some(reset_timer_on_reentry),
            ..Default::default()
        }
    }

    pub fn distance(anchor_price: OrderedValueType, distance_ratio: OrderedValueType) -> Self {
        Self {
            kind: TriggerKind::Distance,
            anchor_price: Some(anchor_price),
            distance_ratio: Some(distance_ratio),
            ..Default::default()
        }
    }

    pub fn proximity(anchor_price: OrderedValueType, target_price: OrderedValueType, proximity: OrderedValueType) -> Self {
        Self {
            kind: TriggerKind::Proximity,
            anchor_price: Some(anchor_price),
            target_price: Some(target_price),
            proximity: Some(proximity),
            ..Default::default()
        }
    }

    pub fn none() -> Self {
        Self {
            kind: TriggerKind::None,
            ..Default::default()
        }
    }

    pub fn is_none(&self) -> bool {
        self.kind == TriggerKind::None
    }

    pub fn is_instant(&self) -> bool {
        self.kind == TriggerKind::Instant
    }

    pub fn is_delayed(&self) -> bool {
        self.kind == TriggerKind::Delayed
    }

    pub fn is_delayed_zone(&self) -> bool {
        self.kind == TriggerKind::DelayedZone
    }

    pub fn is_distance(&self) -> bool {
        self.kind == TriggerKind::Distance
    }

    pub fn is_proximity(&self) -> bool {
        self.kind == TriggerKind::Proximity
    }

    pub fn into_trigger(self) -> Result<Box<dyn Trigger>, TriggerError> {
        match self.kind {
            TriggerKind::None => Ok(Box::new(NoTrigger)),
            TriggerKind::Instant => Ok(Box::new(InstantTrigger)),
            TriggerKind::Delayed => {
                let delay = self.delay.ok_or(TriggerError::MissingParameter("delay"))?;
                Ok(Box::new(DelayedTrigger::new(delay)))
            }
            TriggerKind::DelayedZone => {
                let target_price = self.target_price.ok_or(TriggerError::MissingParameter("target_price"))?;
                let threshold_price = self.threshold_price.ok_or(TriggerError::MissingParameter("threshold_price"))?;
                let delay = self.delay.ok_or(TriggerError::MissingParameter("delay"))?;
                let reset_timer_on_reentry = self.reset_timer_on_reentry.ok_or(TriggerError::MissingParameter("reset_timer_on_reentry"))?;
                Ok(Box::new(DelayedZoneTrigger::new(target_price, threshold_price, delay, reset_timer_on_reentry)?))
            }
            TriggerKind::Distance => {
                let anchor_price = self.anchor_price.ok_or(TriggerError::MissingParameter("anchor_price"))?;
                let distance_ratio = self.distance_ratio.ok_or(TriggerError::MissingParameter("distance_ratio"))?;
                Ok(Box::new(DistanceTrigger::new(anchor_price, distance_ratio)?))
            }
            TriggerKind::Proximity => {
                let anchor_price = self.anchor_price.ok_or(TriggerError::MissingParameter("anchor_price"))?;
                let target_price = self.target_price.ok_or(TriggerError::MissingParameter("target_price"))?;
                let proximity = self.proximity.ok_or(TriggerError::MissingParameter("proximity"))?;
                Ok(Box::new(ProximityTrigger::new(anchor_price, target_price, proximity)?))
            }
        }
    }
}

#[clonable]
pub trait Trigger: Clone + Send + Sync + Debug {
    fn validate(&self) -> Result<(), TriggerError>;
    fn check(&mut self, price: OrderedValueType) -> bool;
    fn reset(&mut self);
    fn metadata(&self) -> TriggerMetadata;
}

#[derive(Debug, Clone)]
pub struct NoTrigger;

impl NoTrigger {
    pub fn new() -> Self {
        Self
    }
}

impl Trigger for NoTrigger {
    fn validate(&self) -> Result<(), TriggerError> {
        Ok(())
    }

    fn check(&mut self, price: OrderedValueType) -> bool {
        false
    }

    fn reset(&mut self) {
        // Do nothing
    }

    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata::none()
    }
}

#[derive(Debug, Clone)]
pub struct InstantTrigger;

impl InstantTrigger {
    pub fn new() -> Self {
        Self
    }
}

impl Trigger for InstantTrigger {
    fn validate(&self) -> Result<(), TriggerError> {
        Ok(())
    }

    fn check(&mut self, _price: OrderedValueType) -> bool {
        true
    }
    fn reset(&mut self) {}

    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata::instant()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger_delayed_zone::DelayedZoneTrigger;
    use ordered_float::OrderedFloat;
    use rand::Rng;
    use std::{thread::sleep, time::Duration};

    #[test]
    fn test_instant_activation() {
        let mut activator = InstantTrigger;
        let start = Instant::now();
        for _ in 0..1000 {
            assert!(activator.check(OrderedValueType::from(1.0)));
        }
        println!("Instant activation duration: {:?}", start.elapsed());
    }
}
