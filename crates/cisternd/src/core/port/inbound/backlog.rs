//! What the core offers over the backlog.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output.
//! Reading what a run wrote is not here: a trace belongs to the run rather than to the task that was registered.
//! `port::inbound::execution` offers it.

use super::Refusal;

/// What `task add` was given.
///
/// The arguments arrive together because they are read together.
/// A parameter list of this length is harder to call correctly than a value with named fields.
pub struct Registration<'a> {
    /// Where the surface was run.
    /// The core runs as a daemon, so it cannot read this from its own process.
    pub cwd: &'a str,
    pub title: &'a str,
    pub instruction: &'a str,
    pub branch: Option<&'a str>,
    pub after: Option<&'a str>,
    pub model: Option<&'a str>,
    /// Register even when the instruction carries too little to run unattended.
    pub force: bool,
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
/// The fields a session fills in are here and empty.
/// They answer as null, which is what section 2.1 says they do before a task has been assigned.
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
    /// The conversation its last run was in, for a task that may be carried on.
    pub conversation: Option<String>,
    pub disposition: Option<String>,
    /// What the branch holds, for a task whose run has ended.
    pub commits: Option<Vec<Made>>,
    /// Commits the base has gained since the task left it.
    pub base_ahead: Option<u64>,
}

/// One commit a task made, as `task show` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    pub sha: String,
    pub subject: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
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

/// `task add`, `task rm`, `task show`, and `backlog`.
pub trait BacklogUseCase {
    /// Registers a task.
    fn add(&self, given: Registration<'_>) -> Result<Added, Refusal>;

    /// Takes a task out of the backlog.
    fn remove(&self, id: &str) -> Result<Removed, Refusal>;

    /// Reads one task in full.
    fn show(&self, id: &str) -> Result<Detail, Refusal>;

    /// Lists the tasks waiting to be assigned.
    fn list(&self) -> Result<Listing, Refusal>;
}
