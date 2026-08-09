//! The system clock.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::port::outbound::Clock;

/// What the machine says the time is.
pub struct SystemClock;

impl Clock for SystemClock {
    /// Seconds since the epoch, and zero for a machine set before it.
    ///
    /// A clock behind the epoch would otherwise stop the daemon over something
    /// no session cares about. Every reading is subtracted from a later one,
    /// and two readings from such a clock are as far apart as they ever were.
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs())
    }
}
