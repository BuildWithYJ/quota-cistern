//! What the core needs from outside, in the core's own terms.
//!
//! A port says what is wanted, never how it is reached. No path, file format,
//! or vendor name appears here.
//!
//! One file per outside. `Unavailable` stands here rather than in one of them,
//! because reaching something outside is what all of them do.
//!
//! A port trait has to stay usable behind `dyn`, which rules out returning
//! `impl Trait` from one. Nothing needs it while a service takes the ports it
//! uses as arguments, but the ports will be gathered into one value once there
//! are enough of them, and that value can only hold them as `dyn` references
//! without a type parameter per port. Honouring the constraint from the start
//! costs nothing; adopting it later would mean designing a trait twice.

mod repository;
mod settings;
mod tasks;

pub use repository::Repository;
pub use settings::{Settings, Stored};
pub use tasks::{StoredBacklog, StoredTask, Tasks};

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
