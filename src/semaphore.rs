//! A counting semaphore.
//!
//! Based on old stdlib code.

use std::sync::Condvar;
use std::sync::Mutex;

pub struct Semaphore {
    permits: Mutex<usize>,
    not_empty: Condvar,
}

impl Semaphore {
    pub fn new(count: usize) -> Semaphore {
        Semaphore {
            permits: Mutex::new(count),
            not_empty: Condvar::new(),
        }
    }

    /// block until at least 1, then take it
    pub fn acquire(&self) {
        let mut count = self.permits.lock().unwrap();
        while 0 == *count {
            count = self.not_empty.wait(count).unwrap();
        }
        *count -= 1;
    }

    /// increase available permits
    pub fn release(&self) {
        *self.permits.lock().unwrap() += 1;
        self.not_empty.notify_one();
    }
}
