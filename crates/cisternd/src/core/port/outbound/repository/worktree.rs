//! Where a task gets a place of its own to work in.

use super::super::Unavailable;

/// What a work area is being made for.
///
/// The core names the repository, the branch, and where to cut it from.
/// Where on a disk that lands is the implementation's.
pub struct Cut<'a> {
    pub repository: &'a str,
    /// The branch the task starts from.
    pub base: &'a str,
    /// The branch the task's result is kept on.
    pub branch: &'a str,
    /// The task, which is what keeps two work areas apart.
    pub task: &'a str,
}

/// Work areas, one to a task.
///
/// A task runs beside others, so each gets a checkout and a branch that nobody else writes to.
pub trait Worktrees: Sync {
    /// Cuts the branch and puts a work area on it, and says where that is.
    ///
    /// The place it answers with is what the agent runs in and what `task show` reports.
    /// It is a path a person can open.
    fn prepare(&self, cut: Cut<'_>) -> Result<String, Unavailable>;
}
