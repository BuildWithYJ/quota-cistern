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
    /// Whether it passes already, which says the work is done or that this does not tell it.
    pub already: bool,
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
    /// The place names nothing the repository holds.
    ///
    /// It was invented. `2026/08/26` reads as a path by every rule of shape there is and holds
    /// no file, and an agent sent there would go and find somewhere else to work.
    Nowhere,
    /// The place reaches more files than a run should be choosing between.
    Reaches { files: usize },
    /// Whether the work is done cannot be told by running anything.
    Unverifiable,
    /// What would say the work is done says so already.
    ///
    /// Either it is done and there is nothing to run, or that command does not tell that it is
    /// not, and an agent handed it would stop at once and report success. Which of the two it is
    /// is not the repository's to say.
    AlreadyDone,
}

impl Undecided {
    /// Which part it is about, where it is about one.
    pub fn part(&self) -> Option<Named> {
        match self {
            Undecided::Unsettled(named) | Undecided::Echoed(named) => Some(*named),
            Undecided::Nowhere | Undecided::Reaches { .. } => Some(Named::Place),
            Undecided::Unverifiable | Undecided::AlreadyDone => Some(Named::DoneWhen),
        }
    }

    /// Whether it is the model's to answer for rather than the author's.
    ///
    /// A part that names a file which is not there, or a command nothing can run, is a mistake
    /// the model made against a repository it can look at again, so it is worth asking again.
    ///
    /// A part nobody settled is not. The model was given everything there is and left it empty,
    /// which is it saying it cannot tell; asking the same question again buys a second answer to
    /// the same question, and a model asked twice whether it is sure answers whatever it thinks
    /// is wanted. It is the author who knows, which is why they are asked.
    pub fn is_the_models(&self) -> bool {
        match self {
            Undecided::Unsettled(_) => false,
            Undecided::Echoed(_)
            | Undecided::Nowhere
            | Undecided::Reaches { .. }
            | Undecided::Unverifiable
            | Undecided::AlreadyDone => true,
        }
    }

    /// Which kind it is, in a word a surface can answer in its own language.
    ///
    /// [`Undecided::left_to_decide`] says it in English, which is one language and this file's
    /// rather than a reader's. A surface that has a person in front of it says it in theirs, and
    /// what it needs to do that is which of these it is.
    pub fn kind(&self) -> &'static str {
        match self {
            Undecided::Unsettled(_) => "unsettled",
            Undecided::Echoed(_) => "echoed",
            Undecided::Nowhere => "nowhere",
            Undecided::Reaches { .. } => "reaches",
            Undecided::Unverifiable => "unverifiable",
            Undecided::AlreadyDone => "already",
        }
    }

    /// How many files the place reaches, where that is what is wrong with it.
    pub fn files(&self) -> Option<usize> {
        match self {
            Undecided::Reaches { files } => Some(*files),
            _ => None,
        }
    }

    /// What the agent would settle for itself while this stands.
    pub fn left_to_decide(&self) -> String {
        match self {
            Undecided::Unsettled(named) => named.left_to_decide().to_owned(),
            Undecided::Echoed(named) => {
                format!("{} (nothing was worked out)", named.left_to_decide())
            }
            Undecided::Nowhere => "where the work is, since nothing is there".to_owned(),
            Undecided::Reaches { files } => {
                format!("how far to reach, over {files} files")
            }
            Undecided::Unverifiable => "whether it is done".to_owned(),
            Undecided::AlreadyDone => "whether there is anything to do".to_owned(),
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
    if !spec.place.is_open() {
        match grounded.files {
            None | Some(0) => left.push(Undecided::Nowhere),
            Some(files) if files > REACHES_AT_MOST => left.push(Undecided::Reaches { files }),
            Some(_) => {}
        }
    }

    if !spec.done_when.is_open() {
        match (grounded.runnable, grounded.already) {
            (false, _) => left.push(Undecided::Unverifiable),
            (true, true) => left.push(Undecided::AlreadyDone),
            (true, false) => {}
        }
    }

    left
}

#[cfg(test)]
mod tests;
