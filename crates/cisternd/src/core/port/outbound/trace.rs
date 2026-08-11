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

/// Somewhere to put what a run writes, a line at a time.
///
/// A value rather than a path, so that whoever writes the lines cannot open the file and
/// learn what a line is kept as.
pub type Keeping = Box<dyn FnMut(&str) + Send>;

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
    /// Somewhere to put what a run writes, a line at a time.
    ///
    /// The core hands this to the agent, which knows a line when it sees one and nothing else.
    /// How a line is kept and how it is read back stay together here.
    fn keeping(&self, task: &str) -> Result<Keeping, Unavailable>;

    /// What the run wrote after `from`, and where to carry on.
    ///
    /// `from` empty means from the start.
    /// An implementation may answer with less than all of it and say so in the cursor it gives back.
    fn read(&self, task: &str, from: &str) -> Result<Read, Unavailable>;
}
