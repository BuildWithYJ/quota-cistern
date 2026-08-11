//! What the core offers the daemon's own workers.
//!
//! Nothing here is a command.
//! A session assigns a task and something has to run it, and that something is not a person
//! waiting on an answer.

/// Why a task was not carried on.
///
/// Not a [`super::Refusal`], because nobody asked for anything.
/// A run that goes wrong is an answer rather than a failure, so what is left is a store that
/// could not be read and a task that is no longer there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotCarried {
    /// No task carries that number any more.
    NoSuchTask { id: String },
    /// The store could not be reached or could not be understood.
    Unavailable { reason: String },
}

/// Running a task a session has already assigned.
///
/// Apart from the commands because the caller is apart from them.
/// A person types `run` and is answered at once; these run for as long as a task takes, on
/// whatever thread the daemon set aside for them.
pub trait Carrying {
    /// Runs one assigned task to the end, and answers with what the decision that followed
    /// assigned next.
    ///
    /// This does not return until the task has.
    /// What comes back has to be run the same way, which is the caller's to arrange.
    fn carry_on(&self, task: &str) -> Result<Vec<String>, NotCarried>;
}
