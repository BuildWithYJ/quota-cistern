//! What a task is being added amid, as git reads it.
//!
//! The only place that knows the commands, and none of it reaches the core.

use std::process::Command;

use crate::core::port::outbound::{Room, Surroundings};

/// The uncommitted changes and by-word matches read out of the repository a task was added from.
pub struct GitSurroundings;

impl Surroundings for GitSurroundings {
    fn changes(&self, repository: &str, room: Room) -> String {
        // Against HEAD, staged or not, and with the body rather than the names: what is being
        // done to a file is what says which file was meant.
        //
        // Widely, so that what surrounds the change comes with it. A reader that can see the
        // function the line sits in, and the test below it, does not have to go and read the
        // file; a reader that goes and reads the file is an agent loop, and one costs what
        // running the task costs.
        within(
            run(repository, &["diff", "HEAD", "--unified=40"]).unwrap_or_default(),
            room,
        )
    }

    fn lately(&self, repository: &str, room: Room) -> String {
        let how_many = format!("-{}", room.most);
        // A repository with no commits at all fails rather than printing nothing, which is the
        // same thing to a reader either way.
        // Subjects alone. What each commit touched is another line per file and five times the
        // reading, and what the repository holds is already listed in full further down.
        within(
            run(repository, &["log", &how_many, "--oneline", "--no-color"]).unwrap_or_default(),
            room,
        )
    }

    fn branch(&self, repository: &str) -> Option<String> {
        let named = run(repository, &["branch", "--show-current"])?;
        let named = named.trim();
        // A detached head prints an empty line rather than failing.
        (!named.is_empty()).then(|| named.to_owned())
    }

    fn tracks(&self, repository: &str, room: Room) -> Vec<String> {
        let held = run(repository, &["ls-files"]).unwrap_or_default();
        within(held, room)
            .lines()
            .filter(|path| !path.starts_with(LEFT_OFF))
            .map(str::to_owned)
            .collect()
    }
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

/// What fits in the room, and a line saying what was left off.
///
/// Two limits, and whichever runs out first is the one that stops it. A count of lines bounds
/// nothing on its own: a lock file and a minified script are each one line per thousand
/// characters, and two hundred of them is a megabyte. What a task costs to add is not a
/// repository's to decide.
///
/// Left off rather than cut short in the middle: a reader given half a hunk reads a change that
/// was never made.
fn within(out: String, room: Room) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut chars = 0;
    let mut left_off = 0;

    for line in out.lines() {
        let line = narrowed(line);
        if kept.len() >= room.most || chars + line.chars().count() > room.chars {
            left_off += 1;
            continue;
        }
        chars += line.chars().count() + 1;
        kept.push(line);
    }
    if left_off > 0 {
        kept.push(format!("{LEFT_OFF} and {left_off} more lines"));
    }
    kept.join("\n")
}

/// What opens the line saying what was left off, so that a reader of paths can tell it from one.
const LEFT_OFF: &str = "...";

/// How wide a line may be before what is past that is generated rather than written.
///
/// A line of code someone typed is not this wide. One that is holds a minified script, a lock
/// file, or a line of data, and what is past the width is more of the same.
const WIDEST: usize = 400;

/// The line, cut to a width a person would have written within.
fn narrowed(line: &str) -> String {
    match line.chars().count() > WIDEST {
        false => line.to_owned(),
        true => {
            let held: String = line.chars().take(WIDEST).collect();
            format!(
                "{held} {LEFT_OFF} and {} more characters",
                line.chars().count() - WIDEST
            )
        }
    }
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

    /// More room than any of these needs, for a test about something other than the room.
    const ROOMY: Room = Room {
        most: 500,
        chars: 100_000,
    };

    fn at(held: &TempDir) -> &str {
        held.path().to_str().unwrap()
    }

    /// The body, not the names: what is being done to a file is what says it was meant.
    #[test]
    fn what_is_open_comes_back_as_the_change_itself() {
        let held = in_a_repository();

        let changes = GitSurroundings.changes(at(&held), ROOMY);

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

        let changes = GitSurroundings.changes(
            at(&held),
            Room {
                most: 20,
                chars: 20_000,
            },
        );

        assert_eq!(changes.lines().count(), 21, "{changes}");
        assert!(changes.ends_with("more lines"), "{changes}");
    }

    #[test]
    fn what_was_committed_lately_comes_back_by_its_subject() {
        let held = in_a_repository();

        let lately = GitSurroundings.lately(at(&held), ROOMY);

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
            GitSurroundings.tracks(at(&held), ROOMY),
            vec!["src/search.rs".to_owned()]
        );
        assert!(
            GitSurroundings
                .tracks(at(&held), Room { most: 0, chars: 0 })
                .is_empty()
        );
    }

    /// A count of lines bounds nothing: two hundred lines of a lock file is a megabyte.
    #[test]
    fn what_is_shown_is_bounded_by_characters_and_not_only_by_lines() {
        let held = in_a_repository();
        // One line per thousand characters, which is what a lock file and a minified script are.
        std::fs::write(
            held.path().join("src/search.rs"),
            (0..50)
                .map(|at| format!("// {}\n", "x".repeat(1000 + at)))
                .collect::<String>(),
        )
        .unwrap();

        let changes = GitSurroundings.changes(
            at(&held),
            Room {
                most: 200,
                chars: 2_000,
            },
        );

        assert!(
            changes.chars().count() < 4_000,
            "{}",
            changes.chars().count()
        );
        assert!(changes.ends_with("more lines"), "{changes}");
    }

    /// A line nobody typed is cut to a width somebody would have.
    #[test]
    fn a_line_too_wide_to_have_been_written_is_cut_to_a_width_that_was() {
        let held = in_a_repository();
        std::fs::write(
            held.path().join("src/search.rs"),
            format!("// {}\n", "x".repeat(10_000)),
        )
        .unwrap();

        let changes = GitSurroundings.changes(at(&held), ROOMY);

        assert!(changes.contains("more characters"), "{changes}");
        assert!(
            changes.lines().all(|line| line.chars().count() < 500),
            "a line was left as wide as it came"
        );
    }

    /// A place that is not a repository answers with nothing rather than failing.
    #[test]
    fn somewhere_that_is_not_a_repository_says_nothing() {
        let held = TempDir::new().unwrap();
        let at = at(&held);

        assert!(GitSurroundings.changes(at, ROOMY).is_empty());
        assert!(GitSurroundings.lately(at, ROOMY).is_empty());
        assert_eq!(GitSurroundings.branch(at), None);
        assert!(GitSurroundings.tracks(at, ROOMY).is_empty());
    }
}
