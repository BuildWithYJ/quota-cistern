//! What a run wrote while it worked, as the core asks for it.

use super::Unavailable;

/// One thing the agent did or said.
///
/// What counts as one is the implementation's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// When it happened, in seconds since the epoch.
    pub at: String,
    pub said: String,
}

/// A stretch of a run's trace, and where to carry on from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub events: Vec<Event>,
    /// Where the next read starts.
    /// Opaque to the core.
    pub cursor: String,
}

/// Where a run's trace is kept.
///
/// A run writes as it works and whoever is watching reads while it does, so both ends are here.
pub trait Traces: Sync {
    /// Where a task's run should write.
    ///
    /// The core hands this to the agent and never looks at it.
    fn at(&self, task: &str) -> Result<String, Unavailable>;

    /// What the run wrote after `from`, and where to carry on.
    ///
    /// `from` empty means from the start.
    /// An implementation may answer with less than all of it and say so in the cursor it gives back.
    fn read(&self, task: &str, from: &str) -> Result<Read, Unavailable>;
}
