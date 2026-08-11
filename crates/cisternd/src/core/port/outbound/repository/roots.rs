//! Which repository a place belongs to.

use super::super::Unavailable;

/// Answers which repository a directory sits in.
pub trait RepositoryRoots: Sync {
    /// The repository the given place belongs to, or nothing when it belongs to none.
    ///
    /// The core reads neither the argument nor the answer.
    /// What marks a repository, and walking upward to find one, are the implementation's.
    fn root_of(&self, from: &str) -> Result<Option<String>, Unavailable>;
}
