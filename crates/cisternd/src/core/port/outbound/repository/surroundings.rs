//! What a task is added amid, in the repository it was added from.

/// What the working tree is in the middle of.
///
/// When an instruction points at "this" or names no place, what the author has open but not
/// committed is the likeliest thing they mean.
pub trait Surroundings: Sync {
    /// The files the working tree has changed and not committed, the nearest work first.
    ///
    /// Empty when nothing is uncommitted, and empty again when the repository cannot be read:
    /// neither is a place to send a run, so both come back the same.
    fn changed(&self, repository: &str) -> Vec<String>;

    /// The files the repository holds that mention the word, by a line of code or by name.
    ///
    /// What an instruction points at without editing: the word it used, found where the code uses
    /// it. Empty when nothing matches, or when the repository cannot be read.
    fn holding(&self, repository: &str, word: &str) -> Vec<String>;
}
