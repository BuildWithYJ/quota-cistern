//! What the core offers, in the core's own terms.
//!
//! One trait per command group, each beside the values its commands answer
//! with. `Refusal` stands here rather than in one of them, because any command
//! may end in one.
//!
//! Nothing here names an exit code or an envelope. Whoever calls a use case
//! decides how a refusal reaches whoever asked.

mod backlog;
mod configuration;

pub use backlog::{Added, BacklogUseCase, Detail, Listing, Registration, Removed, Waiting};
pub use configuration::{Applied, ConfigurationUseCase, View};

use super::outbound::Unavailable;

/// Why the core would not do what was asked.
///
/// It names what was wrong and stops there. Which exit code that becomes is the
/// same question as which envelope carries it, and both belong to the caller,
/// so the codes in `docs/cli.md` do not reach in here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No such key in the specification.
    UnknownKey { key: String },
    /// A key that exists, holding a value it does not take.
    BadValue { key: String, value: String },
    /// No task carries that number.
    NoSuchTask { id: String },
    /// The task has been assigned, so the backlog no longer holds it.
    NotPending { id: String },
    /// The command was run somewhere that belongs to no repository.
    NotARepository { at: String },
    /// The store could not be reached or could not be understood.
    Unavailable { reason: String },
}

impl From<Unavailable> for Refusal {
    fn from(e: Unavailable) -> Self {
        Refusal::Unavailable { reason: e.reason }
    }
}
