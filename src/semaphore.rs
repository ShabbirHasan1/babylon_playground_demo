use std::{
    sync::{Arc, Condvar, Mutex},
    thread,
};

struct SemaphoreInner {
    lock: Mutex<usize>,
    cvar: Condvar,
}

#[derive(Clone)]
pub struct Semaphore(Arc<SemaphoreInner>);

impl Semaphore {
    pub fn new(count: usize) -> Self {
        Semaphore(Arc::new(SemaphoreInner {
            lock: Mutex::new(count),
            cvar: Condvar::new(),
        }))
    }

    pub fn acquire(&self) {
        let mut remaining = self.0.lock.lock().unwrap();
        while *remaining == 0 {
            remaining = self.0.cvar.wait(remaining).unwrap();
        }
        *remaining -= 1;
    }

    pub fn release(&self) {
        let mut remaining = self.0.lock.lock().unwrap();
        *remaining += 1;
        self.0.cvar.notify_one();
    }

    pub fn clone(&self) -> Self {
        Semaphore(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[test]
    fn test_semaphore() {
        let max_concurrent = 10;
        let semaphore = Semaphore::new(max_concurrent);
        let counter = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];

        for _ in 0..100 {
            let sem_clone = semaphore.clone();
            let counter_clone = Arc::clone(&counter);
            let max_seen_clone = Arc::clone(&max_seen);

            handles.push(thread::spawn(move || {
                sem_clone.acquire();

                let current = counter_clone.fetch_add(1, Ordering::SeqCst) + 1;
                let max = max_seen_clone.load(Ordering::SeqCst);

                if current > max {
                    max_seen_clone.store(current, Ordering::SeqCst);
                }

                // Simulate an operation with a delay.
                thread::sleep(Duration::from_millis(10));
                counter_clone.fetch_sub(1, Ordering::SeqCst);
                sem_clone.release();
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(max_seen.load(Ordering::SeqCst), max_concurrent);
    }
}
