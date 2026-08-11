//! The session store, as the core asks for it.

use super::Unavailable;

/// The sessions as a store holds them.
///
/// Every field crosses as the text a user would have typed, whatever a store kept it as.
/// The reason is the one `outbound::backlog` gives.
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
    /// When it opened, in seconds since the epoch.
    /// A time limit runs from here.
    pub started_at: String,
    /// How much of the vendor's limit was spent when it opened.
    ///
    /// A share declares what this session may add to that.
    /// Absent for a session declared in tokens.
    pub limit_at_start: Option<String>,
    /// What it has consumed, in the unit it was declared in.
    ///
    /// Kept rather than worked out again.
    /// A share is how far the vendor's limit moved, and a session that stopped an hour ago can no longer be measured.
    pub consumed: String,
    /// When it last changed, in seconds since the epoch.
    pub updated_at: String,
    /// When the vendor's limit starts over, in seconds since the epoch.
    ///
    /// Kept only for a session the vendor turned away, which is the one case section 2.2 reports it for.
    pub resets_at: Option<String>,
}

/// Where the sessions are kept between runs.
///
/// Apart from the backlog, which is written at other moments by other work.
pub trait SessionStore: Sync {
    /// Reads what is stored, hands it to `change`, and writes back what `change` left behind.
    /// The reason is the one `BacklogStore::update` gives.
    fn update(
        &self,
        change: &mut dyn FnMut(&mut StoredSessions) -> bool,
    ) -> Result<(), Unavailable>;
}
