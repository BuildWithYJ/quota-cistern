//! What a spec leaves for the agent to decide on its own.
//!
//! An unattended run cannot stop to ask, so a decision the spec does not make is one the agent
//! makes without anybody seeing it, and every unattended accident is one of those: a run that
//! cannot tell whether it is done judges its own work, and one that was told nothing about
//! failing invents a way around -- it edits the test.
//!
//! "Detailed enough" cannot be measured. This can: **how many decisions are left**. A spec that
//! leaves none is one a run can be given.
//!
//! Nothing here reads a repository. What the repository says is asked by whoever can and handed
//! in as [`Grounded`], so that the rules are the same wherever they are read and can be held to
//! by a test without a repository to read.

// Nothing counts a spec yet. The gate is rebuilt on this over the commits that follow, and this
// comes off, with `core::domain` re-exporting it, when the service reads a count.
#![allow(dead_code)]

use super::spec::{Named, Settled, Spec};

/// How many files a place may reach before which of them to touch is itself a decision.
///
/// A first value. One file is a place; a directory of two hundred is a search, and what a run
/// would do with it is decide for itself where to stop.
pub const REACHES_AT_MOST: usize = 10;

/// What the repository said about the parts of a spec that name something in it.
///
/// The domain reads no repository. These are the answers, gathered by whoever can, so that the
/// rules stay rules: the same question asked twice gets the same answer, and a test can put the
/// answer in by hand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grounded {
    /// How many files the place reaches, or nothing where it reaches none at all.
    pub files: Option<usize>,
    /// Whether the success condition names something this machine can actually run.
    pub runnable: bool,
}

/// One decision the spec leaves for the agent.
///
/// Each says what would be settled without anybody seeing it, because that is what an author has
/// to be told: not that a field is empty, but what an agent would go and do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undecided {
    /// Nobody settled a part at all.
    Unsettled(Named),
    /// A part was marked as drawn from the repository and is the author's own words unchanged.
    ///
    /// Nothing was drawn. A model that could not tell what was meant and answered by repeating
    /// the question has left the same decision behind, so it is counted rather than taken.
    Echoed(Named),
    /// The place reaches more files than a run should be choosing between.
    Reaches { files: usize },
    /// Whether the work is done cannot be told by running anything.
    Unverifiable,
}

impl Undecided {
    /// What the agent would settle for itself while this stands.
    pub fn left_to_decide(&self) -> String {
        match self {
            Undecided::Unsettled(named) => named.left_to_decide().to_owned(),
            Undecided::Echoed(named) => {
                format!("{} (nothing was worked out)", named.left_to_decide())
            }
            Undecided::Reaches { files } => {
                format!("how far to reach, over {files} files")
            }
            Undecided::Unverifiable => "whether it is done".to_owned(),
        }
    }
}

/// Every decision the spec leaves for the agent, in the order the parts are read in.
///
/// `wrote` is what the author typed, which is what an inference has to have got past to be one.
pub fn left_to_decide(spec: &Spec, wrote: &str, grounded: Grounded) -> Vec<Undecided> {
    let mut left = Vec::new();

    for (named, part) in spec.parts() {
        if part.is_open() {
            // `why` is read after a run rather than by one, so wanting it warns rather than
            // holding the run back. `Spec::undecided` is where that is decided.
            if spec.undecided().contains(&named) {
                left.push(Undecided::Unsettled(named));
            }
            continue;
        }
        let said = part.said.as_deref().unwrap_or_default();
        if part.settled == Settled::Inferred && said.trim() == wrote.trim() {
            left.push(Undecided::Echoed(named));
        }
    }

    // Asked of the place only where a place was settled at all. One nobody settled is already
    // counted above, and saying it twice would have an author answer the same question twice.
    if !spec.place.is_open()
        && let Some(files) = grounded.files
        && files > REACHES_AT_MOST
    {
        left.push(Undecided::Reaches { files });
    }

    if !spec.success.is_open() && !grounded.runnable {
        left.push(Undecided::Unverifiable);
    }

    left
}

#[cfg(test)]
mod tests;
