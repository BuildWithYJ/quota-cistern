//! Work areas, as git makes them.
//!
//! The only place that knows the command and where a work area lands. Neither
//! reaches the core.

use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{Command, Output},
};

use crate::core::port::outbound::{Cut, Unavailable, Worktrees};

/// Work areas kept under a directory fixed when this is built.
///
/// They sit beside the backlog rather than inside the repository, so a task
/// leaves nothing in the working tree a person is looking at.
pub struct GitWorktrees {
    under: PathBuf,
}

impl GitWorktrees {
    /// Takes the directory it is given. This is how a test reaches a temporary
    /// one.
    pub fn under(under: PathBuf) -> Self {
        GitWorktrees { under }
    }

    /// The directory beside the backlog, or nothing when there is nowhere
    /// for it.
    pub fn in_data_home() -> Option<Self> {
        under_of(env::var_os("XDG_DATA_HOME"), env::var_os("HOME")).map(GitWorktrees::under)
    }
}

impl Worktrees for GitWorktrees {
    fn prepare(&self, cut: Cut<'_>) -> Result<String, Unavailable> {
        let at = self.under.join(cut.task);
        let at = at.display().to_string();

        // One command cuts the branch and checks it out, so a failure part of
        // the way through leaves neither behind.
        let done = Command::new("git")
            .args(["-C", cut.repository, "worktree", "add", "-b", cut.branch])
            .args([&at, cut.base])
            .output()
            .map_err(|e| Unavailable::new(format!("git worktree add: {e}")))?;

        match done.status.success() {
            true => Ok(at),
            false => Err(Unavailable::new(format!(
                "git could not make a work area for {} on {}: {}",
                cut.task,
                cut.branch,
                said(&done)
            ))),
        }
    }
}

/// `$XDG_DATA_HOME/cistern/worktrees`, or `~/.local/share/cistern/worktrees`.
fn under_of(data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match data_home {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home?).join(".local").join("share"),
    };
    Some(base.join("cistern").join("worktrees"))
}

/// What git said about a command that failed.
///
/// git writes its complaint to standard error and says nothing on standard
/// output, but a command that fails without a word is worse than one that
/// repeats itself, so both are read.
fn said(done: &Output) -> String {
    let complained = String::from_utf8_lossy(&done.stderr);
    let complained = complained.trim();
    match complained.is_empty() {
        false => complained.to_owned(),
        true => String::from_utf8_lossy(&done.stdout).trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn some(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    /// A repository with one commit on `main`, which is what a task starts
    /// from.
    fn a_repository() -> TempDir {
        let dir = TempDir::new().unwrap();
        let at = dir.path();
        run(at, &["init", "--initial-branch", "main"]);
        run(at, &["config", "user.email", "nobody@example.com"]);
        run(at, &["config", "user.name", "nobody"]);
        fs::write(at.join("README.md"), "a repository\n").unwrap();
        run(at, &["add", "README.md"]);
        run(at, &["commit", "-m", "first"]);
        dir
    }

    fn run(at: &std::path::Path, args: &[&str]) {
        let done = Command::new("git")
            .arg("-C")
            .arg(at)
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}: {}", said(&done));
    }

    fn cutting<'a>(repository: &'a str, task: &'a str, branch: &'a str) -> Cut<'a> {
        Cut {
            repository,
            base: "main",
            branch,
            task,
        }
    }

    #[test]
    fn the_data_directory_wins() {
        assert_eq!(
            under_of(some("/x/.share"), some("/home/a")),
            Some(PathBuf::from("/x/.share/cistern/worktrees"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_data_directory() {
        assert_eq!(
            under_of(None, some("/home/a")),
            Some(PathBuf::from("/home/a/.local/share/cistern/worktrees"))
        );
    }

    #[test]
    fn neither_leaves_nowhere_to_put_them() {
        assert_eq!(under_of(None, None), None);
    }

    #[test]
    fn a_work_area_holds_the_branch_that_was_cut() {
        let repository = a_repository();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        let at = worktrees
            .prepare(cutting(
                &repository.path().display().to_string(),
                "1",
                "cistern/1",
            ))
            .unwrap();

        assert!(PathBuf::from(&at).join("README.md").exists());
        let on = Command::new("git")
            .args(["-C", &at, "rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&on.stdout).trim(), "cistern/1");
    }

    /// Two tasks running at once each get their own, which is what keeps one
    /// from writing over the other.
    #[test]
    fn two_tasks_land_in_two_places() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        let first = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();
        let second = worktrees.prepare(cutting(&at, "2", "cistern/2")).unwrap();

        assert_ne!(first, second);
        assert!(PathBuf::from(&first).exists());
        assert!(PathBuf::from(&second).exists());
    }

    #[test]
    fn a_base_branch_that_is_not_there_fails_and_says_so() {
        let repository = a_repository();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        let refused = worktrees
            .prepare(Cut {
                repository: &repository.path().display().to_string(),
                base: "no-such-branch",
                branch: "cistern/1",
                task: "1",
            })
            .unwrap_err();

        assert!(refused.reason.contains("cistern/1"), "{}", refused.reason);
    }
}
