use num::Integer;
use num_traits::{One, Unsigned, Zero};

#[derive(Debug, Copy, Clone)]
pub struct Countdown<T: Unsigned + Copy + Integer> {
    value:         T,
    initial_value: T,
}

impl<T: Unsigned + Copy + Integer> Countdown<T> {
    pub fn new(value: T) -> Countdown<T> {
        Countdown { value, initial_value: value }
    }

    pub fn checkin(&mut self) -> bool {
        if self.value > T::zero() {
            self.value = self.value - T::one();
            true
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        self.value = self.initial_value;
    }

    pub fn value(&self) -> (T, T) {
        (self.value, self.initial_value)
    }
}
