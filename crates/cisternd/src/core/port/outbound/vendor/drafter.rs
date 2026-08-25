//! A model asked what a loose instruction is missing, without running the work.

/// What a loose instruction is read amid, for a model to propose from.
pub struct Draft<'a> {
    pub instruction: &'a str,
    /// The files the working tree has changed, the likeliest place already narrowed.
    pub changed: &'a [String],
    /// The repository the task was added from, for the model to look in.
    pub repository: &'a str,
}

/// What a model proposed a loose instruction is missing.
pub struct Drafted {
    /// A place to work, when the model found one.
    pub place: Option<String>,
    /// A way to tell the work is done, when the model could give one.
    pub check: Option<String>,
}

/// A model that reads a loose instruction and proposes what it does not say.
///
/// It only reads, and only proposes. What it proposes is written into the instruction and checked
/// by rule before a run is given it, so a wrong guess is a task turned back, not a run misspent.
pub trait Drafter: Sync {
    /// Proposes what the instruction is missing, or nothing when it cannot, or cannot be reached.
    fn draft(&self, ask: Draft<'_>) -> Option<Drafted>;
}
