use crate::{
    trigger::{Trigger, TriggerError, TriggerMetadata},
    types::OrderedValueType,
};
use dyn_clonable::clonable;
use pausable_clock::PausableClock;
use std::time::Duration;

#[derive(Debug)]
pub struct DelayedZoneTrigger {
    target_price:           OrderedValueType,
    threshold_price:        OrderedValueType,
    delay:                  Duration,
    reset_timer_on_reentry: bool,
    clock:                  Option<PausableClock>,
    triggered:              bool,
}

impl Clone for DelayedZoneTrigger {
    fn clone(&self) -> Self {
        Self {
            target_price:           self.target_price,
            threshold_price:        self.threshold_price,
            delay:                  self.delay,
            reset_timer_on_reentry: self.reset_timer_on_reentry,
            clock:                  None,
            triggered:              false,
        }
    }
}

impl DelayedZoneTrigger {
    pub fn new(target_price: OrderedValueType, threshold_price: OrderedValueType, delay: Duration, reset_timer_on_reentry: bool) -> Result<Self, TriggerError> {
        if target_price == threshold_price {
            return Err(TriggerError::InitializationError("target_price and threshold_price must be different".to_string()));
        }

        Ok(Self {
            target_price,
            threshold_price,
            delay,
            reset_timer_on_reentry,
            clock: None,
            triggered: false,
        })
    }

    fn is_in_critical_zone(&self, price: OrderedValueType) -> bool {
        let shorting = self.target_price < self.threshold_price;
        (!shorting && price <= self.threshold_price) || (shorting && price >= self.threshold_price)
    }

    fn is_in_normal_zone(&self, price: OrderedValueType) -> bool {
        let shorting = self.target_price < self.threshold_price;
        (!shorting && price > self.target_price) || (shorting && price < self.target_price)
    }

    fn start_clock(&mut self) {
        self.clock = Some(PausableClock::default());
    }

    fn reset_or_pause_clock(&mut self) {
        if self.reset_timer_on_reentry {
            self.clock = None;
        } else {
            if let Some(clock) = self.clock.as_mut() {
                clock.pause();
            }
        }
    }

    fn check_time_elapsed(&self) -> bool {
        if let Some(clock) = self.clock.as_ref() {
            return clock.now().elapsed_millis() > self.delay.as_millis() as u64;
        }

        false
    }
}

impl Trigger for DelayedZoneTrigger {
    fn validate(&self) -> Result<(), TriggerError> {
        Ok(())
    }

    fn check(&mut self, price: OrderedValueType) -> bool {
        // If the price is in the critical zone, trigger immediately
        if self.is_in_critical_zone(price) {
            self.triggered = true;
            return true;
        }

        // If the price is in the delayed zone but not in the critical zone,
        // start the clock if it's not running
        if !self.is_in_normal_zone(price) && self.clock.is_none() {
            self.start_clock();
        }

        // If the price is in the normal zone and the clock is running,
        // reset or pause the clock
        if self.is_in_normal_zone(price) && self.clock.is_some() {
            self.reset_or_pause_clock();
        }

        // If the clock is running and the time elapsed is more than the TTL,
        // trigger
        if self.clock.is_some() && self.check_time_elapsed() {
            self.triggered = true;
            return true;
        }

        false
    }

    fn reset(&mut self) {
        self.clock = None;
        self.triggered = false;
    }

    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata::delayed_zone(self.target_price, self.threshold_price, self.delay, self.reset_timer_on_reentry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ordered_float::OrderedFloat;
    use std::{thread, time::Duration};

    #[test]
    fn test_delayed_zone_initialization() {
        let _trigger = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(15.0), Duration::from_secs(5), false);
        assert!(_trigger.is_ok());
    }

    #[test]
    fn test_delayed_zone_initialization_error() {
        let trigger_result = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(10.0), Duration::from_secs(5), false);
        assert!(trigger_result.is_err());

        if let Err(e) = trigger_result {
            assert_eq!(e, TriggerError::InitializationError("target_price and threshold_price must be different".to_string()));
        }
    }

    #[test]
    fn test_delayed_zone_check_normal_zone() {
        let mut trigger = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(15.0), Duration::from_secs(5), false).unwrap();
        assert_eq!(trigger.check(OrderedFloat(9.0)), false);
        assert!(trigger.clock.is_none()); // the clock should not start in the normal zone
    }

    #[test]
    fn test_delayed_zone_check_critical_zone() {
        let mut trigger = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(15.0), Duration::from_secs(5), false).unwrap();
        assert_eq!(trigger.check(OrderedFloat(16.0)), true); // it should trigger immediately in the critical zone
    }

    #[test]
    fn test_delayed_zone_check_time_elapsed() {
        let mut trigger = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(20.0), Duration::from_secs(1), false).unwrap();

        // The price is in the delayed zone, so the clock should start
        trigger.check(OrderedFloat(15.0));
        assert!(trigger.clock.is_some());

        // Sleep for the duration of the TTL plus a small delay to ensure the time has elapsed
        std::thread::sleep(Duration::from_secs(1) + Duration::from_millis(100));

        assert_eq!(trigger.check(OrderedFloat(15.0)), true); // it should trigger after the time has elapsed
    }

    #[test]
    fn test_delayed_zone_reset() {
        let mut trigger = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(15.0), Duration::from_secs(5), false).unwrap();
        trigger.check(OrderedFloat(16.0));
        trigger.reset();
        assert_eq!(trigger.check(OrderedFloat(16.0)), true); // it should still trigger in the critical zone after reset
    }

    #[test]
    fn test_delayed_zone_reentry() {
        let mut trigger = DelayedZoneTrigger::new(OrderedFloat(10.0), OrderedFloat(15.0), Duration::from_millis(200), true).unwrap();
        assert_eq!(trigger.check(OrderedFloat(16.0)), true); // it should trigger immediately in the critical zone
        thread::sleep(Duration::from_millis(150));
        assert_eq!(trigger.check(OrderedFloat(9.0)), false); // it should not trigger in the normal zone
        assert_eq!(trigger.check(OrderedFloat(16.0)), true); // it should trigger immediately in the critical zone
        thread::sleep(Duration::from_millis(100));
        assert_eq!(trigger.check(OrderedFloat(16.0)), true); // it should trigger immediately in the critical zone
    }
}
