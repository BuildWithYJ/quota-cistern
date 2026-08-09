//! What the core requires of the outside, in the core's own terms.
//!
//! One file per outside. `Unavailable` stands here rather than in one of them,
//! because reaching something outside is what all of them do.

mod backlog;
mod configuration;
mod repository;
mod session;

pub use backlog::{BacklogStore, StoredBacklog, StoredTask};
pub use configuration::{ConfigurationStore, StoredConfiguration};
pub use repository::RepositoryRoots;
pub use session::{SessionStore, StoredSession, StoredSessions};

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
