//! Claude Code, run without anyone to answer it.
//!
//! The only place that knows the program, its arguments, and what it writes.
//! None of that reaches the core.

use std::{
    collections::HashMap,
    fs,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    sync::{Mutex, PoisonError},
    thread,
    time::Duration,
};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde::Deserialize;
use serde_json::Value;

use crate::core::port::outbound::{Agent, Ended, Observed, Outcome, Spent, Unavailable, Work};

/// How the agent is invoked, and what it is told it is finished.
///
/// Both are read at the moment this is built rather than on every task, and
/// both travel in the binary. What they hold is content: a sentence a person
/// will tune and a list of arguments a person will read when a run behaves
/// wrongly. What is left in this file is what happens to the answer.
const INVOCATION: &str = include_str!("claude.json");
const GOAL: &str = include_str!("claude-goal.txt");

/// How many turns one task may take before it is cut off.
///
/// A guard against a run that goes nowhere, not the ceiling section 1 names. A
/// task's own ceiling is measured in what it consumed and is worked out from
/// the session's budget, which is the supervisor's.
const TURNS: &str = "200";

/// How much one task may spend before it is cut off, in dollars.
///
/// The same guard, against the same runaway, in the other unit. The figure the
/// agent counts against is its own estimate.
const SPEND: &str = "20";

/// What the agent calls the two ceilings it stops at.
const AT_TURNS: &str = "error_max_turns";
const AT_SPEND: &str = "error_max_budget";

/// The format the answer arrives in.
///
/// One object per line, the last of which is the answer. The lines before it
/// are what the run did on the way, which is what a trace is made of. This
/// stands here rather than beside the other arguments because what reads the
/// output expects this one format, and two places holding one agreement is
/// two places to forget.
///
/// The vendor refuses the format without `--verbose` when it is not talking
/// to a person.
const FORMAT: [&str; 3] = ["--output-format", "stream-json", "--verbose"];

/// How long a run has to end on its own before it is made to.
///
/// A run commits as it goes, so there is little for it to tidy up, and
/// whoever asked for it to stop is waiting for the command to come back.
const TIDIES_UP_IN: Duration = Duration::from_secs(2);

/// Runs the agent as a child process and waits for it.
pub struct ClaudeAgent {
    program: String,
    /// The arguments as they were written, with the places still in them.
    args: Vec<Vec<String>>,
    goal: String,
    /// The process group each running task was given, by task.
    ///
    /// A run is ended from whichever thread took the command that asked for
    /// it, not from the one waiting on the run, so where to send that has to
    /// be somewhere both can reach.
    running: Mutex<HashMap<String, u32>>,
}

/// The invocation as its file holds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    /// What the file says about itself. Read by people, not by this.
    #[serde(rename = "_", default)]
    _about: Vec<String>,
    program: String,
    args: Vec<Vec<String>>,
}

impl ClaudeAgent {
    /// Reads how the agent is invoked.
    ///
    /// Failing here stops the daemon starting, which beats failing on every
    /// task that arrives. The file travels in the binary, so this can only
    /// fail on a build nobody should have made, and a test says so.
    pub fn new() -> Result<Self, Unavailable> {
        let read: Invocation = serde_json::from_str(INVOCATION)
            .map_err(|e| Unavailable::new(format!("claude.json: {e}")))?;

        Ok(ClaudeAgent {
            program: read.program,
            args: read.args,
            goal: GOAL.trim().to_owned(),
            running: Mutex::new(HashMap::new()),
        })
    }

    /// The arguments with every place filled, and every group that holds an
    /// empty one dropped.
    ///
    /// A task that named no model loses `--model` along with the value, which
    /// is why the file groups a flag with what follows it.
    fn arguments(&self, filling: &[(&str, &str)]) -> Vec<String> {
        let mut given = Vec::with_capacity(self.args.len() * 2);

        for group in &self.args {
            let filled: Vec<String> = group.iter().map(|token| fill(token, filling)).collect();
            if filled.iter().any(String::is_empty) {
                continue;
            }
            given.extend(filled);
        }
        given
    }
}

/// One argument with `{name}` replaced by what was given for it.
fn fill(token: &str, filling: &[(&str, &str)]) -> String {
    let mut written = token.to_owned();
    for (name, value) in filling {
        written = written.replace(&format!("{{{name}}}"), value);
    }
    written
}

/// What the agent counted for one model, under the names it uses.
///
/// This is the per-model breakdown rather than the run's own `usage`. A run
/// makes calls that leave no message behind, and `usage` counts only the ones
/// that do, while the price the same answer gives covers all of them. Reading
/// the breakdown keeps the counts and the price describing one thing.
///
/// No field is given a default. A vendor that renames one leaves this
/// unreadable rather than answering that the run consumed nothing, which is
/// what a budget would read as untouched.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Counted {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

/// A dollar figure as millionths of one.
const MILLIONTHS: f64 = 1_000_000.0;

/// What the agent said it consumed.
///
/// The answer is read as a whole first, so that a figure this cannot read
/// leaves the sentence about how the run ended where it was.
///
/// One run may reach for more than one model, so every model the answer names
/// is added up. Which of the kinds count against a budget is the supervisor's,
/// and they stay apart as far as the store.
fn observed(answer: Option<&Value>) -> Observed {
    let Some(answer) = answer else {
        return unreadable("the agent answered with nothing this could read");
    };

    let Some(counted) = answer.get("modelUsage").and_then(Value::as_object) else {
        return unreadable("the agent's answer said nothing about what it consumed");
    };
    if counted.is_empty() {
        return unreadable("the agent's answer named no model to have consumed anything");
    }

    let mut spent = [0u64; 4];
    for (_model, held) in counted {
        let Ok(held) = serde_json::from_value::<Counted>(held.clone()) else {
            return unreadable("what the agent counted is not in the shape this reads");
        };
        for (total, one) in spent.iter_mut().zip([
            held.input_tokens,
            held.output_tokens,
            held.cache_creation_input_tokens,
            held.cache_read_input_tokens,
        ]) {
            *total = total.saturating_add(one);
        }
    }

    let Some(priced) = answer.get("total_cost_usd").and_then(Value::as_f64) else {
        return unreadable("the agent's answer put no figure on what it consumed");
    };

    Observed::Spent(Spent {
        input: spent[0].to_string(),
        output: spent[1].to_string(),
        cache_written: spent[2].to_string(),
        cache_read: spent[3].to_string(),
        cost: ((priced * MILLIONTHS).round().max(0.0) as u64).to_string(),
    })
}

fn unreadable(why: &str) -> Observed {
    Observed::Unreadable {
        why: why.to_owned(),
    }
}

impl Agent for ClaudeAgent {
    fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable> {
        let mut running = Command::new(&self.program);
        running
            .current_dir(work.at)
            .args(self.arguments(&[
                ("goal", &self.goal),
                ("instruction", work.instruction),
                ("model", work.model.unwrap_or_default()),
                ("turns", TURNS),
                ("spend", SPEND),
            ]))
            .args(FORMAT)
            // A child that inherited this could read what a surface is sending
            // the core, and would wait on it forever if it tried.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A group of its own, so that everything the run started can be
            // ended together. An agent starts children, and those start their
            // own; ending the one this holds leaves the rest.
            .process_group(0);

        let mut child = running
            .spawn()
            .map_err(|e| Unavailable::new(format!("{}: {e}", self.program)))?;
        let group = child.id();
        self.went(work.task, group);

        // Both pipes are read while the child writes, so a child that writes
        // more than a pipe holds carries on rather than stopping where it is.
        // What comes back on the first is kept as it arrives, since whoever
        // is watching the run reads it before the run has ended.
        let stdout = keeping(child.stdout.take(), work.trace.to_owned());
        let stderr = reading(child.stderr.take());

        let status = child
            .wait()
            .map_err(|e| Unavailable::new(format!("{}: {e}", self.program)))?;

        // Whatever the agent started outlives it and holds the writing end of
        // those pipes. Waiting for the reading to finish before the group is
        // gone would be waiting on a child nobody is watching.
        self.gone(work.task);
        end(group);

        let (stdout, stderr) = (said_by(stdout), said_by(stderr));

        // Read once. How the run ended and what it consumed are two questions
        // about one object, and one of them failing must not lose the other.
        let answer = serde_json::from_slice::<Value>(&stdout).ok();

        let outcome = outcome_of(status.success(), answer.as_ref());
        Ok(Ended {
            outcome,
            reason: match outcome {
                Outcome::Finished => None,
                _ => Some(said(&status, &stderr, answer.as_ref())),
            },
            observed: observed(answer.as_ref()),
        })
    }

    fn stop(&self, task: &str) {
        let Some(group) = self.gone(task) else {
            return;
        };
        end(group);
    }
}

impl ClaudeAgent {
    /// Remembers which group a task's run was given.
    fn went(&self, task: &str, group: u32) {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(task.to_owned(), group);
    }

    /// Forgets it, and says what it was.
    fn gone(&self, task: &str) -> Option<u32> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(task)
    }
}

/// Ends a process group, and makes sure of it.
///
/// The first signal is one a process may act on. A group with nothing left in
/// it is done there, which is every run that ended on its own. Whatever is
/// still there after that is not going to act on it, and whoever asked for
/// this is waiting.
fn end(group: u32) {
    signal(Signal::SIGTERM, group);
    if !still_there(group) {
        return;
    }
    thread::sleep(TIDIES_UP_IN);
    signal(Signal::SIGKILL, group);
}

/// Whether anything in the group is still running.
fn still_there(group: u32) -> bool {
    signal(None, group)
}

/// Signals a whole process group, and says whether there was one.
///
/// Not through the `kill` program. The two systems this runs on disagree about
/// what a negative number on its command line means: one reads it as the group
/// to signal and the other as the signal to send, so on one of them nothing
/// was signalled and nothing said so. Nothing is sent when no signal is given,
/// which only asks whether the group is there.
fn signal(with: impl Into<Option<Signal>>, group: u32) -> bool {
    let Ok(group) = i32::try_from(group) else {
        return false;
    };
    killpg(Pid::from_raw(group), with).is_ok()
}

/// Reads what the run says on a thread of its own, keeping each line as it
/// arrives and handing back the last one.
///
/// The last line is the answer. Everything before it is what the run did on
/// the way there, which nobody waits for.
fn keeping<R: std::io::Read + Send + 'static>(
    held: Option<R>,
    at: String,
) -> thread::JoinHandle<Vec<u8>> {
    use std::io::{BufRead, BufReader, Write};

    thread::spawn(move || {
        let Some(held) = held else {
            return Vec::new();
        };
        let mut keeping = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&at)
            .ok();
        let mut last = String::new();

        for line in BufReader::new(held).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            if let Some(keeping) = keeping.as_mut() {
                let _ = writeln!(keeping, "{}\t{line}", now());
            }
            last = line;
        }
        last.into_bytes()
    })
}

/// Seconds since the epoch, for stamping a line as it arrives.
///
/// The vendor stamps some of its lines and not others, and a trace is read in
/// one order however it was written.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Reads one of the run's pipes on a thread of its own.
fn reading<R: std::io::Read + Send + 'static>(held: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut said = Vec::new();
        if let Some(mut held) = held {
            let _ = held.read_to_end(&mut said);
        }
        said
    })
}

/// What that pipe carried, once nobody is left to write to it.
fn said_by(reading: thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    reading.join().unwrap_or_default()
}

/// What the agent said about a run that failed.
///
/// A run cut off at a guard fails with its answer on standard output and
/// nothing on standard error, so reading the whole of that answer back would
/// put an object nobody can read where a sentence belongs.
fn said(status: &std::process::ExitStatus, stderr: &[u8], answer: Option<&Value>) -> String {
    let complained = String::from_utf8_lossy(stderr);
    let complained = complained.trim();
    if !complained.is_empty() {
        return complained.to_owned();
    }

    if let Some(answer) = answer {
        if let Some(said) = answer.get("result").and_then(Value::as_str)
            && !said.trim().is_empty()
        {
            return said.trim().to_owned();
        }
        if let Some(why) = answer.get("subtype").and_then(Value::as_str) {
            return why_for(why);
        }
    }
    format!("the agent {status} and said nothing")
}

/// How the run came to an end.
///
/// The name the agent gives for stopping is what tells a ceiling from a
/// failure. Which names mean what is this file's to know.
fn outcome_of(finished: bool, answer: Option<&Value>) -> Outcome {
    if finished {
        return Outcome::Finished;
    }
    match answer
        .and_then(|answer| answer.get("subtype"))
        .and_then(Value::as_str)
    {
        Some(AT_TURNS | AT_SPEND) => Outcome::AtCeiling,
        _ => Outcome::Failed,
    }
}

/// A sentence for how the agent says it stopped.
///
/// A run cut off at a guard answers with no text of its own, and the name it
/// gives instead is the only thing there is to report.
fn why_for(subtype: &str) -> String {
    match subtype {
        AT_TURNS => format!("the agent was cut off after {TURNS} turns"),
        AT_SPEND => format!("the agent was cut off at {SPEND} dollars"),
        other => format!("the agent stopped with {other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// The agent that stands in for the vendor's. A shell program, kept as one
    /// rather than as a string this file would have to escape.
    const STANDING_IN: &str = include_str!("standing-agent.sh");

    fn standing_in(held: &TempDir) -> ClaudeAgent {
        let program = held.path().join("agent");
        let saw = held.path().join("prompt");
        fs::write(
            &program,
            STANDING_IN.replace("{prompt}", &saw.display().to_string()),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        wait_until_runnable(&program);

        let mut standing = ClaudeAgent::new().unwrap();
        standing.program = program.display().to_string();
        standing
    }

    /// Waits for the file just written to be one this may run.
    ///
    /// Tests run beside each other, and a thread that starts a child while
    /// another is still writing a file leaves that file open in the child.
    /// Running a file that is open for writing is refused, so this asks until
    /// it is not. The stand-in given no arguments does nothing and exits.
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

    /// An answer a vendor actually sent, kept as it arrived apart from a
    /// shortened `result`. It holds the fields this file reads and a dozen it
    /// does not, which is what makes a test say that a vendor adding one
    /// changes nothing.
    const AN_ANSWER: &str = include_str!("claude-answer.json");

    /// The answer with something about it changed.
    ///
    /// The object is edited rather than its text, so that a fixture kept as a
    /// vendor sent it can be reshaped without a test knowing how it is laid
    /// out.
    fn answer_with(change: impl FnOnce(&mut Value)) -> String {
        let mut answer: Value = serde_json::from_str(AN_ANSWER).unwrap();
        change(&mut answer);
        answer.to_string()
    }

    /// A shell command that answers with `written` and nothing else.
    ///
    /// Written to a file and read back rather than quoted into the command, so
    /// that an answer can be kept as an object a person can read.
    fn answering(held: &TempDir, written: &str) -> String {
        // One line, as the vendor writes it. The file it comes from is laid
        // out for a person to read.
        let one: Value = serde_json::from_str(written).unwrap();
        let at = held.path().join("answer.json");
        fs::write(&at, one.to_string()).unwrap();
        format!("cat '{}'", at.display())
    }

    fn prompt(held: &TempDir) -> String {
        fs::read_to_string(held.path().join("prompt")).unwrap()
    }

    fn working<'a>(at: &'a str, instruction: &'a str) -> Work<'a> {
        Work {
            task: "1",
            at,
            // Kept beside the work area, so a test that looks at what was
            // written knows where to look.
            trace: TRACE.get_or_init(|| format!("{at}/trace.jsonl")),
            instruction,
            model: None,
        }
    }

    /// Where the stand-in's runs keep what they wrote.
    static TRACE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

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
    fn what_the_agent_counted_is_read_out_of_its_answer() {
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

    /// The run's own `usage` counts only the calls that left a message behind,
    /// and this answer shows the gap: 3182 against 3377. Reading it would put a
    /// count beside a price that covers more than the count does.
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

    /// A run may reach for more than one model, and the four kinds are the sum
    /// over all of them.
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

    /// A vendor that renames a field would otherwise report a run that spent a
    /// hundred thousand tokens as having spent none.
    #[test]
    fn a_count_under_a_name_this_does_not_know_is_not_a_count_of_nothing() {
        let held = TempDir::new().unwrap();
        // One of the four, so that what is caught is a name this does not
        // know rather than an answer that lost its whole count.
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

    /// A run that was cut off still consumed what it consumed, and how it ended
    /// is read out of the same answer as what it spent.
    #[test]
    fn a_run_that_failed_still_reports_what_it_consumed() {
        let held = TempDir::new().unwrap();
        // A run cut off at a guard names what stopped it and answers with no
        // text of its own.
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

    /// The child stops where it is when nobody reads what it writes, and a task
    /// that stops there never ends.
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

    /// The agent runs where the task's work area is, not where the core was
    /// started.
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

    /// The command has to lead the prompt. Anywhere else it is read as
    /// ordinary text and nothing gates the end of the task.
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

    /// A run cut off at a guard says nothing of its own, so the name it gives
    /// instead has to become a sentence rather than the whole answer.
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

    /// A run that failed with something to say says it, rather than handing
    /// back the object it was written in.
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

    /// The file travels in the binary, so a build that broke it would fail
    /// every task. This is what keeps that from reaching one.
    #[test]
    fn the_invocation_that_ships_is_readable_and_holds_the_places_it_must() {
        let agent = ClaudeAgent::new().unwrap();
        assert_eq!(agent.program, "claude");
        assert!(agent.goal.starts_with("/goal "), "{}", agent.goal);

        let written: String = agent.args.iter().flatten().cloned().collect();
        for place in ["{goal}", "{instruction}", "{model}", "{turns}", "{spend}"] {
            assert!(written.contains(place), "{place} is not in claude.json");
        }
    }

    /// A group holding a place nobody filled goes whole, so a task that named
    /// no model does not hand the agent a flag with nothing after it.
    #[test]
    fn a_place_nobody_filled_takes_its_flag_with_it() {
        let agent = ClaudeAgent::new().unwrap();

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

    /// An agent that leaves something of its own behind. Ending the agent
    /// alone leaves that holding the pipe this reads, and reading it to the
    /// end would never finish.
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
        let mut nowhere = ClaudeAgent::new().unwrap();
        nowhere.program = "no-such-agent-anywhere".to_owned();
        let refused = nowhere
            .work(working(&held.path().display().to_string(), "exit 0"))
            .unwrap_err();

        assert!(
            refused.reason.contains("no-such-agent-anywhere"),
            "{}",
            refused.reason
        );
    }
}
