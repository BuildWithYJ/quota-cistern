//! What answers the vendor ports, for Claude Code.
//!
//! Every name this vendor uses stops here: its arguments, the fields of its answer, and the words on its status line.

pub mod agent;
pub mod drafter;
pub mod limit;

/// What a user writes to choose this vendor.
///
/// The core does not hold the name. It is told which names this build can run,
/// and this is the one this module answers to.
pub const NAME: &str = "claude";
