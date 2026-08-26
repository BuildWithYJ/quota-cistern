use super::*;

fn a_spec() -> Spec {
    Spec {
        goal: Part::inferred("remove the double count", "diff:src/search.rs:41-43"),
        place: Part::inferred("src/search.rs", "edited and uncommitted"),
        success: Part::given("cargo test search_counts_once"),
        on_failure: Part::open(),
        why: Part::inferred("the counter is incremented twice", "src/search.rs:41,43"),
        scope: Part::inferred("src/search.rs only", "the place"),
    }
}

/// The count is what a run is gated on, so it holds exactly the parts a run cannot start without.
#[test]
fn a_part_nobody_settled_is_a_decision_left_to_the_agent() {
    assert_eq!(a_spec().undecided(), vec![Named::OnFailure]);

    let mut settled = a_spec();
    settled.on_failure = Part::given("stop after three attempts");
    assert!(settled.undecided().is_empty());
}

/// `why` is read after a run rather than by one, so wanting it warns instead of refusing.
#[test]
fn a_missing_why_is_not_a_decision_a_run_waits_on() {
    let mut spec = a_spec();
    spec.on_failure = Part::given("stop after three attempts");
    spec.why = Part::open();

    assert!(spec.undecided().is_empty());
}

/// A model that answers with an empty string has settled nothing, whatever it marked the part.
#[test]
fn a_part_that_says_nothing_is_open_however_it_was_marked() {
    let empty = Part {
        said: Some("   ".to_owned()),
        settled: Settled::Inferred,
        drawn_from: Some("nowhere".to_owned()),
        others: Vec::new(),
    };

    assert!(empty.is_open());
}

/// What is stored and what the agent reads are the same text.
#[test]
fn a_spec_is_written_one_part_to_a_line() {
    let mut spec = a_spec();
    spec.on_failure = Part::given("stop after three attempts. do not edit tests/");

    assert_eq!(
        spec.written(),
        "goal: remove the double count\n\
         place: src/search.rs\n\
         success: cargo test search_counts_once\n\
         on failure: stop after three attempts. do not edit tests/\n\
         why: the counter is incremented twice\n\
         scope: src/search.rs only"
    );
}

/// A part nobody settled is left out rather than written as an empty heading.
#[test]
fn a_part_that_says_nothing_is_not_written() {
    let written = a_spec().written();

    assert!(!written.contains("on failure"), "{written}");
    assert!(written.starts_with("goal: "), "{written}");
}

/// Every part is named, so that a question or a refusal can say which one it means.
#[test]
fn every_part_says_what_is_left_undecided_while_it_is_open() {
    for named in Named::ALL {
        assert!(!named.label().is_empty());
        assert!(!named.left_to_decide().is_empty());
    }
    assert_eq!(Named::OnFailure.label(), "on failure");
    assert_eq!(
        Named::OnFailure.left_to_decide(),
        "what to do when it fails"
    );
}

/// What a surface hands back after asking is the spec it was shown, so the two travel as one text.
#[test]
fn a_spec_reads_back_out_of_what_it_was_written_as() {
    let mut written = a_spec();
    written.on_failure = Part::given("stop after three attempts");

    let read = Spec::read(&written.written()).expect("it names parts");

    assert_eq!(read.written(), written.written());
    // Read back, every part is the author's own: they saw it and sent it back.
    assert!(read.parts().all(|(_, part)| part.settled == Settled::Given));
    assert!(read.undecided().is_empty());
}

/// An ordinary instruction names no part, and is not a spec that happens to be empty.
#[test]
fn an_instruction_that_names_no_part_is_not_a_spec() {
    assert_eq!(Spec::read("fix parse() in src/util.rs; cargo test"), None);
    assert_eq!(Spec::read(""), None);
}

/// A part read from standard input runs to paragraphs, and stays whole.
#[test]
fn a_part_that_runs_past_one_line_keeps_the_rest_of_it() {
    let read =
        Spec::read("goal: rewrite the parser\nit panics on an empty line\nplace: src/util.rs")
            .unwrap();

    assert_eq!(
        read.goal.said.as_deref(),
        Some("rewrite the parser\nit panics on an empty line")
    );
    assert_eq!(read.place.said.as_deref(), Some("src/util.rs"));
}

/// A part nobody wrote is open when it is read back, not filled with an empty string.
#[test]
fn a_part_left_out_of_the_text_is_read_as_open() {
    let read = Spec::read("place: src/util.rs\nsuccess: cargo test").unwrap();

    assert!(read.on_failure.is_open());
    assert_eq!(
        read.undecided(),
        vec![Named::Goal, Named::OnFailure, Named::Scope]
    );
}
