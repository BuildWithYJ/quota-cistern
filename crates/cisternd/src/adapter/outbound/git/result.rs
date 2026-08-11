//! What a task left on its branch, as git reads it.
//!
//! The only place that knows which commands answer these questions.
//! None of them reaches the core.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use crate::core::port::outbound::{
    Between, Changes, Commit, Counts, NotApplied, Results, Touched, Unavailable,
};

use super::said;

/// Results read out of the repository the task was added from.
///
/// Nothing is kept here.
/// The branch is where the work is, which is why a result outlives the work area it was made in.
pub struct GitResults;

impl Results for GitResults {
    fn counts(&self, between: Between<'_>) -> Option<Counts> {
        Some(Counts {
            commits: counted(&git(
                between.repository,
                &["rev-list", "--count", &behind(between.base, between.branch)],
            )?),
            base_ahead: counted(&git(
                between.repository,
                &["rev-list", "--count", &behind(between.branch, between.base)],
            )?),
        })
    }

    fn made(&self, between: Between<'_>) -> Option<Vec<Commit>> {
        let range = behind(between.base, between.branch);
        let written = git(
            between.repository,
            &[
                "log",
                &format!("--format={MARKS}%h%x09%s"),
                "--numstat",
                &range,
            ],
        )?;
        Some(made(&written))
    }

    fn changes(&self, between: Between<'_>) -> Option<Changes> {
        let range = diverged(between.base, between.branch);

        Some(Changes {
            files: touched(&git(between.repository, &["diff", "--numstat", &range])?),
            patch: git(between.repository, &["diff", "--patch", &range])?,
        })
    }

    fn apply(&self, between: Between<'_>) -> Result<Vec<Touched>, NotApplied> {
        if !clean(between.repository) {
            return Err(NotApplied::NotCommitted);
        }

        let range = diverged(between.base, between.branch);
        let Some(patch) = git(between.repository, &["diff", "--patch", &range]) else {
            return Err(NotApplied::NotThere);
        };
        if patch.trim().is_empty() {
            return Err(NotApplied::Nothing);
        }

        // Asked before anything is written, and answered without touching the working tree.
        // A refusal leaves nothing behind.
        if let Err(why) = putting(between.repository, &patch, Asking::Whether) {
            return Err(match already(between.repository, &patch) {
                true => NotApplied::Already,
                false => NotApplied::Conflicts { why },
            });
        }
        putting(between.repository, &patch, Asking::For)
            .map_err(|why| NotApplied::Conflicts { why })?;

        Ok(touched(
            &git(between.repository, &["diff", "--numstat", &range]).unwrap_or_default(),
        ))
    }

    fn reachable(&self, repository: &str) -> Result<(), Unavailable> {
        match git(repository, &["rev-parse", "--git-dir"]) {
            Some(_) => Ok(()),
            None => Err(Unavailable::new(format!(
                "{repository} is not a repository this can read"
            ))),
        }
    }
}

/// Whether a patch is one the working tree already holds.
///
/// A patch that goes in backwards is a patch that is in already.
/// Asked only when it would not go in forwards, so that nothing else is called that.
fn already(repository: &str, patch: &str) -> bool {
    putting(repository, patch, Asking::WhetherBackwards).is_ok()
}

/// Whether git is being asked or told.
#[derive(Clone, Copy)]
enum Asking {
    /// Would this go in?
    Whether,
    /// Is this in already?
    WhetherBackwards,
    /// Put it in.
    For,
}

/// The range from where the two branches parted to the end of the second.
///
/// Three dots rather than two, so that what the base gained afterwards is not read as something the task undid.
fn diverged(base: &str, branch: &str) -> String {
    format!("{base}...{branch}")
}

/// What the second holds and the first does not.
fn behind(first: &str, second: &str) -> String {
    format!("{first}..{second}")
}

/// Whether the repository holds changes nobody has committed.
fn clean(repository: &str) -> bool {
    git(repository, &["status", "--porcelain"]).is_some_and(|said| said.trim().is_empty())
}

/// Runs one command in the repository and hands back what it printed.
///
/// Nothing is answered for a command that failed.
/// A branch that is not there and a repository that is not there both come back this way: there is nothing to read.
fn git(repository: &str, args: &[&str]) -> Option<String> {
    let done = Command::new("git")
        .args(["-C", repository, "--no-pager"])
        .args(args)
        .output()
        .ok()?;

    done.status
        .success()
        .then(|| String::from_utf8_lossy(&done.stdout).into_owned())
}

/// Puts a patch to the working tree, or asks whether it would go.
fn putting(repository: &str, patch: &str, asking: Asking) -> Result<(), String> {
    let mut running = Command::new("git");
    running.args(["-C", repository, "apply"]);
    match asking {
        Asking::Whether => {
            running.arg("--check");
        }
        Asking::WhetherBackwards => {
            running.args(["--check", "--reverse"]);
        }
        Asking::For => (),
    }

    let mut child = running
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git apply: {e}"))?;

    if let Some(given) = child.stdin.as_mut() {
        let _ = given.write_all(patch.as_bytes());
    }
    drop(child.stdin.take());

    let done = child
        .wait_with_output()
        .map_err(|e| format!("git apply: {e}"))?;

    match done.status.success() {
        true => Ok(()),
        // One line of it.
        // What this becomes is a sentence printed beside the task it is about, and git's detail runs to several.
        false => Err(said(&done).lines().next().unwrap_or_default().to_owned()),
    }
}

/// What marks the start of a commit in what `git log` wrote.
///
/// The counted lines that follow a commit are tab-separated too, so a tab alone does not tell
/// one from the other.
/// This is the record separator, which cannot appear in a subject.
const MARKS: &str = "\u{1e}";

/// Commits as `git log` writes them, each followed by the files it touched.
///
/// A merge names no files and comes back counted as nothing, which is what it changed.
fn made(written: &str) -> Vec<Commit> {
    let mut all: Vec<Commit> = Vec::new();
    for line in written.lines().filter(|line| !line.trim().is_empty()) {
        match line.strip_prefix(MARKS) {
            Some(said) => {
                let (sha, subject) = said.split_once('\t').unwrap_or((said, ""));
                all.push(Commit {
                    sha: sha.to_owned(),
                    subject: subject.to_owned(),
                    added: "0".to_owned(),
                    removed: "0".to_owned(),
                });
            }
            None => {
                let Some(one) = all.last_mut() else { continue };
                let mut said = line.split('\t');
                let (Some(added), Some(removed)) = (said.next(), said.next()) else {
                    continue;
                };
                one.added = added_to(&one.added, added);
                one.removed = added_to(&one.removed, removed);
            }
        }
    }
    all
}

/// One more file's count added to what a commit has so far.
///
/// git counts no lines in a binary file, so a commit of nothing but those keeps the count it
/// had rather than becoming unreadable.
fn added_to(so_far: &str, one: &str) -> String {
    match (so_far.parse::<u64>(), one.parse::<u64>()) {
        (Ok(so_far), Ok(one)) => (so_far + one).to_string(),
        _ => so_far.to_owned(),
    }
}

/// Per-file counts, as `--numstat` writes them.
///
/// A binary file is counted with a dash rather than a number.
/// It is handed on as it was written: whoever reads it is a person.
fn touched(written: &str) -> Vec<Touched> {
    written
        .lines()
        .filter_map(|line| {
            let mut said = line.split('\t');
            Some(Touched {
                added: said.next()?.to_owned(),
                removed: said.next()?.to_owned(),
                path: said.next()?.to_owned(),
            })
        })
        .collect()
}

/// A count as `rev-list` writes it.
fn counted(written: &str) -> String {
    match written.trim() {
        "" => "0".to_owned(),
        said => said.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::TempDir;

    use super::*;

    /// A repository holding one commit on `main`, and a task's result on a branch beside it.
    fn a_repository_with_a_result() -> TempDir {
        let dir = TempDir::new().unwrap();
        let at = dir.path();
        run(at, &["init", "--initial-branch", "main"]);
        run(at, &["config", "user.email", "nobody@example.com"]);
        run(at, &["config", "user.name", "nobody"]);
        write(at, "README.md", "a repository\n");
        run(at, &["add", "."]);
        run(at, &["commit", "-m", "first"]);

        run(at, &["branch", "cistern/1"]);
        run(at, &["switch", "cistern/1"]);
        write(at, "verify.ts", "one\ntwo\n");
        run(at, &["add", "."]);
        run(at, &["commit", "-m", "the work"]);
        run(at, &["switch", "main"]);
        dir
    }

    fn run(at: &Path, args: &[&str]) {
        let done = Command::new("git")
            .arg("-C")
            .arg(at)
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}: {}", said(&done));
    }

    fn write(at: &Path, name: &str, text: &str) {
        fs::write(at.join(name), text).unwrap();
    }

    fn between<'a>(at: &'a str, branch: &'a str) -> Between<'a> {
        Between {
            repository: at,
            base: "main",
            branch,
        }
    }

    fn path_of(dir: &TempDir) -> String {
        dir.path().display().to_string()
    }

    #[test]
    fn what_a_task_changed_is_read_off_its_branch() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);

        let changes = GitResults.changes(between(&at, "cistern/1")).unwrap();
        assert_eq!(changes.files.len(), 1);
        assert_eq!(changes.files[0].path, "verify.ts");
        assert_eq!(changes.files[0].added, "2");
        assert_eq!(changes.files[0].removed, "0");
        assert!(changes.patch.contains("verify.ts"), "{}", changes.patch);

        let counts = GitResults.counts(between(&at, "cistern/1")).unwrap();
        assert_eq!(counts.commits, "1");
        assert_eq!(counts.base_ahead, "0");
    }

    /// Section 2.1 lists what the branch holds, newest first, with what each commit counted.
    #[test]
    fn every_commit_the_branch_gained_is_listed_newest_first() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        run(dir.path(), &["switch", "-q", "cistern/1"]);
        write(dir.path(), "notes.md", "later\n");
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "and a note"]);
        run(dir.path(), &["switch", "-q", "main"]);

        let made = GitResults.made(between(&at, "cistern/1")).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(made[0].subject, "and a note");
        assert_eq!(made[0].added, "1");
        assert_eq!(made[0].removed, "0");
        assert_eq!(made[1].subject, "the work");
        assert_eq!(made[1].added, "2");
        assert!(!made[0].sha.is_empty());
    }

    /// A subject holding a tab would be read as a count if a tab were what told them apart.
    #[test]
    fn a_subject_with_a_tab_in_it_is_still_one_commit() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        run(dir.path(), &["switch", "-q", "cistern/1"]);
        write(dir.path(), "more.md", "x\n");
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "one\ttwo\tthree"]);
        run(dir.path(), &["switch", "-q", "main"]);

        let made = GitResults.made(between(&at, "cistern/1")).unwrap();
        assert_eq!(made.len(), 2);
        assert_eq!(made[0].subject, "one\ttwo\tthree");
        assert_eq!(made[0].added, "1");
    }

    /// Section 2.4 reports how far the base has moved since the task left it.
    /// That is what the three-dot range keeps out of the diff.
    #[test]
    fn what_the_base_gained_afterwards_is_counted_and_not_diffed() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        write(dir.path(), "elsewhere.ts", "later\n");
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "moved on"]);

        assert_eq!(
            GitResults
                .counts(between(&at, "cistern/1"))
                .unwrap()
                .base_ahead,
            "1"
        );
        let changes = GitResults.changes(between(&at, "cistern/1")).unwrap();
        assert!(!changes.patch.contains("elsewhere.ts"), "{}", changes.patch);
    }

    /// A branch that is gone is not a branch that changed nothing, so it is answered for on its own.
    #[test]
    fn a_branch_that_is_not_there_reads_as_nothing() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        assert!(GitResults.changes(between(&at, "cistern/9")).is_none());
        assert!(GitResults.counts(between(&at, "cistern/9")).is_none());
        assert_eq!(
            GitResults.apply(between(&at, "cistern/9")),
            Err(NotApplied::NotThere)
        );
    }

    #[test]
    fn a_repository_that_is_not_there_reads_as_nothing() {
        let dir = TempDir::new().unwrap();
        let at = path_of(&dir);
        assert!(GitResults.changes(between(&at, "cistern/1")).is_none());
        assert!(GitResults.reachable(&at).is_err());
    }

    #[test]
    fn a_result_brought_in_is_in_the_working_tree_and_not_committed() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);

        let applied = GitResults.apply(between(&at, "cistern/1")).unwrap();
        assert_eq!(applied.len(), 1);
        assert!(dir.path().join("verify.ts").exists());

        let on = Command::new("git")
            .args(["-C", &at, "rev-list", "--count", "main"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&on.stdout).trim(), "1");
    }

    #[test]
    fn a_working_tree_holding_uncommitted_changes_is_refused() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        write(dir.path(), "README.md", "changed by hand\n");

        assert_eq!(
            GitResults.apply(between(&at, "cistern/1")),
            Err(NotApplied::NotCommitted)
        );
    }

    #[test]
    fn a_branch_holding_nothing_new_has_nothing_to_bring_in() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        run(dir.path(), &["branch", "cistern/2"]);

        assert_eq!(
            GitResults.apply(between(&at, "cistern/2")),
            Err(NotApplied::Nothing)
        );
    }

    /// Applying twice is the one refusal that means nothing is wrong.
    /// It is told apart from a clash with something else.
    #[test]
    fn a_result_that_is_in_the_working_tree_already_says_so() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);

        GitResults.apply(between(&at, "cistern/1")).unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "took it up"]);

        assert_eq!(
            GitResults.apply(between(&at, "cistern/1")),
            Err(NotApplied::Already)
        );
    }

    /// Section 2.4 says nothing is applied on a conflict.
    /// `--check` settles that before anything is written.
    #[test]
    fn a_result_that_clashes_leaves_the_working_tree_as_it_was() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);
        write(dir.path(), "verify.ts", "something else\n");
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "written by hand"]);

        let refused = GitResults.apply(between(&at, "cistern/1"));
        assert!(
            matches!(refused, Err(NotApplied::Conflicts { .. })),
            "{refused:?}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("verify.ts")).unwrap(),
            "something else\n"
        );
    }

    /// Section 2.4 says a result outlives the work area it was made in, which it does because nothing here reads one.
    #[test]
    fn a_result_is_read_off_the_branch_and_not_out_of_a_work_area() {
        let dir = a_repository_with_a_result();
        let at = path_of(&dir);

        assert!(GitResults.reachable(&at).is_ok());
        assert!(GitResults.changes(between(&at, "cistern/1")).is_some());
        assert!(!dir.path().join("worktrees").exists());
    }
}
