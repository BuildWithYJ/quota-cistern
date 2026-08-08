//! Where a repository begins.
//!
//! The only place that knows what marks one. That knowledge does not reach the
//! core.

use std::path::{Path, PathBuf};

use crate::core::port::outbound::{RepositoryRoots, Unavailable};

/// Finds the repository a directory sits in by looking for `.git` above it.
pub struct GitRoots;

impl RepositoryRoots for GitRoots {
    fn root_of(&self, from: &str) -> Result<Option<String>, Unavailable> {
        // Resolved first, so that what is stored does not depend on where the
        // surface happened to be when it built the request.
        let start =
            std::fs::canonicalize(from).map_err(|e| Unavailable::new(format!("{from}: {e}")))?;
        Ok(root_above(&start).map(|root| root.display().to_string()))
    }
}

/// The nearest directory at or above this one that holds `.git`.
///
/// `.git` is a file rather than a directory in a worktree and in a submodule.
/// Either way the directory is a repository, so what it is does not matter
/// here. Worktrees are how each task will get a work area of its own, so this
/// is not a corner case for long.
fn root_above(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// A directory with `.git` in it. git is never run, so nothing has to be a
    /// real repository for this to be answered.
    fn a_repository() -> TempDir {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    fn root_of(dir: &Path) -> Option<String> {
        GitRoots.root_of(&dir.display().to_string()).unwrap()
    }

    fn resolved(dir: &Path) -> String {
        fs::canonicalize(dir).unwrap().display().to_string()
    }

    #[test]
    fn a_directory_below_the_root_answers_with_the_root() {
        let dir = a_repository();
        let below = dir.path().join("src").join("core");
        fs::create_dir_all(&below).unwrap();
        assert_eq!(root_of(&below), Some(resolved(dir.path())));
    }

    #[test]
    fn the_root_answers_with_itself() {
        let dir = a_repository();
        assert_eq!(root_of(dir.path()), Some(resolved(dir.path())));
    }

    #[test]
    fn a_marker_that_is_a_file_counts_as_much_as_a_directory() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(root_of(dir.path()), Some(resolved(dir.path())));
    }

    /// A temporary directory is not inside a repository, which is what makes it
    /// usable for this.
    #[test]
    fn somewhere_that_belongs_to_no_repository_answers_with_nothing() {
        let dir = TempDir::new().unwrap();
        assert_eq!(root_of(dir.path()), None);
    }

    #[test]
    fn a_place_that_is_not_there_is_a_failure_rather_than_an_answer() {
        assert!(GitRoots.root_of("/no/such/place").is_err());
    }
}
