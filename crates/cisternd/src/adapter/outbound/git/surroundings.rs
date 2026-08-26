//! What a task is being added amid, as git reads it.
//!
//! The only place that knows the commands, and none of it reaches the core.

use std::process::Command;

use crate::core::port::outbound::Surroundings;

/// The uncommitted changes and by-word matches read out of the repository a task was added from.
pub struct GitSurroundings;

impl Surroundings for GitSurroundings {
    fn changes(&self, repository: &str, lines: usize) -> String {
        // Against HEAD, staged or not, and with the body rather than the names: what is being
        // done to a file is what says which file was meant.
        //
        // Widely, so that what surrounds the change comes with it. A reader that can see the
        // function the line sits in, and the test below it, does not have to go and read the
        // file; a reader that goes and reads the file is an agent loop, and one costs what
        // running the task costs.
        capped(
            run(repository, &["diff", "HEAD", "--unified=40"]).unwrap_or_default(),
            lines,
        )
    }

    fn lately(&self, repository: &str, commits: usize) -> String {
        let how_many = format!("-{commits}");
        // A repository with no commits at all fails rather than printing nothing, which is the
        // same thing to a reader either way.
        // Subjects alone. What each commit touched is another line per file and five times the
        // reading, and what the repository holds is already listed in full further down.
        run(repository, &["log", &how_many, "--oneline", "--no-color"]).unwrap_or_default()
    }

    fn branch(&self, repository: &str) -> Option<String> {
        let named = run(repository, &["branch", "--show-current"])?;
        let named = named.trim();
        // A detached head prints an empty line rather than failing.
        (!named.is_empty()).then(|| named.to_owned())
    }

    fn tracks(&self, repository: &str, paths: usize) -> Vec<String> {
        run(repository, &["ls-files"])
            .map(|held| held.lines().take(paths).map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

/// The first lines of what a command printed, and a line saying what was left off.
///
/// Left off rather than cut short in the middle: a reader given half a hunk reads a change that
/// was never made.
fn capped(out: String, lines: usize) -> String {
    let held: Vec<&str> = out.lines().collect();
    if held.len() <= lines {
        return out;
    }
    let mut capped = held[..lines].join("\n");
    capped.push_str(&format!("\n... and {} more lines", held.len() - lines));
    capped
}

/// Runs one git command in the repository and hands back what it printed, nothing when it failed.
fn run(repository: &str, args: &[&str]) -> Option<String> {
    let done = Command::new("git")
        .args(["-C", repository, "--no-pager"])
        .args(args)
        .output()
        .ok()?;

    done.status
        .success()
        .then(|| String::from_utf8_lossy(&done.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use tempfile::TempDir;

    use super::*;

    /// A repository with one commit behind it and one file open.
    fn in_a_repository() -> TempDir {
        let held = TempDir::new().unwrap();
        let at = held.path();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
        ] {
            Command::new("git")
                .arg("-C")
                .arg(at)
                .args(args)
                .output()
                .unwrap();
        }
        fs::create_dir_all(at.join("src")).unwrap();
        fs::write(at.join("src/search.rs"), "fn search() {}\n").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["add", "-A"])
            .output()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(at)
            .args(["commit", "-qm", "add search"])
            .output()
            .unwrap();
        // Open but not committed, which is what "this" means.
        fs::write(at.join("src/search.rs"), "fn search() { /* twice */ }\n").unwrap();
        held
    }

    fn at(held: &TempDir) -> &str {
        held.path().to_str().unwrap()
    }

    /// The body, not the names: what is being done to a file is what says it was meant.
    #[test]
    fn what_is_open_comes_back_as_the_change_itself() {
        let held = in_a_repository();

        let changes = GitSurroundings.changes(at(&held), 200);

        assert!(changes.contains("src/search.rs"), "{changes}");
        assert!(
            changes.contains("+fn search() { /* twice */ }"),
            "{changes}"
        );
        assert!(changes.contains("-fn search() {}"), "{changes}");
    }

    /// A working tree can hold a rewrite, and a reader that is a model is paid for by the line.
    #[test]
    fn a_change_too_long_to_read_is_left_off_rather_than_cut_short() {
        let held = in_a_repository();
        fs::write(
            held.path().join("src/search.rs"),
            (0..500)
                .map(|at| format!("fn f{at}() {{}}\n"))
                .collect::<String>(),
        )
        .unwrap();

        let changes = GitSurroundings.changes(at(&held), 20);

        assert_eq!(changes.lines().count(), 21, "{changes}");
        assert!(changes.ends_with("more lines"), "{changes}");
    }

    #[test]
    fn what_was_committed_lately_comes_back_by_its_subject() {
        let held = in_a_repository();

        let lately = GitSurroundings.lately(at(&held), 10);

        assert!(lately.contains("add search"), "{lately}");
    }

    /// Half of what an author means, for one command.
    #[test]
    fn the_branch_is_read_and_a_detached_head_is_not_one() {
        let held = in_a_repository();
        assert_eq!(GitSurroundings.branch(at(&held)).as_deref(), Some("main"));

        Command::new("git")
            .arg("-C")
            .arg(held.path())
            .args(["checkout", "-q", "--detach"])
            .output()
            .unwrap();
        assert_eq!(GitSurroundings.branch(at(&held)), None);
    }

    /// What a place is checked against, so that one that was invented is told from one that is.
    #[test]
    fn the_paths_the_repository_tracks_come_back_capped() {
        let held = in_a_repository();

        assert_eq!(
            GitSurroundings.tracks(at(&held), 100),
            vec!["src/search.rs".to_owned()]
        );
        assert!(GitSurroundings.tracks(at(&held), 0).is_empty());
    }

    /// A place that is not a repository answers with nothing rather than failing.
    #[test]
    fn somewhere_that_is_not_a_repository_says_nothing() {
        let held = TempDir::new().unwrap();
        let at = at(&held);

        assert!(GitSurroundings.changes(at, 200).is_empty());
        assert!(GitSurroundings.lately(at, 10).is_empty());
        assert_eq!(GitSurroundings.branch(at), None);
        assert!(GitSurroundings.tracks(at, 100).is_empty());
    }
}
