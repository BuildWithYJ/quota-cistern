//! What the commands do.
//!
//! One file per command group, named for the section of `docs/cli.md` it
//! answers.

mod backlog;
mod configuration;

pub use backlog::{Registration, add, list, remove, show};
pub use configuration::{get, set};
