use crate::{
    trigger::{Trigger, TriggerError, TriggerMetadata},
    types::OrderedValueType,
};
use ordered_float::OrderedFloat;

#[derive(Debug, Clone)]
pub struct DistanceTrigger {
    anchor_price:   OrderedValueType,
    distance_ratio: OrderedValueType,
    shorting:       bool,
    triggered:      bool,
}

impl DistanceTrigger {
    pub fn new(anchor_price: OrderedValueType, distance_ratio: OrderedValueType) -> Result<Self, TriggerError> {
        if distance_ratio <= OrderedFloat(0.0) {
            return Err(TriggerError::InitializationError("distance_ratio must be greater than 0.0".to_string()));
        }

        Ok(Self {
            anchor_price,
            distance_ratio,
            shorting: false,
            triggered: false,
        })
    }

    fn has_reached_distance(&self, price: OrderedValueType) -> bool {
        let distance = OrderedFloat((price - self.anchor_price).abs());
        let threshold_distance = self.distance_ratio * self.anchor_price;

        distance >= threshold_distance
    }
}

impl Trigger for DistanceTrigger {
    fn validate(&self) -> Result<(), TriggerError> {
        if self.distance_ratio <= OrderedFloat(0.0) {
            return Err(TriggerError::NegativeDistanceRatio(self.distance_ratio));
        }

        Ok(())
    }

    fn check(&mut self, price: OrderedValueType) -> bool {
        if self.has_reached_distance(price) {
            self.triggered = true;
        }

        self.triggered
    }

    fn reset(&mut self) {
        self.triggered = false;
    }

    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata::distance(self.anchor_price, self.distance_ratio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OrderedValueType;
    use ordered_float::OrderedFloat;

    #[test]
    fn test_trigger_has_reached_distance() {
        let mut trigger = DistanceTrigger::new(OrderedFloat(1.0), OrderedFloat(0.5)).unwrap();

        assert!(trigger.check(OrderedFloat(1.6)));
        trigger.reset();
        assert!(!trigger.check(OrderedFloat(1.4)));
        assert!(trigger.check(OrderedFloat(2.0)));
    }

    #[test]
    fn test_trigger_distance_out_of_bounds() {
        let trigger = DistanceTrigger::new(OrderedFloat(1.0), OrderedFloat(0.0));
        assert!(trigger.is_err());
    }
}
