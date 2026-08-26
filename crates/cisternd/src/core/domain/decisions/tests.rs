use super::super::spec::Part;
use super::*;

const WROTE: &str = "make it stop double-counting";

/// A spec with nothing left, as the repository answered for it.
fn settled() -> (Spec, Grounded) {
    (
        Spec {
            goal: Part::inferred("remove the double count", "diff:src/search.rs:41-43"),
            place: Part::inferred("src/search.rs", "edited and uncommitted"),
            success: Part::given("cargo test search_counts_once"),
            on_failure: Part::given("stop after three attempts. do not edit tests/"),
            why: Part::inferred("the counter is incremented twice", "src/search.rs:41,43"),
            scope: Part::inferred("src/search.rs only", "the place"),
        },
        Grounded {
            files: Some(1),
            runnable: true,
        },
    )
}

#[test]
fn a_spec_that_settles_everything_leaves_nothing_to_decide() {
    let (spec, grounded) = settled();
    assert_eq!(left_to_decide(&spec, WROTE, grounded), Vec::new());
}

/// The one an agent settles by editing the test, so it is the one that must not be left.
#[test]
fn nothing_said_about_failing_is_a_decision_left() {
    let (mut spec, grounded) = settled();
    spec.on_failure = Part::open();

    let left = left_to_decide(&spec, WROTE, grounded);

    assert_eq!(left, vec![Undecided::Unsettled(Named::OnFailure)]);
    assert_eq!(left[0].left_to_decide(), "what to do when it fails");
}

/// A reviewer reads it afterwards; a run does not read it at all.
#[test]
fn nothing_said_about_why_holds_no_run_back() {
    let (mut spec, grounded) = settled();
    spec.why = Part::open();

    assert_eq!(left_to_decide(&spec, WROTE, grounded), Vec::new());
}

/// A model that could not tell what was meant and answered by repeating the question worked
/// nothing out, whatever it marked the part.
#[test]
fn an_inference_that_is_the_authors_own_words_inferred_nothing() {
    let (mut spec, grounded) = settled();
    spec.goal = Part::inferred(WROTE, "the instruction");

    let left = left_to_decide(&spec, WROTE, grounded);

    assert_eq!(left, vec![Undecided::Echoed(Named::Goal)]);
    assert!(left[0].left_to_decide().contains("nothing was worked out"));
}

/// The author's own words are the author's, however much they look like an echo.
#[test]
fn what_the_author_said_themselves_is_never_an_echo() {
    let (mut spec, grounded) = settled();
    spec.goal = Part::given(WROTE);

    assert_eq!(left_to_decide(&spec, WROTE, grounded), Vec::new());
}

/// A directory of two hundred files is a search, and where to stop is a decision.
#[test]
fn a_place_that_reaches_too_far_is_a_decision_left() {
    let (spec, mut grounded) = settled();
    grounded.files = Some(REACHES_AT_MOST + 1);

    let left = left_to_decide(&spec, WROTE, grounded);

    assert_eq!(
        left,
        vec![Undecided::Reaches {
            files: REACHES_AT_MOST + 1
        }]
    );
    assert!(left[0].left_to_decide().contains("11 files"));

    // The line is where it is said to be, and one file short of it is not over it.
    grounded.files = Some(REACHES_AT_MOST);
    assert_eq!(left_to_decide(&spec, WROTE, grounded), Vec::new());
}

/// Nobody is asked twice about the same part.
#[test]
fn a_place_nobody_settled_is_counted_once_and_not_measured() {
    let (mut spec, mut grounded) = settled();
    spec.place = Part::open();
    grounded.files = Some(REACHES_AT_MOST + 1);

    assert_eq!(
        left_to_decide(&spec, WROTE, grounded),
        vec![Undecided::Unsettled(Named::Place)]
    );
}

/// A sentence about what "done" looks like leaves the judging to the agent.
#[test]
fn a_success_condition_nothing_can_run_leaves_the_agent_to_judge_itself() {
    let (mut spec, mut grounded) = settled();
    spec.success = Part::given("the count should match the number of documents");
    grounded.runnable = false;

    let left = left_to_decide(&spec, WROTE, grounded);

    assert_eq!(left, vec![Undecided::Unverifiable]);
    assert_eq!(left[0].left_to_decide(), "whether it is done");
}

/// Which is told apart from nobody having said anything about it at all.
#[test]
fn a_success_condition_nobody_settled_is_counted_once() {
    let (mut spec, mut grounded) = settled();
    spec.success = Part::open();
    grounded.runnable = false;

    assert_eq!(
        left_to_decide(&spec, WROTE, grounded),
        vec![Undecided::Unsettled(Named::Success)]
    );
}

/// The two words that pass the gate today, counted.
///
/// `src/search.rs cargo test` names a real file and holds the word `test`, which is all the
/// signals it used to read. Nothing says what the work is, what to do when it fails, or how far
/// to reach, and none of that was ever counted.
#[test]
fn the_two_words_that_used_to_pass_leave_three_decisions() {
    let mut spec = Spec::open();
    spec.place = Part::given("src/search.rs");
    spec.success = Part::given("cargo test");

    let left = left_to_decide(
        &spec,
        "src/search.rs cargo test",
        Grounded {
            files: Some(1),
            runnable: true,
        },
    );

    assert_eq!(
        left,
        vec![
            Undecided::Unsettled(Named::Goal),
            Undecided::Unsettled(Named::OnFailure),
            Undecided::Unsettled(Named::Scope),
        ]
    );
}
