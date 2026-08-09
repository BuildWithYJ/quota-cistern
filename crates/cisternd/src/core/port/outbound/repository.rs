//! Which repository a place belongs to.

use super::Unavailable;

/// Answers which repository a directory sits in.
pub trait RepositoryRoots: Sync {
    /// The repository the given place belongs to, or nothing when it belongs
    /// to none.
    ///
    /// The core reads neither the argument nor the answer. A surface says where
    /// it was run and that is passed through; what comes back is kept on the
    /// task and handed back unchanged. Walking upward, and knowing what marks a
    /// repository, are the implementation's.
    ///
    /// The core cannot read its own working directory for this. It runs as a
    /// daemon, so where it was started is not where the user typed the command.
    fn root_of(&self, from: &str) -> Result<Option<String>, Unavailable>;
}
