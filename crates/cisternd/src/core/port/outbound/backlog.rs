//! The backlog store, as the core asks for it.

use super::Unavailable;

/// The backlog as a store holds it.
///
/// Every field crosses as the text a user would have typed, whatever a store
/// kept it as, so what comes back meets the same parse an argument does. A task
/// carries more fields than the configuration and this is longer for it, but
/// one rule is worth the length: a field an adapter typed instead would be the
/// one field refused somewhere else, with a different code.
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
}

/// Where the backlog is kept between runs.
pub trait BacklogStore {
    /// Reads what is stored.
    ///
    /// Nothing stored is an empty backlog rather than a failure, since
    /// `backlog` has to answer before anyone has registered a task. Telling
    /// that apart from a store that cannot be read is the implementation's job.
    fn load(&self) -> Result<StoredBacklog, Unavailable>;

    /// Writes the whole backlog.
    ///
    /// The whole of it, so that adding one task stays in the core and the
    /// implementation never learns the shape.
    fn store(&self, backlog: &StoredBacklog) -> Result<(), Unavailable>;
}
