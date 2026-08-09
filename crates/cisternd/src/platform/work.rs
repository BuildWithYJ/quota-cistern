//! A queue of work the daemon carries on with after it has answered.
//!
//! It touches no port. What is queued is a piece of work someone else built,
//! and this only decides when a thread picks it up.

use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, PoisonError},
};

/// Work waiting for a thread, and threads waiting for work.
#[derive(Default)]
pub struct Queue {
    waiting: Mutex<VecDeque<String>>,
    arrived: Condvar,
}

impl Queue {
    /// Adds one piece of work and wakes a thread to take it.
    pub fn add(&self, what: String) {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.push_back(what);
        self.arrived.notify_one();
    }

    /// Waits until there is something to do, and takes it.
    ///
    /// It never answers with nothing, because the daemon runs until it is
    /// stopped and a thread with nothing to do should sleep rather than spin.
    pub fn take(&self) -> String {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(what) = waiting.pop_front() {
                return what;
            }
            waiting = self
                .arrived
                .wait(waiting)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use super::*;

    #[test]
    fn what_was_added_is_what_is_taken_and_in_that_order() {
        let queue = Queue::default();
        queue.add("task:1".to_owned());
        queue.add("task:2".to_owned());

        assert_eq!(queue.take(), "task:1");
        assert_eq!(queue.take(), "task:2");
    }

    /// A thread that finds nothing waits rather than answering, so a daemon
    /// with nothing to do is not spinning.
    #[test]
    fn a_thread_that_finds_nothing_waits_for_it() {
        let queue = Arc::new(Queue::default());
        let waiting = Arc::clone(&queue);
        let taken = thread::spawn(move || waiting.take());

        queue.add("task:7".to_owned());
        assert_eq!(taken.join().unwrap(), "task:7");
    }
}
