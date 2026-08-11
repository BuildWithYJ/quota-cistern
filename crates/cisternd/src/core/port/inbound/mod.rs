//! What the core offers, in the core's own terms.
//!
//! One trait per command group, each beside the values its commands answer with.
//! `Refusal` stands here rather than in one of them, because any command may end in one.

mod backlog;
mod configuration;
mod execution;
mod review;
mod work;

pub use backlog::{Added, BacklogUseCase, Detail, Listing, Registration, Removed, Waiting};
pub use configuration::{Applied, ConfigurationUseCase, View};
pub use execution::{
    Declaration, Declared, ExecutionUseCase, Happened, Listed, Page, Ran, Report, Started, Stopped,
    Trail,
};
pub use review::{Awaiting, Changed, Difference, Dropped, Queue, ReviewUseCase, Taken};
pub use work::{Carrying, NotCarried};

use super::outbound::Unavailable;

/// Why the core would not do what was asked.
///
/// It names what was wrong and stops there.
/// Which exit code that becomes is the same question as which envelope carries it.
/// Both belong to the caller, so the codes in `docs/cli.md` do not reach in here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No such key in the specification.
    UnknownKey { key: String },
    /// A key that exists, holding a value it does not take.
    BadValue { key: String, value: String },
    /// No task carries that number.
    NoSuchTask { id: String },
    /// No session carries that identifier.
    NoSuchSession { id: String },
    /// The task has been assigned, so the backlog no longer holds it.
    NotPending { id: String },
    /// The command was run somewhere that belongs to no repository.
    NotARepository { at: String },
    /// A session is running, and section 2.2 allows one at a time.
    AlreadyRunning { id: String },
    /// Nothing is running, and the command only makes sense while one is.
    NoSessionRunning,
    /// Nothing in the backlog may start.
    NothingToAssign,
    /// The run has not ended, so there is no result to decide about.
    NotEnded { id: String },
    /// The repository holds changes nobody has committed.
    Uncommitted { at: String },
    /// The task left nothing on its branch.
    NoChange { id: String },
    /// What the task changed is in the working tree already.
    AlreadyApplied { id: String },
    /// What the task changed does not go into the working tree as it stands.
    Conflicts { why: String },
    /// The result branch is not in the repository the task was added from.
    NoResult { branch: String, at: String },
    /// The repository the task was added from cannot be read.
    NoRepository { at: String },
    /// The store could not be reached or could not be understood.
    Unavailable { reason: String },
}

impl From<Unavailable> for Refusal {
    fn from(e: Unavailable) -> Self {
        Refusal::Unavailable { reason: e.reason }
    }
}
