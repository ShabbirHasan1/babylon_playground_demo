use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
};

#[derive(Clone)]
struct WaitGroup {
    counter: Arc<AtomicUsize>,
}

impl WaitGroup {
    fn new() -> WaitGroup {
        WaitGroup {
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn add(&self, count: usize) {
        self.counter.fetch_add(count, Ordering::SeqCst);
    }

    fn done(&self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }

    fn wait(&self) {
        while self.counter.load(Ordering::SeqCst) > 0 {
            thread::yield_now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_waitgroup() {
        let wg = WaitGroup::new();
        wg.add(1);

        let wg_clone = wg.clone();
        thread::spawn(move || {
            println!("Doing some work in a thread");
            wg_clone.done();
        });

        wg.wait();
        println!("All work is done");
    }
}
