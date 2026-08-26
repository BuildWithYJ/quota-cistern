//! What a task is added amid, in the repository it was added from.

/// How much of something a reader may be shown.
///
/// Two numbers, and whichever runs out first is the one that stops it. A count alone does not
/// bound anything: two hundred lines of a lock file or of one minified script is a megabyte, and
/// a repository would be deciding what adding a task to it costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Room {
    /// How many lines, or paths, at most.
    pub most: usize,
    /// How many characters at most, over all of them together.
    pub chars: usize,
}

/// What the working tree is in the middle of.
///
/// An instruction pointing at "this" points here. What the author has open but not committed,
/// what they committed before it, and the branch they are on are what they were reading when
/// they wrote the line, so they are what a reader working out the line has to be given.
pub trait Surroundings: Sync {
    /// What the working tree has changed, in full, within the room it is given.
    ///
    /// A list of filenames says which files are open and nothing about what is being done to
    /// them. What is being done to them is the whole of what "this" means, so a reader that has
    /// to work out an author's intent is given the change itself.
    ///
    /// Bounded because a working tree can hold a rewrite, and a reader that is a model is paid
    /// for by the character. Empty when nothing is uncommitted or the repository cannot be read.
    fn changes(&self, repository: &str, room: Room) -> String;

    /// What has been committed lately, one line each.
    ///
    /// An instruction that points at "the thing from earlier" points here. Empty when the
    /// repository holds no commits or cannot be read.
    fn lately(&self, repository: &str, room: Room) -> String;

    /// The branch the working tree is on, where it is on one.
    ///
    /// A name like `fix/search-dupe` carries half of what an author means and costs one command
    /// to read. Nothing on a detached head, or where the repository cannot be read.
    fn branch(&self, repository: &str) -> Option<String>;

    /// The paths the repository tracks, within the room they are given.
    ///
    /// What a place may be checked against, and what a reader is shown so that it names one that
    /// exists rather than one that would be reasonable.
    fn tracks(&self, repository: &str, room: Room) -> Vec<String>;
}
