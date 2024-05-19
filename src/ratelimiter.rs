use log::info;
use nonzero_ext::nonzero;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard, RawMutex};
use ratelimit_meter::{algorithms::NotUntil, DirectRateLimiter, LeakyBucket, NegativeMultiDecision, NonConformance, GCRA};
use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Inner {
    websocket_messages: DirectRateLimiter<LeakyBucket>,
    request_weights:    DirectRateLimiter<LeakyBucket>,
    raw_requests:       DirectRateLimiter<LeakyBucket>,
    roundtrips:         DirectRateLimiter<LeakyBucket>,
    orders:             DirectRateLimiter<LeakyBucket>,
    orders_per_day:     DirectRateLimiter<LeakyBucket>,
    metadata_updates:   DirectRateLimiter<LeakyBucket>,
}

// TODO: Re-implement this to use a wait_until function instead of a loop.
#[derive(Debug, Clone)]
pub struct UnifiedRateLimiter(Arc<Mutex<Inner>>);

impl Default for UnifiedRateLimiter {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Inner {
            websocket_messages: DirectRateLimiter::<LeakyBucket>::new(nonzero!(2u32), Duration::from_secs(1)),
            request_weights:    DirectRateLimiter::<LeakyBucket>::new(nonzero!(5500u32), Duration::from_secs(60)),
            raw_requests:       DirectRateLimiter::<LeakyBucket>::new(nonzero!(5500u32), Duration::from_secs(60)),
            roundtrips:         DirectRateLimiter::<LeakyBucket>::new(nonzero!(1u32), Duration::from_secs(1)),
            orders:             DirectRateLimiter::<LeakyBucket>::new(nonzero!(90u32), Duration::from_secs(10)),
            orders_per_day:     DirectRateLimiter::<LeakyBucket>::new(nonzero!(200000u32), Duration::from_secs(86400)),
            metadata_updates:   DirectRateLimiter::<LeakyBucket>::new(nonzero!(10u32), Duration::from_secs(1)),
        })))
    }
}

impl UnifiedRateLimiter {
    pub fn wait_for_websocket_message_limit(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().websocket_messages.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().websocket_messages.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }

    pub fn wait_for_request_weight_limit(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().request_weights.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().request_weights.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }

    pub fn wait_for_raw_request_limit(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().raw_requests.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().raw_requests.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }

    pub fn wait_for_roundtrip_limit_per_second(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().roundtrips.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().roundtrips.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }

    pub fn wait_for_order_limit_per_second(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().orders.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().orders.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }

    pub fn wait_for_order_limit_per_day(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().orders_per_day.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().orders_per_day.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }

    pub fn wait_for_metadata_updates_per_second(&self, weight: u32) {
        loop {
            let result = match weight {
                0 => break, // If the weight is 0, exit the loop. This is used for disabling rate limits.
                1 => match self.0.lock().metadata_updates.check() {
                    Ok(_) => break, // If the check is successful, exit the loop.
                    Err(err) => {
                        // If the check fails, sleep for the required wait time and then continue the loop.
                        std::thread::sleep(err.wait_time_from(Instant::now()));
                    }
                },
                _ => match self.0.lock().metadata_updates.check_n(weight) {
                    Ok(_) => break,
                    Err(NegativeMultiDecision::InsufficientCapacity(n)) => {
                        panic!("Weight is greater than available capacity");
                    }
                    Err(NegativeMultiDecision::BatchNonConforming(_, err)) => std::thread::sleep(err.wait_time_from(Instant::now())),
                },
            };
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_ratelimiter() {}
}
