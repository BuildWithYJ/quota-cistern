use super::*;

/// The words the shipped definition reads an answer by.
const GOAL: &str = "GOAL";
const PLACE: &str = "PLACE";
const SUCCESS: &str = "SUCCESS";

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
         SUCCESS: cargo test search\n\
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
    assert_eq!(read.success.unwrap().drawn_from, None);
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
    for word in [GOAL, PLACE, SUCCESS, "-ASK", "-OR"] {
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
    let held = drafter.read("PLACE: src/serch.rs\nSUCCESS: cargo test search");
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
