//! What the core requires of the outside, in the core's own terms.
//!
//! One file per outside. `Unavailable` stands here rather than in one of them,
//! because reaching something outside is what all of them do.
//!
//! Every one of these is safe to share between threads. A task runs beside the
//! commands a user is still typing, so more than one thread reaches the same
//! outside at once.

mod agent;
mod backlog;
mod configuration;
mod repository;
mod session;
mod worktree;

pub use agent::{Agent, Ended, Observed, Spent, Work};
pub use backlog::{BacklogStore, StoredBacklog, StoredConsumption, StoredTask};
pub use configuration::{ConfigurationStore, StoredConfiguration};
pub use repository::RepositoryRoots;
pub use session::{SessionStore, StoredSession, StoredSessions};
pub use worktree::{Cut, Worktrees};

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
