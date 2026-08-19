//! What each run of a task cost, kept one line at a time.

use super::{StoredConsumption, Unavailable};

/// One run of one task, as the ledger holds it.
///
/// A task runs more than once when the vendor turns it away, and each run costs what it costs.
/// The task itself keeps only the most recent, which is what `task show` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub task: String,
    /// The session that assigned it, which a task on its way back to waiting no longer names.
    pub session: Option<String>,
    /// The model that was asked for, when the task or the session named one.
    pub model: Option<String>,
    /// When the run started and when it stopped, in seconds since the epoch.
    pub started_at: String,
    pub ended_at: String,
    /// The state the run left the task in.
    pub outcome: String,
    /// Why it ended as it did, for a run that did not simply finish.
    pub reason: Option<String>,
    /// What it consumed, once its answer said.
    pub spent: Option<StoredConsumption>,
    /// Why what it consumed could not be read, when it could not.
    ///
    /// Both this and `spent` are absent for a run that reported nothing at all.
    pub unreadable: Option<String>,
    /// How far the vendor's limit was spent when the run started and when it stopped, in
    /// hundredths of a percent.
    ///
    /// The difference is what the run cost in the unit a share is declared in. Tokens say what
    /// a run cost in the other unit, and neither converts to the other on its own: how much of
    /// the limit a token moves is the vendor's to decide and differs between subscriptions.
    ///
    /// Both are readings the session already took -- one when the run before it ended, one
    /// when this one did -- so writing them down costs no further asking. Absent for a session
    /// declared in tokens, which never asks, and for the first run of a session, which has
    /// nothing before it.
    pub limit_before: Option<String>,
    pub limit_after: Option<String>,
}

/// Every run there has been, in the order they ended.
///
/// Appended and never rewritten, so a second run of a task does not displace the first.
/// What a budget is worked out from is read from here rather than from the backlog, which holds
/// one run per task and cannot hold two.
pub trait Runs: Sync {
    /// Puts one run at the end.
    fn append(&self, run: Run) -> Result<(), Unavailable>;
}
