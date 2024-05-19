use parking_lot::{MappedMutexGuard, Mutex, MutexGuard, RawMutex};
use std::{fmt::Debug, sync::Arc, thread};
use tokio::sync::watch;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    NotStarted,
    Initializing,
    Processing,
    Finished,
    Cancelled,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ProgressInfo<T> {
    state:    TaskState,
    current:  usize,
    total:    usize,
    metadata: T,
}

#[derive(Clone)]
pub struct ProgressTracker<T>
where
    T: Clone + 'static + Default + Send,
{
    info:     Arc<Mutex<ProgressInfo<T>>>,
    state_tx: Arc<Mutex<watch::Sender<TaskState>>>,
    state_rx: watch::Receiver<TaskState>,
}

impl<T> ProgressTracker<T>
where
    T: Clone + 'static + Default + Send,
{
    pub fn new(total: usize, metadata: T) -> Self {
        let (state_tx, state_rx) = watch::channel(TaskState::NotStarted);
        Self {
            info: Arc::new(Mutex::new(ProgressInfo {
                state: TaskState::NotStarted,
                current: 0,
                total,
                metadata,
            })),
            state_tx: Arc::new(Mutex::new(state_tx)),
            state_rx,
        }
    }

    pub fn start_if_not_started(&self) {
        {
            let mut info = self.info.lock();
            if info.state != TaskState::NotStarted {
                return;
            }
        }

        self.set_state(TaskState::Processing);
    }

    pub fn increment(&self) {
        self.start_if_not_started();

        {
            let mut info = self.info.lock();

            info.current += 1;

            if info.current < info.total {
                return;
            }
        }

        self.set_state(TaskState::Finished);
    }

    pub fn set_total(&self, new_total: usize) {
        self.info.lock().total = new_total;
    }

    pub fn update<F>(&self, f: F)
    where
        F: FnOnce(&mut T),
    {
        self.start_if_not_started();
        let mut info = self.info.lock();
        f(&mut info.metadata);
    }

    pub fn get_info(&self) -> ProgressInfo<T> {
        let info = self.info.lock();
        info.clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<TaskState> {
        self.state_rx.clone()
    }

    pub fn set_state(&self, new_state: TaskState) {
        let mut info = self.info.lock();
        match new_state {
            TaskState::Initializing => {
                // Transition to Initializing is only possible from NotStarted (this state is optional)
                if let TaskState::NotStarted = info.state {
                    info.state = new_state.clone();
                    let _ = self.state_tx.lock().send(new_state);
                }
            }

            TaskState::Processing => {
                // Transition to Processing is only possible from Initializing or NotStarted
                match info.state {
                    TaskState::Initializing | TaskState::NotStarted => {
                        info.state = new_state.clone();
                        let _ = self.state_tx.lock().send(new_state);
                    }
                    _ => {}
                }
            }

            TaskState::Cancelled | TaskState::Error(_) => {
                // Transition to Cancelled or Error is always possible from NotStarted or Processing
                match info.state {
                    TaskState::NotStarted | TaskState::Initializing | TaskState::Processing => {
                        info.state = new_state.clone();
                        let _ = self.state_tx.lock().send(new_state);
                    }
                    _ => {}
                }
            }
            TaskState::Finished => {
                // Transition to Finished is always possible from NotStarted or Processing
                match info.state {
                    TaskState::NotStarted | TaskState::Initializing | TaskState::Processing => {
                        info.state = new_state.clone();
                        let _ = self.state_tx.lock().send(new_state);
                    }
                    _ => {}
                }
            }

            _ => {}
        }
    }

    pub fn cancel(&self) {
        self.set_state(TaskState::Cancelled)
    }

    pub fn error(&self, err: String) {
        self.set_state(TaskState::Error(err))
    }

    pub fn finish(&self) {
        self.set_state(TaskState::Finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Default)]
    struct TestMetadata {
        name: String,
    }

    #[tokio::test]
    async fn test_progress_tracker_new() {
        // Create a new ProgressTracker
        let total = 10;
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());

        // The initial state should be NotStarted
        let initial_state = tracker.get_info().state;
        assert_eq!(initial_state, TaskState::NotStarted);

        // The initial progress should be 0
        let initial_progress = tracker.get_info().current;
        assert_eq!(initial_progress, 0);
    }

    #[tokio::test]
    async fn test_progress_tracker_increment() {
        let total = 10;
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());

        let initial_state = tracker.get_info().state;
        assert_eq!(initial_state, TaskState::NotStarted);

        // Increment the tracker and assert the state is Processing
        tracker.increment();
        assert_eq!(tracker.get_info().state, TaskState::Processing);
        assert_eq!(tracker.get_info().current, 1);

        let initial_state = tracker.get_info().state;
        assert_eq!(initial_state, TaskState::Processing);

        // Increment the tracker to the total and assert the state is Finished
        for _ in 1..total {
            tracker.increment();
        }

        assert_eq!(tracker.get_info().state, TaskState::Finished);
        assert_eq!(tracker.get_info().current, total);
    }

    #[tokio::test]
    async fn test_progress_tracker_update() {
        // Set up a ProgressTracker with a total of 10 and default metadata
        let total = 10;
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());

        // Update the metadata
        let updated_name = "updated".to_string();
        tracker.update(|metadata| {
            metadata.name = updated_name.clone();
        });

        // Assert that the metadata has been updated correctly
        assert_eq!(tracker.get_info().metadata.name, updated_name);
    }

    #[tokio::test]
    async fn test_progress_tracker_set_state() {
        let total = 10;
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());

        // Increment the tracker to the total and assert the state is Finished
        for _ in 0..total {
            tracker.increment();
        }
        assert_eq!(tracker.get_info().state, TaskState::Finished);

        // Try to set an error and assert the state is still Finished
        let error_message = "test error".to_string();
        tracker.error(error_message.clone());
        assert_eq!(tracker.get_info().state, TaskState::Finished);

        // Try to set the tracker state to Cancelled, it should still be Finished
        tracker.cancel();
        assert_eq!(tracker.get_info().state, TaskState::Finished);

        // Reset tracker and set an error when Processing, assert the state is Error
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());
        tracker.increment();
        tracker.error(error_message.clone());
        assert_eq!(tracker.get_info().state, TaskState::Error(error_message.clone()));

        // Reset tracker and set it to Cancelled when Processing, assert the state is Cancelled
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());
        tracker.increment();
        tracker.cancel();
        assert_eq!(tracker.get_info().state, TaskState::Cancelled);
    }

    #[tokio::test]
    async fn test_progress_tracker_finish() {
        let total = 10;
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());

        // Finish the tracker and assert the state is Finished
        tracker.finish();
        assert_eq!(tracker.get_info().state, TaskState::Finished);
    }

    #[tokio::test]
    async fn test_progress_tracker_state_changes() {
        let total = 10;
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());

        // Subscribe to state changes
        let mut rx = tracker.subscribe();

        // Start the task and check that the state changes to Processing
        tracker.start_if_not_started();
        assert_eq!(rx.borrow().clone(), TaskState::Processing);

        // Increment the progress and check that the state is still Processing
        tracker.increment();
        assert_eq!(rx.borrow().clone(), TaskState::Processing);

        // Finish the task and check that the state changes to Finished
        tracker.finish();
        assert_eq!(rx.borrow().clone(), TaskState::Finished);

        // Cancel the task and check that the state does not change
        tracker.cancel();
        assert_eq!(rx.borrow().clone(), TaskState::Finished);

        // Create a new tracker and subscribe to state changes
        let tracker = ProgressTracker::<TestMetadata>::new(total, TestMetadata::default());
        let mut rx = tracker.subscribe();

        // Start the task and check that the state changes to Processing
        tracker.start_if_not_started();
        assert_eq!(rx.borrow().clone(), TaskState::Processing);

        // Cancel the task and check that the state changes to Cancelled
        tracker.cancel();
        assert_eq!(rx.borrow().clone(), TaskState::Cancelled);

        // Try to finish the task and check that the state does not change
        tracker.finish();
        assert_eq!(rx.borrow().clone(), TaskState::Cancelled);
    }
}
