//! What decides, and what it needs from outside.
//!
//! ADR 0002 records this arrangement. `domain` is private, so the types below
//! are what the rest of `cisternd` sees.

mod domain;
pub mod port;
pub mod service;

use port::Unavailable;

/// A setting that was stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub key: String,
    pub value: String,
}

/// What was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    /// One key, holding nothing when nobody has set it.
    One { key: String, value: Option<String> },
    /// Every key that holds something.
    All { entries: Vec<(String, String)> },
}

/// A task that was registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub id: String,
    pub title: String,
    pub base_branch: String,
    pub after: Option<String>,
    pub model: Option<String>,
    pub repository: String,
    pub state: String,
}

/// A task that was taken out of the backlog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub id: String,
    pub title: String,
}

/// One task in full.
///
/// The fields a session fills in are here and empty. Nothing runs a task yet,
/// so they answer as null, which is what section 2.1 says they do before a task
/// has been assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub id: String,
    pub session: Option<String>,
    pub state: String,
    pub title: String,
    pub base_branch: String,
    pub after: Option<String>,
    pub model: Option<String>,
    pub repository: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    pub worktree: Option<String>,
    pub disposition: Option<String>,
}

/// The tasks waiting to be assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub items: Vec<Waiting>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub id: String,
    pub title: String,
    pub base_branch: String,
}

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
