//! What a run wrote while it worked, as the core asks for it.

use super::Unavailable;

/// One thing the agent did or said.
///
/// Both fields cross as text, for the reason `outbound::backlog` gives. What
/// counts as one of these is the implementation's: the core is handed a time
/// and a line and puts them in front of whoever asked.
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
    /// Where the next read starts. Opaque to the core.
    pub cursor: String,
}

/// Where a run's trace is kept.
///
/// A run writes as it works and whoever is watching reads while it does, so
/// the two ends are here together. Nothing else in the core writes one.
pub trait Traces: Sync {
    /// Where a task's run should write.
    ///
    /// The core hands this to the agent and never looks at it. Which place
    /// that is belongs here, so that reading and writing cannot drift apart.
    fn at(&self, task: &str) -> Result<String, Unavailable>;

    /// What the run wrote after `from`, and where to carry on.
    ///
    /// `from` empty means from the start. A run may write more than anyone
    /// wants at once, so an implementation may answer with less than all of
    /// it and say so in the cursor it gives back.
    fn read(&self, task: &str, from: &str) -> Result<Read, Unavailable>;
}
