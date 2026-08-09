//! The session store, as the core asks for it.

use super::Unavailable;

/// The sessions as a store holds them.
///
/// Every field crosses as the text a user would have typed, whatever a store
/// kept it as, for the reason `outbound::backlog` gives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredSessions {
    pub next_id: String,
    pub sessions: Vec<StoredSession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSession {
    pub id: String,
    pub state: String,
    /// Why it stopped, for a session that has.
    pub stopped_reason: Option<String>,
    /// What `--usage` was given, as it was written.
    pub usage: String,
    /// What `--time` was given, as it was written.
    pub time: String,
    pub model: Option<String>,
}

/// Where the sessions are kept between runs.
///
/// Sessions and the backlog are kept apart because they are written at
/// different moments by different work: the backlog changes when a user
/// registers something, and a session changes while one runs.
pub trait SessionStore {
    /// Reads what is stored, hands it to `change`, and writes back what
    /// `change` left behind, for the reason `BacklogStore::update` gives.
    fn update(
        &self,
        change: &mut dyn FnMut(&mut StoredSessions) -> bool,
    ) -> Result<(), Unavailable>;
}
