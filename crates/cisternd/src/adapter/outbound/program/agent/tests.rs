use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::json;
use tempfile::TempDir;

use super::*;

/// The agent that stands in for a vendor's.
/// A shell program, kept as one rather than as a string this file would have to escape.
const STANDING_IN: &str = include_str!("standing-agent.sh");

/// An answer Claude Code actually sent, kept as it arrived apart from a shortened `result`.
///
/// It holds the fields the definition names and a dozen it does not.
/// That is what makes a test say that a vendor adding one changes nothing.
const AN_ANSWER: &str = include_str!("claude-answer.json");

fn standing_in(held: &TempDir) -> ProgramAgent {
    let program = held.path().join("agent");
    let saw = held.path().join("prompt");
    fs::write(
        &program,
        STANDING_IN.replace("{prompt}", &saw.display().to_string()),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    wait_until_runnable(&program);

    let mut definition = Definition::of("claude", None).unwrap();
    definition.program = program.display().to_string();
    ProgramAgent::new(definition)
}

/// Waits for the file just written to be one this may run.
///
/// A thread that starts a child while another is writing a file leaves it open in that child.
/// Running a file open for writing is refused.
fn wait_until_runnable(program: &std::path::Path) {
    for _ in 0..100 {
        let ran = Command::new(program)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if ran.is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The answer with something about it changed.
///
/// The object rather than its text.
/// A fixture kept as the vendor sent it can then be reshaped without a test knowing its layout.
fn answer_with(change: impl FnOnce(&mut Value)) -> String {
    let mut answer: Value = serde_json::from_str(AN_ANSWER).unwrap();
    change(&mut answer);
    answer.to_string()
}

/// A shell command that answers with `written` and nothing else.
fn answering(held: &TempDir, written: &str) -> String {
    let one: Value = serde_json::from_str(written).unwrap();
    let at = held.path().join("answer.json");
    fs::write(&at, one.to_string()).unwrap();
    format!("cat '{}'", at.display())
}

fn prompt(held: &TempDir) -> String {
    fs::read_to_string(held.path().join("prompt")).unwrap()
}

/// Every argument the agent was given, one to a line.
fn given(held: &TempDir) -> String {
    fs::read_to_string(held.path().join("prompt.args")).unwrap()
}

fn working<'a>(at: &'a str, instruction: &'a str) -> Work<'a> {
    Work {
        task: "1",
        at,
        trace: Box::new(|_line: &str| {}),
        instruction,
        model: None,
        ceiling: None,
    }
}

#[test]
fn an_agent_that_finished_is_answered_as_done() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(&held.path().display().to_string(), "exit 0"))
        .unwrap();

    assert_eq!(ended.outcome, Outcome::Finished);
    assert_eq!(ended.reason, None);
}

#[test]
fn what_the_agent_counted_is_read_by_the_paths_the_definition_names() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &answering(&held, AN_ANSWER),
        ))
        .unwrap();

    assert_eq!(
        ended.observed,
        Observed::Spent(Spent {
            input: "77".to_owned(),
            output: "3377".to_owned(),
            cache_written: "28879".to_owned(),
            cache_read: "263483".to_owned(),
            // 0.0921703 dollars, as millionths of one.
            cost: "92170".to_owned(),
        })
    );
}

/// The run's own `usage` counts only the calls that left a message behind.
/// This answer shows the gap: 3182 against 3377.
#[test]
fn what_a_run_counted_is_more_than_the_calls_that_left_a_message() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &answering(&held, AN_ANSWER),
        ))
        .unwrap();

    let Observed::Spent(spent) = ended.observed else {
        panic!("the answer holds a count");
    };
    assert_ne!(spent.output, "3182");
}

/// A run may reach for more than one model, and the star in the path adds them up.
#[test]
fn a_run_that_reached_for_two_models_counts_both() {
    let held = TempDir::new().unwrap();
    let two = answer_with(|answer| {
        answer["modelUsage"]["claude-opus-5"] = json!({
            "inputTokens": 1,
            "outputTokens": 2,
            "cacheCreationInputTokens": 3,
            "cacheReadInputTokens": 4,
        });
    });
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &answering(&held, &two),
        ))
        .unwrap();

    let Observed::Spent(spent) = ended.observed else {
        panic!("the answer holds a count");
    };
    assert_eq!(spent.input, "78");
    assert_eq!(spent.output, "3379");
    assert_eq!(spent.cache_written, "28882");
    assert_eq!(spent.cache_read, "263487");
}

/// A vendor that renames a field would otherwise hide what a run spent.
/// It would report a run that spent a hundred thousand tokens as having spent none.
#[test]
fn a_count_under_a_name_this_does_not_know_is_not_a_count_of_nothing() {
    let held = TempDir::new().unwrap();
    let renamed = answer_with(|answer| {
        let counted = &mut answer["modelUsage"]["claude-haiku-4-5-20251001"];
        counted["tokensIn"] = counted["inputTokens"].take();
    });
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &answering(&held, &renamed),
        ))
        .unwrap();

    assert!(matches!(ended.observed, Observed::Unreadable { .. }));
}

#[test]
fn an_answer_naming_no_model_is_not_a_count_of_nothing() {
    let held = TempDir::new().unwrap();
    let none = answer_with(|answer| answer["modelUsage"] = json!({}));
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &answering(&held, &none),
        ))
        .unwrap();

    assert!(matches!(ended.observed, Observed::Unreadable { .. }));
}

#[test]
fn an_answer_that_says_nothing_about_a_count_is_not_a_count_of_nothing() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(&held.path().display().to_string(), "echo done"))
        .unwrap();

    assert!(matches!(ended.observed, Observed::Unreadable { .. }));
}

/// What the vendor answers with when the spend ceiling cut the run off.
///
/// Checked against Claude Code 2.1.227: `--max-budget-usd` ends the run with
/// `error_max_budget_usd`. A word the definition does not carry would have this reported as a
/// failure rather than as a run that reached a ceiling we set.
#[test]
fn a_run_cut_off_at_the_spend_ceiling_says_so_rather_than_failing() {
    let held = TempDir::new().unwrap();
    let cut_off = answer_with(|answer| {
        answer["subtype"] = json!("error_max_budget_usd");
        answer["result"] = Value::Null;
    });
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &format!("{}; exit 1", answering(&held, &cut_off)),
        ))
        .unwrap();

    assert_eq!(
        ended.reason.as_deref(),
        Some("the agent was cut off at 20 dollars")
    );
}

/// A run that was cut off still consumed what it consumed.
#[test]
fn a_run_that_failed_still_reports_what_it_consumed() {
    let held = TempDir::new().unwrap();
    let cut_off = answer_with(|answer| {
        answer["subtype"] = json!("error_max_turns");
        answer["result"] = Value::Null;
    });
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &format!("{}; exit 1", answering(&held, &cut_off)),
        ))
        .unwrap();

    assert_ne!(ended.outcome, Outcome::Finished);
    assert_eq!(
        ended.reason.as_deref(),
        Some("the agent was cut off after 200 turns")
    );
    assert!(matches!(ended.observed, Observed::Spent(_)));
}

#[test]
fn an_agent_that_failed_is_answered_with_what_it_said() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            "echo it went wrong >&2; exit 3",
        ))
        .unwrap();

    assert_ne!(ended.outcome, Outcome::Finished);
    assert_eq!(ended.reason.as_deref(), Some("it went wrong"));
}

/// The child stops where it is when nobody reads what it writes, and a task that stops there never ends.
#[test]
fn a_child_that_writes_more_than_a_pipe_holds_still_finishes() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            "yes abcdefghijklmnopqrstuvwxyz | head -c 2000000",
        ))
        .unwrap();

    assert_eq!(ended.outcome, Outcome::Finished, "{ended:?}");
}

/// The agent runs where the task's work area is, not where the core was started.
#[test]
fn the_agent_runs_in_the_work_area_it_was_given() {
    let held = TempDir::new().unwrap();
    let at = held.path().join("area");
    fs::create_dir(&at).unwrap();

    standing_in(&held)
        .work(working(
            &at.display().to_string(),
            "echo here > it-ran-here.txt",
        ))
        .unwrap();

    assert!(at.join("it-ran-here.txt").exists());
}

/// The goal has to lead the prompt.
/// Anywhere else it is read as ordinary text and nothing gates the end of the task.
#[test]
fn the_prompt_leads_with_the_goal_and_the_instruction_follows_it() {
    let held = TempDir::new().unwrap();
    standing_in(&held)
        .work(working(&held.path().display().to_string(), "exit 0"))
        .unwrap();

    let asked = prompt(&held);
    assert!(asked.starts_with("/goal "), "{asked}");
    assert!(asked.ends_with("\n\nexit 0"), "{asked}");
}

/// A run cut off at a ceiling says nothing of its own.
/// The word it gives instead has to become the sentence the definition carries for it.
#[test]
fn a_run_that_was_cut_off_is_reported_as_a_sentence() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            r#"echo '{"is_error":true,"subtype":"error_max_turns","result":null}'; exit 1"#,
        ))
        .unwrap();

    assert_ne!(ended.outcome, Outcome::Finished);
    assert_eq!(
        ended.reason.as_deref(),
        Some("the agent was cut off after 200 turns")
    );
}

/// A run that failed with something to say says it, rather than handing back the object it was written in.
#[test]
fn a_run_that_answered_is_reported_in_its_own_words() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            r#"echo '{"is_error":true,"subtype":"success","result":"I could not build it"}'; exit 1"#,
        ))
        .unwrap();

    assert_eq!(ended.reason.as_deref(), Some("I could not build it"));
}

/// A place nobody filled takes its flag with it.
/// A task that named no model does not hand the agent a flag with nothing after it.
#[test]
fn a_place_nobody_filled_takes_its_flag_with_it() {
    let agent = ProgramAgent::new(Definition::of("claude", None).unwrap());

    let named = agent.arguments(&[
        ("goal", "g"),
        ("instruction", "i"),
        ("model", "haiku"),
        ("turns", "1"),
        ("spend", "2"),
    ]);
    assert!(named.contains(&"--model".to_owned()));
    assert!(named.contains(&"haiku".to_owned()));

    let unnamed = agent.arguments(&[
        ("goal", "g"),
        ("instruction", "i"),
        ("model", ""),
        ("turns", "1"),
        ("spend", "2"),
    ]);
    assert!(!unnamed.contains(&"--model".to_owned()), "{unnamed:?}");
    assert!(unnamed.contains(&"--max-turns".to_owned()), "{unnamed:?}");
}

/// A run still answers when the agent leaves a child of its own behind.
#[test]
fn a_run_that_left_a_child_behind_still_answers() {
    let held = TempDir::new().unwrap();
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            "sleep 60 & echo the agent said this",
        ))
        .unwrap();

    assert_eq!(ended.outcome, Outcome::Finished);
}

/// What the run wrote is still read, though what wrote it is gone by then.
#[test]
fn what_a_run_wrote_survives_the_end_of_its_group() {
    let held = TempDir::new().unwrap();
    let said = answering(&held, AN_ANSWER);
    let ended = standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &format!("sleep 60 & {said}; exit 1"),
        ))
        .unwrap();

    assert!(matches!(ended.observed, Observed::Spent(_)), "{ended:?}");
}

/// Nothing the run started is left once it has been answered for.
#[test]
fn nothing_of_the_run_outlives_the_answer() {
    let held = TempDir::new().unwrap();
    let marker = held.path().join("still-here");
    standing_in(&held)
        .work(working(
            &held.path().display().to_string(),
            &format!("(sleep 3; touch '{}') & echo done", marker.display()),
        ))
        .unwrap();

    // Longer than the child would have taken, had it lived.
    thread::sleep(Duration::from_secs(5));
    assert!(!marker.exists(), "something the run started outlived it");
}

#[test]
fn a_program_that_is_not_installed_fails_rather_than_answering() {
    let held = TempDir::new().unwrap();
    let mut definition = Definition::of("claude", None).unwrap();
    definition.program = "no-such-agent-anywhere".to_owned();
    let refused = ProgramAgent::new(definition)
        .work(working(&held.path().display().to_string(), "exit 0"))
        .unwrap_err();

    assert!(
        refused.reason.contains("no-such-agent-anywhere"),
        "{}",
        refused.reason
    );
}

/// A task's instruction is a person's own words, not a template.
///
/// Filling each name in turn over the whole string would leave the instruction open to every
/// name filled after it, and the model is one of those.
#[test]
fn a_place_written_by_one_name_is_not_filled_again_by_the_next() {
    let filling = [("instruction", "rename {model} to X"), ("model", "haiku")];

    assert_eq!(fill("{instruction}", &filling), "rename {model} to X");
    assert_eq!(fill("{model}", &filling), "haiku");
}

#[test]
fn a_name_nothing_fills_stays_as_it_was_written() {
    let filling = [("model", "haiku")];

    assert_eq!(fill("--{whatever}", &filling), "--{whatever}");
    assert_eq!(fill("{model", &filling), "{model");
    assert_eq!(fill("plain", &filling), "plain");
}

/// A name with nothing behind it empties the argument, and `arguments` drops the group it was
/// in. That is how a task naming no model loses `--model` along with the value.
#[test]
fn a_name_filled_with_nothing_leaves_the_argument_empty() {
    let filling = [("model", "")];

    assert_eq!(fill("{model}", &filling), "");
}

/// A `{` nothing closes is not a place, and what came before it was written once already.
///
/// Only an unmatched brace at the very start was covered, which is the one shape where the
/// prefix is empty and writing it twice cannot be seen.
#[test]
fn an_unmatched_brace_is_written_once() {
    let filling = [("model", "haiku")];

    assert_eq!(super::fill("--flag={x", &filling), "--flag={x");
    assert_eq!(super::fill("{model} {", &filling), "haiku {");
    assert_eq!(super::fill("{", &filling), "{");
    assert_eq!(super::fill("a{b{c", &filling), "a{b{c");
}

/// The session's own figure, not the definition's, once there is one.
#[test]
fn a_run_the_session_put_a_ceiling_on_is_held_to_that() {
    let held = TempDir::new().unwrap();
    let agent = standing_in(&held);

    agent
        .work(Work {
            ceiling: Some("2500000"),
            ..working(held.path().to_str().unwrap(), "true")
        })
        .unwrap();

    let said = given(&held);
    assert!(said.contains("--max-budget-usd\n2.50\n"), "{said}");
    assert!(!said.contains("\n20\n"), "{said}");
}

/// Without one, the definition's figure stands. That one is a guard against a run that goes
/// nowhere rather than a session's budget.
#[test]
fn a_run_with_no_ceiling_is_held_to_what_the_definition_carries() {
    let held = TempDir::new().unwrap();
    let agent = standing_in(&held);

    agent
        .work(working(held.path().to_str().unwrap(), "true"))
        .unwrap();

    let said = given(&held);
    assert!(said.contains("--max-budget-usd\n20\n"), "{said}");
}
