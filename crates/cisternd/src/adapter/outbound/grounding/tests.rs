use std::{fs, process::Command};

use tempfile::TempDir;

use super::*;

/// A repository holding one file under `src`, and one script that fails.
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
    fs::write(at.join("src/index.rs"), "fn index() {}\n").unwrap();
    fs::write(at.join("fails.sh"), "#!/bin/sh\nexit 1\n").unwrap();
    fs::write(at.join("passes.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::write(at.join("waits.sh"), "#!/bin/sh\nsleep 30\n").unwrap();
    for script in ["fails.sh", "passes.sh", "waits.sh"] {
        Command::new("chmod")
            .args(["+x"])
            .arg(at.join(script))
            .output()
            .unwrap();
    }
    Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["add", "-A"])
        .output()
        .unwrap();
    Command::new("git")
        .arg("-C")
        .arg(at)
        .args(["commit", "-qm", "first"])
        .output()
        .unwrap();
    held
}

fn at(held: &TempDir) -> &str {
    held.path().to_str().unwrap()
}

/// A file reaches one, a directory reaches what it holds, and a date reaches nothing.
///
/// This is the whole of what the old rules were guessing at. `2026/08/26` reads as a path by
/// every rule of shape there is, and the repository settles it in one question.
#[test]
fn where_a_place_reaches_is_asked_rather_than_guessed() {
    let held = in_a_repository();

    assert_eq!(
        RepositoryGrounding.reaches(at(&held), "src/search.rs"),
        Some(1)
    );
    assert_eq!(RepositoryGrounding.reaches(at(&held), "src"), Some(2));
    assert_eq!(RepositoryGrounding.reaches(at(&held), "2026/08/26"), None);
    assert_eq!(RepositoryGrounding.reaches(at(&held), "he/she/they"), None);
    assert_eq!(
        RepositoryGrounding.reaches(at(&held), "src/nothing.rs"),
        None
    );
}

/// A place in a directory that is not a repository is not a place.
#[test]
fn somewhere_that_is_not_a_repository_reaches_nothing() {
    let held = TempDir::new().unwrap();
    assert_eq!(RepositoryGrounding.reaches(at(&held), "src"), None);
}

/// A command that does not exist is the model's mistake; one that fails is the task's point.
#[test]
fn a_command_this_machine_does_not_have_is_not_runnable() {
    let held = in_a_repository();

    assert!(RepositoryGrounding.runnable(at(&held), "git status"));
    assert!(RepositoryGrounding.runnable(at(&held), "./fails.sh"));
    assert!(!RepositoryGrounding.runnable(at(&held), "cargo-that-nobody-has test"));
    // A sentence about what done looks like is not a command, however true it is.
    assert!(!RepositoryGrounding.runnable(at(&held), "the count should match the documents"));
    assert!(!RepositoryGrounding.runnable(at(&held), ""));
}

/// Failing now is what a task worth running looks like. Passing now is a question.
#[test]
fn running_it_says_which_of_the_three_it_was() {
    let held = in_a_repository();
    let within = Duration::from_secs(30);

    assert_eq!(
        RepositoryGrounding.run(at(&held), "./fails.sh", within),
        Ran::Failed
    );
    assert_eq!(
        RepositoryGrounding.run(at(&held), "./passes.sh", within),
        Ran::Passed
    );
    assert_eq!(
        RepositoryGrounding.run(at(&held), "nothing-here --please", within),
        Ran::Unknown
    );
}

/// A run of the gate is not a run of the work.
#[test]
fn a_command_that_will_not_finish_is_stopped_and_says_nothing() {
    let held = in_a_repository();

    let ran = RepositoryGrounding.run(at(&held), "./waits.sh", Duration::from_millis(300));

    assert_eq!(ran, Ran::Unknown);
}

/// No shell stands between, so what would have been a second command is an argument nobody takes.
#[test]
fn nothing_a_shell_would_have_read_is_read() {
    let held = in_a_repository();
    let wrote = held.path().join("written");

    let ran = RepositoryGrounding.run(
        at(&held),
        &format!("./passes.sh ; touch {}", wrote.display()),
        Duration::from_secs(30),
    );

    // The script takes arguments it does not read, and there was no second command at all.
    assert_eq!(ran, Ran::Passed);
    assert!(!wrote.exists(), "a shell read what was meant as arguments");
}
