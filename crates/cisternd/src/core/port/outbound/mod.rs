//! What the core requires of the outside, in the core's own terms.
//!
//! One outside, one place.
//! An outside that holds several conversations gets a directory of its own and an outside that holds one gets a file.
//! `Unavailable` stands here rather than in one of them, because reaching something outside is what all of them do.

mod backlog;
mod clock;
mod configuration;
mod repository;
mod session;
mod trace;
mod vendor;

pub use backlog::{BacklogStore, StoredBacklog, StoredConsumption, StoredTask};
pub use clock::Clock;
pub use configuration::{ConfigurationStore, StoredConfiguration};
pub use repository::{
    Between, Changes, Counts, Cut, NotApplied, RepositoryRoots, Results, Touched, Worktrees,
};
pub use session::{SessionStore, StoredSession, StoredSessions};
pub use trace::{Event, Keeping, Read, Traces};
pub use vendor::{Agent, Ended, Limit, Observed, Outcome, Reading, Spent, Work};

/// The outside could not be reached or could not be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    /// What went wrong, in the words of whatever failed.
    pub reason: String,
}

impl Unavailable {
    pub fn new(reason: impl Into<String>) -> Self {
        Unavailable {
            reason: reason.into(),
        }
    }
}
