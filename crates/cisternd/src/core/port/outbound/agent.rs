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

/// What a run consumed, in the core's own words.
///
/// A vendor reports these under names of its own, and none of those names
/// reaches here. Every figure crosses as the text it was written as, for the
/// reason `outbound::backlog` gives.
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
/// An answer that could not be read is not an answer of nothing. A vendor that
/// renames a field would otherwise report a full session as having spent
/// nothing, which reads as a budget nobody has touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Spent(Spent),
    Unreadable { why: String },
}

/// How a run came to an end.
///
/// Three outcomes rather than a pass and a fail, because section 1 of
/// `docs/cli.md` treats one of the failures differently from the rest: a run
/// stopped at its own ceiling leaves the session running, and one that failed
/// on its own account says nothing about the session either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It did the work.
    Finished,
    /// It was stopped at the ceiling on one run, whatever that ceiling is.
    AtCeiling,
    /// It stopped without doing the work.
    Failed,
    /// The vendor would not run it, having reached a limit of its own.
    ///
    /// Not the same as a task that failed. Nothing about the task was wrong
    /// and running it again later is what it wants.
    AtVendorLimit,
}

/// How an agent finished.
///
/// Keeping what it wrote as a trace is its own issue and attaches here.
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
