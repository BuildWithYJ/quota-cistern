//! What the core offers over sessions and execution.
//!
//! Section 2.2 of `docs/cli.md` fixes the arguments and the output.

use super::Refusal;

/// What `run` was given.
pub struct Declaration<'a> {
    /// What may be consumed, as it was written.
    pub usage: &'a str,
    /// How long it may run, as it was written.
    pub time: &'a str,
    /// The model tasks fall back to when they name none.
    pub model: Option<&'a str>,
}

/// The budget a session was opened with, in the words it was declared in.
///
/// Section 2.2 reports consumption in the unit that was declared, so what was
/// written is what is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub usage: String,
    pub time: String,
}

/// A session that was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub session: String,
    pub state: String,
    /// Tasks assigned at the start.
    pub assigned: u32,
    pub budget: Declared,
}

/// `run`.
pub trait ExecutionUseCase {
    /// Opens a session and starts what it was able to assign.
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal>;
}
