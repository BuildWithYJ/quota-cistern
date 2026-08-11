//! The agent that does a task's work.

use super::super::Unavailable;

/// What an agent is being asked to do.
pub struct Work<'a> {
    /// The task this is a run of, which is what a later `stop` names.
    pub task: &'a str,
    /// The work area it runs in.
    pub at: &'a str,
    /// Where to keep what the run writes.
    /// The core does not read it.
    pub trace: &'a str,
    pub instruction: &'a str,
    /// The model to run, when the task or the session named one.
    pub model: Option<&'a str>,
}

/// What a run consumed, in the core's own words.
///
/// A vendor reports these under names of its own, and none of those reaches here.
/// Every figure crosses as the text it was written as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spent {
    pub input: String,
    pub output: String,
    pub cache_written: String,
    pub cache_read: String,
    /// What the vendor priced the run at, in millionths of its currency.
    pub cost: String,
}

/// What the answer said about what the run consumed.
///
/// An answer that could not be read is not an answer of nothing.
/// A vendor that renamed a field would otherwise report a full session as having spent nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Spent(Spent),
    Unreadable { why: String },
}

/// How a run came to an end.
///
/// Three rather than a pass and a fail.
/// Section 1 of `docs/cli.md` leaves the session running for one of the failures and not the other.
/// Why a run failed is not here: the vendor's own limit looks like any other failure.
/// Telling them apart takes a question this cannot answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It did the work.
    Finished,
    /// It was stopped at the ceiling on one run, whatever that ceiling is.
    AtCeiling,
    /// It stopped without doing the work.
    Failed,
}

/// How an agent finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub outcome: Outcome,
    /// What it said about how it ended, in its own words.
    pub reason: Option<String>,
    /// What the run consumed.
    pub observed: Observed,
}

/// The agent, as the core asks for it.
///
/// An implementation owes it a figure for what the run consumed.
/// A budget means nothing while that cannot be read.
pub trait Agent: Sync {
    /// Runs the agent and answers once it has finished.
    ///
    /// This does not return until the agent has, so whoever calls it is not on a thread that anything else waits for.
    fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable>;

    /// Ends the run a task has going, if it still has one.
    ///
    /// Nothing is answered, and asking about a task that is not running does nothing.
    /// How the task ended is the core's to record.
    fn stop(&self, task: &str);
}
