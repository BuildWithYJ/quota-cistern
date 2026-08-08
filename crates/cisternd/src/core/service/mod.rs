//! What the commands do.
//!
//! One file per command group, named for the section of `docs/cli.md` it
//! answers.

mod configuration;

pub use configuration::{get, set};
