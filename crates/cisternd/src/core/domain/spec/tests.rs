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
    // And it is not something to show an author either: there is nothing to show.
    assert!(!empty.wants_seeing());
}

/// What the author said is theirs. What was drawn from the repository they have not seen yet.
#[test]
fn only_what_was_drawn_from_the_repository_wants_seeing() {
    let spec = a_spec();
    assert_eq!(
        spec.unseen(),
        vec![Named::Goal, Named::Place, Named::Why, Named::Scope]
    );
    // The one the author wrote is not among them, and neither is the one nobody wrote.
    assert!(!spec.success.wants_seeing());
    assert!(!spec.on_failure.wants_seeing());
}

/// Accepting the spec is the author seeing every inference at once.
#[test]
fn accepting_makes_every_inference_the_authors_own() {
    let mut spec = a_spec();
    spec.place = spec.place.clone().beside(&["src/index.rs".to_owned()]);

    spec.seen();

    assert!(spec.unseen().is_empty());
    assert_eq!(spec.place.settled, Settled::Given);
    // The others were the alternatives to an inference nobody had taken. It has been taken.
    assert!(spec.place.others.is_empty());
    // Nothing that was open is settled by being looked at.
    assert_eq!(spec.undecided(), vec![Named::OnFailure]);
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
