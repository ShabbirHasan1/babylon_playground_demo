use crate::{
    trigger::{Trigger, TriggerError, TriggerMetadata},
    types::OrderedValueType,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct DelayedTrigger {
    delay:      Duration,
    start_time: Option<Instant>,
    triggered:  bool,
}

impl DelayedTrigger {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            start_time: Some(Instant::now()),
            triggered: false,
        }
    }

    fn check_time_elapsed(&self) -> bool {
        if let Some(start_time) = self.start_time {
            return start_time.elapsed() >= self.delay;
        }

        false
    }
}

impl Trigger for DelayedTrigger {
    fn validate(&self) -> Result<(), TriggerError> {
        Ok(())
    }

    fn check(&mut self, _price: OrderedValueType) -> bool {
        if !self.triggered && self.check_time_elapsed() {
            self.triggered = true;
            true
        } else {
            false
        }
    }

    fn reset(&mut self) {
        self.start_time = Some(Instant::now());
        self.triggered = false;
    }

    fn metadata(&self) -> TriggerMetadata {
        TriggerMetadata::delayed(self.delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::OrderedValueType;
    use ordered_float::OrderedFloat;
    use std::thread::sleep;

    #[test]
    fn test_delayed_trigger() {
        let mut trigger = DelayedTrigger::new(Duration::from_millis(500));
        assert_eq!(trigger.check(OrderedFloat(0.0)), false);

        sleep(Duration::from_millis(600));

        assert_eq!(trigger.check(OrderedFloat(0.0)), true);
    }
}
