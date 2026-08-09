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
    /// The tasks assigned at the start.
    pub assigned: Vec<String>,
    pub budget: Declared,
}

/// `run`, and the work a run leaves behind.
pub trait ExecutionUseCase {
    /// Opens a session and assigns what may start.
    ///
    /// It answers as soon as the session exists, because section 2.2 says the
    /// command returns at once and the tasks keep running.
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal>;

    /// Runs one assigned task to the end, and answers with whatever the
    /// decision that followed assigned next.
    ///
    /// This does not return until the task has, so whoever calls it is not the
    /// same thread that answered `run`. When that happens is the composition
    /// root's to arrange; what happens is here. What comes back has to be run
    /// the same way the tasks `run` answered with are.
    fn carry_on(&self, task: &str) -> Result<Vec<String>, Refusal>;
}
