use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Cooldown {
    duration:    Duration,
    uses_left:   u32,
    total_uses:  u32,
    last_used:   Instant,
    in_cooldown: bool,
}

impl Cooldown {
    pub fn new(cooldown: Duration, uses_per_cooldown: u32) -> Self {
        Self {
            duration:    cooldown,
            uses_left:   uses_per_cooldown,
            total_uses:  uses_per_cooldown,
            last_used:   Instant::now(),
            in_cooldown: false,
        }
    }

    pub fn use_cooldown(&mut self) -> bool {
        if self.uses_left > 0 && !self.in_cooldown {
            self.uses_left -= 1;
            if self.uses_left == 0 {
                self.in_cooldown = true;
                self.last_used = Instant::now();
            }
            return true;
        }

        if self.uses_left > 0 && self.is_cooldown_expired() {
            self.reset();
            return true;
        }

        if self.is_cooldown_expired() {
            self.reset();
            return true;
        }

        false
    }

    pub fn uses_left(&self) -> u32 {
        self.uses_left
    }

    pub fn downtime_left(&self) -> Duration {
        if self.in_cooldown {
            self.duration - self.last_used.elapsed()
        } else {
            Duration::from_secs(0)
        }
    }

    pub fn is_cooldown_expired(&self) -> bool {
        self.in_cooldown && self.last_used.elapsed() >= self.duration
    }

    pub fn is_available(&self) -> bool {
        (self.uses_left > 0 && !self.in_cooldown) || self.is_cooldown_expired()
    }

    fn reset(&mut self) {
        self.uses_left = self.total_uses;
        self.in_cooldown = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    #[test]
    fn test_cooldown() {
        let mut cooldown = Cooldown::new(Duration::from_secs(1), 2);

        // At first, the cooldown is available for use
        assert!(cooldown.is_available());
        assert!(!cooldown.is_cooldown_expired());

        // Applying the cooldown, still available for one more use
        assert!(cooldown.use_cooldown());
        assert!(cooldown.is_available());
        assert!(!cooldown.is_cooldown_expired());

        // Consuming the last use, goes into cooldown
        assert!(cooldown.use_cooldown());
        assert!(!cooldown.is_available());
        assert!(!cooldown.is_cooldown_expired());

        // In cooldown, so apply returns false
        assert!(!cooldown.use_cooldown());
        assert!(!cooldown.is_available());
        assert!(!cooldown.is_cooldown_expired());

        // After waiting for more than the cooldown duration, it should be available again
        sleep(Duration::from_secs(2));
        assert!(cooldown.is_cooldown_expired());
        assert!(cooldown.is_available());
        assert!(cooldown.use_cooldown());
    }
}
