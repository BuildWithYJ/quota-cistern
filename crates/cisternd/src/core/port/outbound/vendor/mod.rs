//! The vendor that runs a task, and how much of its allowance is left.
//!
//! Two conversations with one outside. Whoever the vendor is, both are asked of
//! the same account, so a second vendor is a second adapter over these two and
//! nothing else.

mod agent;
mod limit;

pub use agent::{Agent, Ended, Observed, Outcome, Spent, Work};
pub use limit::{Limit, Reading};
