//! The repository a task was added from.
//!
//! Three conversations with one outside.
//! It belongs to whoever is using this and they may change it between one command and the next.
//! What is asked of it is asked again each time rather than remembered.

mod grounding;
mod result;
mod roots;
mod surroundings;
mod worktree;

pub use grounding::{Grounding, Ran};
pub use result::{Between, Changes, Commit, Counts, NotApplied, Results, Touched};
pub use roots::RepositoryRoots;
pub use surroundings::{Room, Surroundings};
pub use worktree::{Cut, Worktrees};
