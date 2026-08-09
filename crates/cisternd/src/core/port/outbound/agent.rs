//! The agent that does a task's work.

use super::Unavailable;

/// What an agent is being asked to do.
pub struct Work<'a> {
    /// The work area it runs in.
    pub at: &'a str,
    pub instruction: &'a str,
    /// The model to run, when the task or the session named one.
    pub model: Option<&'a str>,
}

/// How an agent finished.
///
/// Whatever it wrote is read while it runs and dropped for now. Reading usage
/// out of it and keeping it as a trace are their own issues, and both attach
/// here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    /// Whether it finished the work rather than failing at it.
    pub done: bool,
    /// What it said when it did not, in its own words.
    pub reason: Option<String>,
}

/// The agent, as the core asks for it.
///
/// The core hands over a task's instruction and is told how it ended. Which
/// program that is and how it is asked belong to the implementation, because a
/// budget only means something while what a run consumes can be read, and that
/// can only be promised for a command the implementation writes itself.
pub trait Agent: Sync {
    /// Runs the agent and answers once it has finished.
    ///
    /// This does not return until the agent has, so whoever calls it is not on
    /// a thread that anything else waits for.
    fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable>;
}
