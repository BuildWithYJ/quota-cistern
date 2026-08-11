//! The backlog store, as the core asks for it.

use super::Unavailable;

/// The backlog as a store holds it.
///
/// Every field crosses as the text a user would have typed, so what comes back meets the same parse an argument does.
/// A field an adapter typed instead would be the one field refused elsewhere, with a different code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredBacklog {
    pub next_id: String,
    pub tasks: Vec<StoredTask>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTask {
    pub id: String,
    pub title: String,
    pub instruction: String,
    /// The branch that was named, if one was.
    pub branch: Option<String>,
    /// The predecessor that was named, if one was.
    pub after: Option<String>,
    pub model: Option<String>,
    pub repository: String,
    pub state: String,
    /// The session that assigned it, once one has.
    pub session: Option<String>,
    /// Where it is being worked on, once a work area was made.
    pub worktree: Option<String>,
    /// Why it ended as it did, for a task that did not simply finish.
    pub reason: Option<String>,
    /// What it consumed, once a run's answer said.
    pub consumed: Option<StoredConsumption>,
    /// Why what it consumed could not be read, when it could not.
    ///
    /// Both this and `consumed` are absent for a task that has not run.
    pub unreadable: Option<String>,
    /// What was decided about its result, once anyone decided.
    pub disposition: Option<String>,
}

/// What a task consumed, as a store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredConsumption {
    pub input: String,
    pub output: String,
    pub cache_written: String,
    pub cache_read: String,
    pub cost: String,
}

/// Where the backlog is kept between runs.
pub trait BacklogStore: Sync {
    /// Reads what is stored.
    ///
    /// Nothing stored is an empty backlog rather than a failure.
    /// `backlog` has to answer before anyone has registered a task.
    fn load(&self) -> Result<StoredBacklog, Unavailable>;

    /// Reads what is stored, hands it to `change`, and writes back what `change` left behind.
    ///
    /// One call rather than two.
    /// Two tasks ending at the same moment do not each write over what the other recorded.
    /// `change` answers whether it changed anything, and nothing is written when it did not.
    fn update(&self, change: &mut dyn FnMut(&mut StoredBacklog) -> bool)
    -> Result<(), Unavailable>;
}
