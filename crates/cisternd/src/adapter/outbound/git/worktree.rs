//! Work areas, as git makes them.
//!
//! The only place that knows the command and where a work area lands.
//! Neither reaches the core.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use crate::core::port::outbound::{Cut, Unavailable, Worktrees};

use super::said;

/// Work areas kept under a directory fixed when this is built.
///
/// They sit beside the backlog rather than inside the repository.
/// A task leaves nothing in the working tree a person is looking at.
pub struct GitWorktrees {
    under: PathBuf,
}

impl GitWorktrees {
    /// Takes the directory it is given.
    /// This is how a test reaches a temporary one.
    pub fn under(under: PathBuf) -> Self {
        GitWorktrees { under }
    }

    /// The directory beside the backlog, or nothing when there is nowhere for it.
    pub fn in_data_home() -> Option<Self> {
        under_of(env::var_os("XDG_DATA_HOME"), env::var_os("HOME")).map(GitWorktrees::under)
    }
}

impl GitWorktrees {
    /// The branch a work area already has checked out, or nothing where there is no work area.
    /// The repository a work area belongs to, as git names it.
    ///
    /// The common directory rather than the path a caller wrote, so that two ways of naming one
    /// repository answer alike.
    fn belongs_to(&self, at: &Path) -> Option<String> {
        let done = git(&[
            "-C",
            &at.display().to_string(),
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ])
        .ok()?;
        done.status
            .success()
            .then(|| String::from_utf8_lossy(&done.stdout).trim().to_owned())
    }

    fn on(&self, at: &Path) -> Option<String> {
        if !at.exists() {
            return None;
        }
        let done = git(&["-C", &at.display().to_string(), "branch", "--show-current"]).ok()?;
        done.status
            .success()
            .then(|| String::from_utf8_lossy(&done.stdout).trim().to_owned())
    }

    /// Whether the repository already holds the branch a task's result is kept on.
    fn holds(&self, repository: &str, branch: &str) -> bool {
        git(&[
            "-C",
            repository,
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .is_ok_and(|done| done.status.success())
    }
}

impl GitWorktrees {
    /// Whether a work area is a checkout of this repository.
    ///
    /// Answered by git rather than by comparing the paths a caller wrote, since a repository
    /// reached through a link or through a relative path is the same repository.
    fn of(&self, repository: &str, at: &Path) -> bool {
        match (self.belongs_to(at), self.belongs_to(Path::new(repository))) {
            (Some(theirs), Some(ours)) => theirs == ours,
            // A repository git will not answer for is one nothing can be checked against, and
            // the command that follows fails on it anyway.
            _ => false,
        }
    }
}

impl Worktrees for GitWorktrees {
    fn prepare(&self, cut: Cut<'_>) -> Result<String, Unavailable> {
        let at = self.under.join(cut.task);
        let at = at.display().to_string();

        // A task the vendor turned away runs again, and section 2.4 keeps its branch either way.
        // The work area it ran in is where that branch is checked out, so a second run carries on
        // in it rather than being refused for finding its own work behind.
        //
        // Which task a work area is for is its directory name, and a task number belongs to one
        // backlog rather than to one repository. Two repositories can hold a task of that number
        // and a branch of that name, so the branch alone does not say this work area is this
        // task's: the repository has to answer as well, or a run would be handed the other
        // repository's checkout and change it.
        match self.on(Path::new(&at)) {
            Some(on) if on == cut.branch && self.of(cut.repository, Path::new(&at)) => {
                return Ok(at);
            }
            Some(on) if on == cut.branch => {
                return Err(Unavailable::new(format!(
                    "the work area for {} is a checkout of another repository, not of {}",
                    cut.task, cut.repository
                )));
            }
            Some(on) => {
                return Err(Unavailable::new(format!(
                    "the work area for {} is on {on}, not on {}",
                    cut.task, cut.branch
                )));
            }
            None => {}
        }

        // A work area removed from the disk is still registered until this is run.
        let _ = git(&["-C", cut.repository, "worktree", "prune"]);

        // One command cuts the branch and checks it out, so a failure part of the way through leaves neither behind.
        // A branch that is already there is checked out rather than cut again.
        let made = match self.holds(cut.repository, cut.branch) {
            true => git(&["-C", cut.repository, "worktree", "add", &at, cut.branch]),
            false => git(&[
                "-C",
                cut.repository,
                "worktree",
                "add",
                "-b",
                cut.branch,
                &at,
                cut.base,
            ]),
        };
        let done = made.map_err(|e| Unavailable::new(format!("git worktree add: {e}")))?;

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

    fn remove(&self, repository: &str, at: &str) -> Result<(), Unavailable> {
        // Registered but not on the disk is what somebody removing one by hand leaves, and git
        // will not remove what it cannot find. Pruning first answers for that one too.
        let _ = git(&["-C", repository, "worktree", "prune"]);
        if !Path::new(at).exists() {
            return Ok(());
        }

        // No `--force`. Git refuses a work area holding changes nobody committed, and that
        // refusal is the guard: what a run left uncommitted is in no branch.
        let done = git(&["-C", repository, "worktree", "remove", at])
            .map_err(|e| Unavailable::new(format!("git worktree remove: {e}")))?;

        match done.status.success() {
            true => Ok(()),
            false => Err(Unavailable::new(said(&done))),
        }
    }
}

fn git(args: &[&str]) -> std::io::Result<Output> {
    Command::new("git").args(args).output()
}

/// `$XDG_DATA_HOME/cistern/worktrees`, or `~/.local/share/cistern/worktrees`.
fn under_of(data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match data_home {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home?).join(".local").join("share"),
    };
    Some(base.join("cistern").join("worktrees"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn some(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    /// A repository with one commit on `main`, which is what a task starts from.
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

    /// Two tasks running at once each get their own, which is what keeps one from writing over the other.
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

    /// A task the vendor turned away runs again, and its branch and work area are still there.
    /// Preparing again is what a second run does first, so it must not be what ends it.
    #[test]
    fn preparing_a_second_time_answers_with_the_same_place() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        let first = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();
        let again = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();

        assert_eq!(first, again);
    }

    /// What the last run committed is what the next one carries on from.
    #[test]
    fn a_second_run_finds_what_the_first_one_left() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        let made = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();
        fs::write(PathBuf::from(&made).join("half.txt"), "as far as it got\n").unwrap();
        run(Path::new(&made), &["add", "half.txt"]);
        run(Path::new(&made), &["commit", "-m", "as far as it got"]);

        let again = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();

        assert!(PathBuf::from(&again).join("half.txt").exists());
    }

    /// The work area may be gone while the branch it was on is not, which is what a tidy-up leaves.
    #[test]
    fn a_branch_without_its_work_area_is_checked_out_again() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        let made = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();
        fs::remove_dir_all(&made).unwrap();

        let again = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();

        assert_eq!(made, again);
        assert!(PathBuf::from(&again).join("README.md").exists());
    }

    /// Section 2.4 keeps the branch whatever happens to the work area, so what a run committed
    /// is still there to apply after the place it worked in has gone.
    #[test]
    fn taking_a_work_area_away_keeps_the_branch_it_was_on() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));
        let made = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();

        worktrees.remove(&at, &made).unwrap();

        assert!(!PathBuf::from(&made).exists());
        assert!(worktrees.holds(&at, "cistern/1"));
    }

    /// What a run left uncommitted is in no branch and the work area is the only place it is.
    /// Nothing here forces it, so git's refusal is what keeps it.
    #[test]
    fn a_work_area_holding_uncommitted_changes_is_refused_and_left() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));
        let made = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();
        fs::write(PathBuf::from(&made).join("half.txt"), "not committed\n").unwrap();
        run(Path::new(&made), &["add", "half.txt"]);

        let refused = worktrees.remove(&at, &made).unwrap_err();

        assert!(PathBuf::from(&made).join("half.txt").exists());
        assert!(!refused.reason.is_empty());
    }

    /// A work area somebody removed by hand is still registered, and a registration nothing
    /// prunes is what stops the branch being checked out again.
    #[test]
    fn a_work_area_that_is_already_gone_is_not_a_failure() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));
        let made = worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();
        fs::remove_dir_all(&made).unwrap();

        worktrees.remove(&at, &made).unwrap();

        assert!(worktrees.prepare(cutting(&at, "1", "cistern/1")).is_ok());
    }

    /// Two tasks never share a place, so a work area on another branch is not one to carry on in.
    #[test]
    fn a_work_area_on_another_branch_is_refused_and_says_which() {
        let repository = a_repository();
        let at = repository.path().display().to_string();
        let held = TempDir::new().unwrap();
        let worktrees = GitWorktrees::under(held.path().join("worktrees"));

        worktrees.prepare(cutting(&at, "1", "cistern/1")).unwrap();

        let refused = worktrees
            .prepare(cutting(&at, "1", "cistern/9"))
            .unwrap_err();

        assert!(refused.reason.contains("cistern/1"), "{}", refused.reason);
        assert!(refused.reason.contains("cistern/9"), "{}", refused.reason);
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
