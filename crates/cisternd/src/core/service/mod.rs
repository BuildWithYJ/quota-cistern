//! What the commands do, and what a session does between them.
//!
//! One service per command group, named for the section of `docs/cli.md` it answers.
//! Each holds the outbound ports its own commands need and implements the use case those commands are declared as.
//!
//! `supervision` answers no command. Whether a session carries on and with what is one
//! decision, and both the commands and the daemon's own workers reach it, so it is a role of
//! its own rather than something either of them owns.

use crate::core::domain::TaskId;

#[cfg(test)]
pub(super) mod fixtures;

mod backlog;
mod configuration;
mod execution;
mod review;
mod sessions;
mod supervision;
mod work;

pub use backlog::BacklogService;
pub use configuration::ConfigurationService;
pub use execution::ExecutionService;
pub use review::ReviewService;
pub use supervision::{Outside, Supervisor};
pub use work::WorkService;

/// Task numbers as a person reads them.
///
/// Here rather than beside one of them, since every service that answers with a task answers
/// with it in this form.
pub(super) fn labelled(ids: Vec<TaskId>) -> Vec<String> {
    ids.iter().map(TaskId::labelled).collect()
}
