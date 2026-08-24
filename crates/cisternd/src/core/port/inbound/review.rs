//! What the core offers over a result and what is done about it.
//!
//! Sections 2.3 and 2.4 of `docs/cli.md` fix the arguments and the output.

use super::Refusal;

/// One file a task changed.
///
/// A count is absent for a file git counts no lines in, which is what a binary file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub path: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

/// What a task changed, as `diff` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub base: String,
    /// The result branch, or nothing for a task that never got one.
    pub branch: Option<String>,
    pub files: Vec<Changed>,
    pub patch: String,
}

/// One task waiting to be disposed of.
///
/// The two counts are absent for a task whose branch cannot be read.
/// The repository belongs to whoever is using this and they may delete a branch.
/// A task that was registered still has to appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awaiting {
    pub id: String,
    pub title: String,
    pub session: Option<String>,
    pub branch: Option<String>,
    pub state: String,
    pub commit_count: Option<u64>,
    pub base_ahead: Option<u64>,
}

/// Everything waiting to be disposed of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub items: Vec<Awaiting>,
}

/// A result that was brought into the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    pub task: String,
    pub branch: String,
    pub files: Vec<Changed>,
}

/// A result that was taken out of the queue and left where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub task: String,
    pub branch: String,
}

/// `diff`, `review ls`, `apply`, and `discard`.
pub trait ReviewUseCase {
    /// What a task changed on its branch.
    ///
    /// A task that never got a branch changed nothing.
    /// Section 2.3 reports that the same way as a branch holding nothing.
    fn diff(&self, id: &str) -> Result<Difference, Refusal>;

    /// Everything waiting to be disposed of, across sessions.
    fn queue(&self) -> Result<Queue, Refusal>;

    /// Brings a result into the working tree of the repository it came from.
    fn apply(&self, id: &str) -> Result<Taken, Refusal>;

    /// Takes a result out of the queue and leaves it where it is.
    fn discard(&self, id: &str) -> Result<Dropped, Refusal>;

    /// Puts a task that ended back where it started, so a session may do it over.
    fn retry(&self, id: &str) -> Result<Requeued, Refusal>;

    /// The same, keeping the conversation its last run was in, so the next run carries that
    /// conversation on rather than starting one.
    fn resume(&self, id: &str) -> Result<Requeued, Refusal>;

    /// Takes away the work areas of tasks that have been disposed of.
    fn tidy(&self) -> Result<Tidying, Refusal>;
}

/// One task's work area, and what became of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tidied {
    pub task: String,
    pub worktree: String,
    /// Why it was left where it is, or nothing for one that was taken away.
    pub kept: Option<String>,
}

/// What tidying up came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tidying {
    pub items: Vec<Tidied>,
}

/// A task that ended and is waiting again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requeued {
    pub task: String,
    /// The branch its last run left, which stays.
    pub branch: String,
    /// How many times it has been assigned so far.
    pub attempts: String,
    /// Whether the next run carries a conversation on, or starts one.
    pub carries_on: bool,
}
