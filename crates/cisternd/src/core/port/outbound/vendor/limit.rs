//! How much of the vendor's limit has been spent, as the core asks for it.

use super::super::Unavailable;

/// What the vendor says about the limit a session runs against.
///
/// Both fields cross as the text a user would have typed, for the reason `outbound::backlog` gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// How much of the limit is spent, in hundredths of a percent.
    ///
    /// Finer than the percentage a person is shown.
    /// One task moves the limit by less than a point, and a budget divided by nothing is one nobody can plan against.
    pub used: String,
    /// When the limit starts over, in seconds since the epoch.
    pub resets_at: String,
}

/// What the vendor's limit is at.
///
/// A reading rather than a total: asking twice gives two readings, not two amounts to add up.
/// What one task consumed is not here; it comes back with the task, on [`super::Agent`].
pub trait Limit: Sync {
    /// Asks the vendor where its limit stands.
    fn read(&self) -> Result<Reading, Unavailable>;
}
