//! Whether what a spec says about a repository is so.

use std::time::Duration;

/// What running a success condition said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ran {
    /// It failed, which is what a task worth running looks like before it is done.
    Failed,
    /// It passed already, so either the work is done or this does not tell that it is not.
    Passed,
    /// It could not be run, or did not finish inside the time it was given.
    Unknown,
}

/// Whether a spec names things that are there.
///
/// A gate that reads the shape of the words guesses; this asks. `2026/08/26` reads as a path and
/// names no file, and the difference between those two is one question to the repository.
pub trait Grounding: Sync {
    /// How many files a place reaches, or nothing where it names none at all.
    ///
    /// A file reaches one, a directory reaches what it holds, and a name nothing tracks reaches
    /// nothing -- which is how a place that was invented is told from a place that is.
    fn reaches(&self, repository: &str, place: &str) -> Option<usize>;

    /// Whether a success condition names something this machine could run.
    ///
    /// Asked before running it, because a command that does not exist is the model's mistake and
    /// a command that fails is the task's whole point.
    fn runnable(&self, repository: &str, command: &str) -> bool;

    /// Runs it once, in the repository, and says how it went.
    ///
    /// Once, at the moment a task is registered, and never again: this is the one check that says
    /// whether the task has anything to do. A command still going when the time is up is stopped
    /// and answers [`Ran::Unknown`], since a run of the gate is not a run of the work.
    fn run(&self, repository: &str, command: &str, within: Duration) -> Ran;
}
