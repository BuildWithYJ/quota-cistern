use std::{fs, os::unix::fs::PermissionsExt};

use tempfile::TempDir;

use super::*;

/// The words the shipped definition reads an answer by.
const GOAL: &str = "GOAL";
const PLACE: &str = "PLACE";
const DONE_WHEN: &str = "DONEWHEN";

fn shipped() -> ProgramDrafter {
    ProgramDrafter::new(Definition::of("claude", None).expect("the shipped definition parses"))
}

fn asking<'a>(instruction: &'a str, changes: &'a str, tracks: &'a [String]) -> Draft<'a> {
    Draft {
        instruction,
        changes,
        lately: "",
        branch: Some("fix/search-dupe"),
        tracks,
        repository: "/work/api",
    }
}

#[test]
fn a_part_is_read_with_what_it_was_drawn_from_and_what_else_was_allowed() {
    let read = shipped().read(
        "GOAL: remove the double count\n\
         GOAL-FROM: src/search.rs:41,43\n\
         PLACE: src/search.rs\n\
         PLACE-FROM: edited and uncommitted\n\
         PLACE-OR: src/index.rs, src/count.rs\n\
         DONEWHEN: cargo test search\n\
         ONFAIL:\n\
         ONFAIL-ASK: What should it do when it cannot get there?\n\
         ONFAIL-OR: stop after three attempts, do not edit tests, put it back\n\
         WHY: the counter is incremented twice\n\
         SCOPE: src/search.rs only",
    );

    let place = read.place.expect("a place was proposed");
    assert_eq!(place.said, "src/search.rs");
    assert_eq!(place.drawn_from.as_deref(), Some("edited and uncommitted"));
    assert_eq!(place.others, vec!["src/index.rs", "src/count.rs"]);

    assert_eq!(read.goal.unwrap().said, "remove the double count");
    // A part it was told to leave empty still comes back, carrying what to ask about it: that
    // is the whole of what a person is shown for one nobody settled.
    let on_failure = read.on_failure.expect("a question was written about it");
    assert!(on_failure.said.is_empty());
    assert_eq!(
        on_failure.asks.as_deref(),
        Some("What should it do when it cannot get there?")
    );
    assert_eq!(on_failure.others.len(), 3);
    // A part with no evidence line is still a part.
    assert_eq!(read.done_when.unwrap().drawn_from, None);
}

/// `PLACE` must not be read off the `PLACE-FROM` line that follows it.
#[test]
fn a_part_is_told_from_the_line_that_says_where_it_came_from() {
    let read = shipped().read("PLACE-FROM: edited and uncommitted\nPLACE: src/search.rs");

    assert_eq!(read.place.unwrap().said, "src/search.rs");
}

#[test]
fn an_answer_that_names_no_part_proposes_nothing() {
    let read = shipped().read("I could not tell from what I was given.");

    assert_eq!(read, Drafted::default());
}

/// The vendor's words live in the definition, so an answer written in another vendor's is read.
#[test]
fn the_words_the_definition_names_are_the_ones_read() {
    let mut definition = Definition::of("claude", None).unwrap();
    definition.drafter.place = "WHERE".to_owned();
    definition.drafter.drawn_from = " BECAUSE".to_owned();

    let read = ProgramDrafter::new(definition).read("WHERE: src/api.rs\nWHERE BECAUSE: the diff");

    let place = read.place.expect("a place was proposed");
    assert_eq!(place.said, "src/api.rs");
    assert_eq!(place.drawn_from.as_deref(), Some("the diff"));
}

/// Everything the author was looking at reaches the model, and nothing is left standing.
#[test]
fn what_is_asked_is_filled_in_from_the_definition() {
    let tracks = ["src/search.rs".to_owned(), "src/index.rs".to_owned()];
    let asked = shipped().asking(&asking(
        "make it stop double-counting",
        "--- a/src/search.rs\n+    count += 1;",
        &tracks,
    ));

    assert!(asked.contains("make it stop double-counting"));
    assert!(asked.contains("+    count += 1;"));
    assert!(asked.contains("fix/search-dupe"));
    assert!(asked.contains("src/index.rs"));
    // The words are written in from the same place the reader takes them from.
    for word in [GOAL, PLACE, DONE_WHEN, "-ASK", "-OR"] {
        assert!(asked.contains(word), "{word} is not asked for");
    }
    for standing in [
        "{instruction}",
        "{changes}",
        "{files}",
        "{branch}",
        "{place}",
    ] {
        assert!(!asked.contains(standing), "{standing} was left standing");
    }
}

/// An instruction is filled in among the other places, so one reading like a place of its own
/// must not be filled in again.
#[test]
fn an_instruction_naming_a_place_is_written_as_it_stands() {
    let asked = shipped().asking(&asking("rename {files} to something else", "", &[]));

    assert!(asked.contains("rename {files} to something else"));
}

/// Nothing open and nothing committed are both said rather than left blank, so a model is not
/// left to read an empty heading as an omission.
#[test]
fn an_empty_heading_says_that_it_is_empty() {
    let asked = shipped().asking(&asking("it feels off", "", &[]));

    assert!(asked.contains("(nothing is uncommitted)"));
    assert!(asked.contains("(nothing has been committed)"));
    assert!(asked.contains("(unavailable)"));
}

/// Asked again, the model is given its own answer and what the repository said about it.
///
/// All three: a model told only what was wrong writes the rest again from nothing.
#[test]
fn asking_again_carries_the_first_answer_and_what_did_not_hold_up() {
    let drafter = shipped();
    let held = drafter.read("PLACE: src/serch.rs\nDONEWHEN: cargo test search");
    let asked = format!(
        "{}\n\n{}\n\n{}",
        drafter.asking(&asking("make it faster", "", &[])),
        written(&held, &drafter.definition.drafter),
        super::super::fill(
            drafter.definition.drafter.again.trim(),
            &[("amiss", "the repository tracks no src/serch.rs")]
        )
    );

    assert!(asked.contains("PLACE: src/serch.rs"));
    assert!(asked.contains("the repository tracks no src/serch.rs"));
    assert!(!asked.contains("{amiss}"));
}

/// A vendor that writes down which model it was asked for, and answers as it was told to.
///
/// A shell script rather than a fake behind the trait, because what is under test is the arm
/// this file chooses: a fake in place of `ProgramDrafter` would be a test of the fake.
struct Standing {
    held: TempDir,
}

impl Standing {
    /// One that answers with the given lines whatever it is asked.
    fn answering(with: &str) -> Self {
        Standing::running(&format!("cat <<'ANSWER'\n{with}\nANSWER"))
    }

    /// One that runs the given shell, with the model it was asked for in `$MODEL`.
    fn running(shell: &str) -> Self {
        let held = TempDir::new().unwrap();
        let at = held.path().join("vendor");
        // The arguments arrive as the definition groups them: `-p <prompt> --model <model>
        // --max-turns <turns>`, so the model asked for is the fourth.
        fs::write(
            &at,
            format!(
                "#!/bin/sh\nMODEL=\"$4\"\necho \"$MODEL\" >> \"$(dirname \"$0\")/asked\"\n{shell}\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&at, fs::Permissions::from_mode(0o755)).unwrap();
        Standing { held }
    }

    /// Somewhere that exists for the program to be run in.
    fn at(&self) -> &str {
        self.held.path().to_str().unwrap()
    }

    /// A drafter that runs it instead of the vendor.
    fn drafter(&self) -> ProgramDrafter {
        let mut definition = Definition::of("claude", None).unwrap();
        definition.drafter.program = self.held.path().join("vendor").display().to_string();
        ProgramDrafter::new(definition)
    }

    /// The models it was asked for, in the order they were asked.
    fn asked(&self) -> Vec<String> {
        fs::read_to_string(self.held.path().join("asked"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

/// The same, run somewhere that exists, since the program is run in the repository.
fn asking_in<'a>(instruction: &'a str, repository: &'a str) -> Draft<'a> {
    Draft {
        repository,
        ..asking(instruction, "", &[])
    }
}

/// A whole answer, and one that settles nothing but says what it could not.
const WORKED_OUT: &str = "GOAL: stop the double count\nPLACE: src/search.rs";
const ASKED_BACK: &str = "GOAL:\nGOAL-ASK: which part of it?\nGOAL-OR: the counter, the loop";

/// The cheap model answered, so nothing else is asked.
#[test]
fn a_model_that_answered_is_the_answer() {
    let standing = Standing::answering(WORKED_OUT);

    let drafted = standing
        .drafter()
        .draft(asking_in("make it faster", standing.at()))
        .expect("it answered");

    assert_eq!(drafted.place.unwrap().said, "src/search.rs");
    assert_eq!(standing.asked(), vec!["haiku"]);
}

/// A part left open carrying a question is an answer, and the costly kind to mistake for one.
///
/// It is the model saying what it could not tell, which is what the author is there to settle.
/// A stronger one cannot tell it either, so asking buys a second answer to the same question at
/// another ask and another minute.
#[test]
fn a_part_left_open_with_a_question_is_still_an_answer() {
    let standing = Standing::answering(ASKED_BACK);

    let drafted = standing
        .drafter()
        .draft(asking_in("make it faster", standing.at()))
        .expect("it answered");

    assert!(drafted.place.is_none());
    assert_eq!(
        drafted.goal.unwrap().asks.as_deref(),
        Some("which part of it?")
    );
    assert_eq!(standing.asked(), vec!["haiku"]);
}

/// An answer naming no part is nothing, and nothing is what the stronger model is for.
#[test]
fn a_stronger_model_is_asked_where_nothing_came_back() {
    let standing = Standing::running(&format!(
        "case \"$MODEL\" in haiku) echo 'I could not tell.';; *) cat <<'ANSWER'\n{WORKED_OUT}\nANSWER\n;; esac"
    ));

    let drafted = standing
        .drafter()
        .draft(asking_in("make it faster", standing.at()))
        .expect("the stronger one answered");

    assert_eq!(drafted.place.unwrap().said, "src/search.rs");
    assert_eq!(standing.asked(), vec!["haiku", "sonnet"]);
}

/// A correction is a correction, not a harder question, so the cheap model makes it.
#[test]
fn asking_again_asks_the_cheap_model() {
    let standing = Standing::answering(WORKED_OUT);
    let held = standing.drafter().read(WORKED_OUT);

    standing
        .drafter()
        .draft_again(
            asking_in("make it faster", standing.at()),
            &held,
            &["nowhere".to_owned()],
        )
        .expect("it answered");

    assert_eq!(standing.asked(), vec!["haiku"]);
}
