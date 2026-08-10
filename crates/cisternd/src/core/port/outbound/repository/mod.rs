//! The repository a task was added from.
//!
//! Three conversations with one outside. It belongs to whoever is using this
//! and they may change it between one command and the next, so what is asked
//! of it is asked again each time rather than remembered.

mod result;
mod roots;
mod worktree;

pub use result::{Between, Changes, Counts, NotApplied, Results, Touched};
pub use roots::RepositoryRoots;
pub use worktree::{Cut, Worktrees};
