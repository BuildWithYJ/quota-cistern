//! What the core needs from outside, in the core's own terms.
//!
//! A port says what is wanted, never how it is reached. No path, file format,
//! or vendor name appears here.
//!
//! One file per outside. `Unavailable` stands here rather than in one of them,
//! because reaching something outside is what all of them do.

mod settings;

pub use settings::{Settings, Stored};

/// The outside could not be reached or could not be understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unavailable {
    /// What went wrong, in the words of whatever failed.
    pub reason: String,
}

impl Unavailable {
    pub fn new(reason: impl Into<String>) -> Self {
        Unavailable {
            reason: reason.into(),
        }
    }
}
