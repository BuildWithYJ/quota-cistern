//! The entities and the rules over them.
//!
//! One file per concept. The module is private, so what a file inside declares
//! public is still out of reach from outside `core`.

mod configuration;
mod task;

pub use configuration::{Configuration, Key, Setting};
pub use task::{Backlog, NotABacklog, RemovalRefused, Repository, TaskId};
