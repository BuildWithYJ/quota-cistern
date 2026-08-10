//! What a task left on its branch, as the core asks for it.

use super::super::Unavailable;

/// Which branches a question is about.
///
/// All three cross as the text they were stored as. The core never reads any
/// of them as a place or as a revision.
pub struct Between<'a> {
    pub repository: &'a str,
    pub base: &'a str,
    pub branch: &'a str,
}

/// What one file gained and lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touched {
    pub path: String,
    pub added: String,
    pub removed: String,
}

/// What a task changed, and how far its base has moved since.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub files: Vec<Touched>,
    /// The whole of it, as a patch.
    pub patch: String,
    /// Commits the branch gained after it left the base.
    pub commits: String,
    /// Commits the base gained after the branch left it.
    pub base_ahead: String,
}

/// Why a result could not be taken up.
///
/// A repository belongs to whoever is using this, and they may commit,
/// check out, or delete anything in it between one command and the next.
/// Every one of these is that rather than a fault, so each is answered for
/// rather than raised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotApplied {
    /// The result cannot be read at all.
    ///
    /// Told apart from a branch holding nothing, since one is a branch that is
    /// gone and the other is a task that changed nothing.
    NotThere,
    /// The working tree holds changes nobody has committed.
    NotCommitted,
    /// The branch holds nothing the base does not.
    Nothing,
    /// It is in the working tree already.
    Already,
    /// It does not go in as the working tree stands.
    Conflicts { why: String },
}

/// What a task left behind, and how it is taken up.
pub trait Results: Sync {
    /// What lies between the two branches.
    ///
    /// Nothing is answered for a branch or a repository that is not there.
    /// A task whose result has been deleted is still a task, and a list of
    /// them that failed over one of them would hide the rest.
    fn changes(&self, between: Between<'_>) -> Option<Changes>;

    /// Brings those changes into the repository's working tree.
    ///
    /// Nothing is committed and no branch is moved. Whether it would go in is
    /// settled before anything is written, so a refusal leaves the working
    /// tree as it was.
    fn apply(&self, between: Between<'_>) -> Result<Vec<Touched>, NotApplied>;

    /// Whether the repository can be reached at all.
    ///
    /// Told apart from a result that is not there, since one is worth saying
    /// and the other is a task to get on with.
    fn reachable(&self, repository: &str) -> Result<(), Unavailable>;
}
