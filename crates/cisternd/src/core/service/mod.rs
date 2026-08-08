//! What the commands do.
//!
//! One service per command group, named for the section of `docs/cli.md` it
//! answers. Each holds the outbound ports its own commands need and implements
//! the use case those commands are declared as.

mod backlog;
mod configuration;

pub use backlog::BacklogService;
pub use configuration::ConfigurationService;
