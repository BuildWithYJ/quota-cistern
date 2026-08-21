//! The vendor that runs a task, reads what it is missing, and says how much allowance is left.
//!
//! Conversations with one outside.
//! Whoever the vendor is, all are asked of the same account.
//! A second vendor is a second adapter over these and nothing else.

mod agent;
mod drafter;
mod limit;

pub use agent::{Agent, Ended, Observed, Outcome, Spent, Work};
pub use drafter::{Draft, Drafted, Drafter};
pub use limit::{Limit, Reading};
