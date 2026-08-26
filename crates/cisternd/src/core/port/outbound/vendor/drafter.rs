//! A model asked to work out what a loose instruction meant, without running the work.

/// What a loose instruction is read amid, for a model to work from.
///
/// The names of the files that are open say which files are open and nothing about what is being
/// done to them, and what is being done to them is the whole of what "this" means. So the change
/// itself is here, and what was committed lately, and the branch: an author writing one line is
/// leaning on all of it, and a reader that cannot see it is guessing where they were reading.
pub struct Draft<'a> {
    /// What the author typed.
    pub instruction: &'a str,
    /// What the working tree has changed, body and all, already capped.
    pub changes: &'a str,
    /// What was committed lately, one line each with what it touched.
    pub lately: &'a str,
    /// The branch the tree is on, where it is on one.
    pub branch: Option<&'a str>,
    /// The paths the repository tracks, already capped.
    pub tracks: &'a [String],
    /// The repository the task was added from, for the model to look in.
    pub repository: &'a str,
}

/// One part of a spec as a model proposed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposed {
    pub said: String,
    /// What it was drawn from, for the author to judge it by rather than take on trust.
    pub drawn_from: Option<String>,
    /// The others the repository allows, where the model found the question open.
    pub others: Vec<String>,
}

/// What a model proposed a spec should say, part by part.
///
/// Every part is optional, and a part it left out is one nobody has settled. A model that cannot
/// tell is meant to leave the part alone rather than fill it with something that reads well: the
/// count of what is left is what the gate is, and a part filled to get past it defeats the point.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drafted {
    pub goal: Option<Proposed>,
    pub place: Option<Proposed>,
    pub success: Option<Proposed>,
    pub on_failure: Option<Proposed>,
    pub why: Option<Proposed>,
    pub scope: Option<Proposed>,
}

/// A model that reads a loose instruction amid its repository and proposes what a spec should say.
///
/// It only reads, and only proposes. Every part it proposes is checked against the repository
/// before a run is given anything, so a wrong guess is a question asked, not a run misspent.
pub trait Drafter: Sync {
    /// Proposes what the spec should say, or nothing when the model cannot be reached.
    fn draft(&self, ask: Draft<'_>) -> Option<Drafted>;

    /// The same, told what did not hold up, so that it may answer again.
    ///
    /// A check the repository failed is the model's to fix rather than the author's: it named a
    /// file that is not there, or a command nothing can run, and it is the one that can look
    /// again. Only what survives a second answer is worth anybody's attention.
    fn draft_again(&self, ask: Draft<'_>, held: &Drafted, amiss: &[String]) -> Option<Drafted>;
}
