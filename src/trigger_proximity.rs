use crate::{
    trigger::{Trigger, TriggerError, TriggerMetadata},
    types::OrderedValueType,
    util::{calc_relative_position_between, is_outside_inclusive},
};
use ordered_float::OrderedFloat;

#[derive(Debug, Clone)]
pub struct ProximityTrigger {
    previous_price: Option<OrderedValueType>,
    anchor_price:   OrderedValueType, // Typically the entry price
    target_price:   OrderedValueType, // Typically the exit (take profit) price
    proximity:      OrderedValueType, // A ratio between 0.0 and 1.0; 0.10 means when the price is within the remaining 10% of the distance to the target
    shorting:       bool,             // Whether the target price is above or below the anchor price
    triggered:      bool,             // Whether the trigger has been triggered; once triggered, it will not be triggered again until reset
}

impl ProximityTrigger {
    pub fn new(anchor_price: OrderedValueType, target_price: OrderedValueType, proximity: OrderedValueType) -> Result<Self, TriggerError> {
        if is_outside_inclusive(proximity, OrderedFloat(0.0), OrderedFloat(1.0)) {
            return Err(TriggerError::InitializationError("proximity must be between 0.0 and 1.0".to_string()));
        }

        Ok(Self {
            previous_price: None,
            anchor_price,
            target_price,
            proximity,
            shorting: anchor_price > target_price,
            triggered: false,
        })
    }

    pub fn update_target(&mut self, new_target: OrderedValueType) {
        self.target_price = new_target;
        self.shorting = self.anchor_price > self.target_price;
    }

    fn is_near_target(&self, current_price: OrderedValueType) -> bool {
        let Some(previous_price) = self.previous_price else {
            return false;
        };

        let crossed_target = (self.shorting && previous_price < self.target_price && current_price >= self.target_price)
            || (!self.shorting && previous_price > self.target_price && current_price <= self.target_price);

        let within_proximity =
            OrderedFloat((1.0 - calc_relative_position_between(current_price.0, self.anchor_price.0, self.target_price.0)).abs()) <= self.proximity;

        crossed_target || within_proximity
    }
}

impl Trigger for ProximityTrigger {
    fn validate(&self) -> Result<(), TriggerError> {
        if is_outside_inclusive(self.proximity, OrderedFloat(0.0), OrderedFloat(1.0)) {
            return Err(TriggerError::InvalidProximity(self.proximity));
        }

        Ok(())
    }

    fn check(&mut self, price: OrderedValueType) -> bool {
        if self.is_near_target(price) {
            self.triggered = true;
        }

        self.previous_price = Some(price);

        self.triggered
    }

    fn reset(&mut self) {
        self.triggered = false;
    }

    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata::proximity(self.anchor_price, self.target_price, self.proximity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OrderedValueType;
    use ordered_float::OrderedFloat;

    #[test]
    fn test_trigger_is_near_target() {
        let mut trigger = ProximityTrigger::new(OrderedFloat(1.0), OrderedFloat(2.0), OrderedFloat(0.5)).unwrap();

        assert!(trigger.check(OrderedFloat(1.5)));
        trigger.reset();
        assert!(!trigger.check(OrderedFloat(1.4)));
        assert!(trigger.check(OrderedFloat(2.0)));
    }

    #[test]
    fn test_trigger_shorting() {
        let mut trigger = ProximityTrigger::new(OrderedFloat(2.0), OrderedFloat(1.0), OrderedFloat(0.5)).unwrap();

        assert!(trigger.check(OrderedFloat(1.5)));
        trigger.reset();
        assert!(!trigger.check(OrderedFloat(1.6)));
        assert!(trigger.check(OrderedFloat(1.0)));
    }

    #[test]
    fn test_trigger_update_target() {
        let mut trigger = ProximityTrigger::new(OrderedFloat(1.0), OrderedFloat(2.0), OrderedFloat(0.5)).unwrap();

        trigger.update_target(OrderedFloat(3.0));
        assert!(trigger.check(OrderedFloat(2.5)));
        trigger.reset();
        assert!(!trigger.check(OrderedFloat(1.5)));
        assert!(trigger.check(OrderedFloat(3.0)));
    }

    #[test]
    fn test_trigger_proximity_out_of_bounds() {
        let trigger = ProximityTrigger::new(OrderedFloat(1.0), OrderedFloat(2.0), OrderedFloat(1.5));
        assert!(trigger.is_err());
    }
}
