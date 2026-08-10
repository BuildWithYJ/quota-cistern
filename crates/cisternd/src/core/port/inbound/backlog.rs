//! What the core offers over the backlog.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output.

use super::Refusal;

/// What `task add` was given.
///
/// The arguments arrive together because they are read together, and a
/// parameter list of this length is harder to call correctly than a value with
/// named fields.
pub struct Registration<'a> {
    /// Where the surface was run. The core runs as a daemon, so it cannot read
    /// this from its own process.
    pub cwd: &'a str,
    pub title: &'a str,
    pub instruction: &'a str,
    pub branch: Option<&'a str>,
    pub after: Option<&'a str>,
    pub model: Option<&'a str>,
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

/// One thing a run did or said, as `trace` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Happened {
    pub at: String,
    pub said: String,
}

/// A stretch of a task's trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trail {
    pub events: Vec<Happened>,
    /// Where to carry on from.
    pub cursor: String,
    /// True once the task has ended and the trace can grow no more.
    pub done: bool,
}

/// `task add`, `task rm`, `task show`, and `backlog`.
pub trait BacklogUseCase {
    /// Registers a task.
    fn add(&self, given: Registration<'_>) -> Result<Added, Refusal>;

    /// Takes a task out of the backlog.
    fn remove(&self, id: &str) -> Result<Removed, Refusal>;

    /// Reads one task in full.
    fn show(&self, id: &str) -> Result<Detail, Refusal>;

    /// What a task's run has written so far, from `since` onwards.
    ///
    /// Section 2.3 answers the same way whether the task is still running or
    /// has ended, so nothing here asks which.
    fn trace(&self, id: &str, since: Option<&str>) -> Result<Trail, Refusal>;

    /// Lists the tasks waiting to be assigned.
    fn list(&self) -> Result<Listing, Refusal>;
}
