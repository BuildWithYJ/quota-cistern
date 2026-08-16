//! An agent run as a child process, with nobody to answer it.
//!
//! Which program, what to hand it, and where each figure sits in its answer all come from
//! the definition. What is here is the part that does not change with the vendor: starting
//! the child, ending its process group, and reading its pipes while it writes.

use std::{
    collections::HashMap,
    os::unix::process::CommandExt,
    process::{Command, ExitStatus, Stdio},
    sync::{Mutex, PoisonError},
    thread,
    time::Duration,
};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde_json::Value;

use crate::core::port::outbound::{
    Agent, Ended, Keeping, Observed, Outcome, Spent, Unavailable, Work,
};

use super::{Definition, definition::Reader, path};

/// How long a run has to end on its own before it is made to.
///
/// A run commits as it goes, so there is little for it to tidy up.
/// Whoever asked for it to stop is waiting for the command to come back.
const TIDIES_UP_IN: Duration = Duration::from_secs(2);

/// Runs the program a definition describes and waits for it.
pub struct ProgramAgent {
    definition: Definition,
    /// The process group each running task was given, by task.
    ///
    /// A run is ended from the thread that took the command, not the one waiting on the run.
    /// Both have to reach this.
    running: Mutex<HashMap<String, u32>>,
}

impl ProgramAgent {
    pub fn new(definition: Definition) -> Self {
        ProgramAgent {
            definition,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// The arguments with every place filled, and every group that holds an empty one dropped.
    ///
    /// A task that named no model loses `--model` along with the value.
    /// That is why the definition groups a flag with what follows it.
    fn arguments(&self, filling: &[(&str, &str)]) -> Vec<String> {
        let mut given = Vec::with_capacity(self.definition.args.len() * 2);

        for group in &self.definition.args {
            let filled: Vec<String> = group.iter().map(|token| fill(token, filling)).collect();
            if filled.iter().any(String::is_empty) {
                continue;
            }
            given.extend(filled);
        }
        given
    }

    /// What the definition says to fill in, apart from the task's own words.
    fn its_own(&self) -> [(&str, &str); 3] {
        [
            ("goal", self.definition.goal.trim()),
            ("turns", &self.definition.turns),
            ("spend", &self.definition.spend),
        ]
    }

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

/// One argument with `{name}` replaced by what was given for it.
fn fill(token: &str, filling: &[(&str, &str)]) -> String {
    let mut written = token.to_owned();
    for (name, value) in filling {
        written = written.replace(&format!("{{{name}}}"), value);
    }
    written
}

impl Agent for ProgramAgent {
    fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable> {
        let program = &self.definition.program;
        let mut filling = self.its_own().to_vec();
        filling.push(("instruction", work.instruction));
        filling.push(("model", work.model.unwrap_or_default()));

        let mut running = Command::new(program);
        running
            .current_dir(work.at)
            .args(self.arguments(&filling))
            // A child that inherited this could read what a surface is sending the core.
            // It would wait on it forever if it tried.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A group of its own.
            // The agent starts children and those start their own, and ending only the one this holds leaves the rest.
            .process_group(0);

        let mut child = running
            .spawn()
            .map_err(|e| Unavailable::new(format!("{program}: {e}")))?;
        let group = child.id();
        self.went(work.task, group);

        // Read while the child writes, so one that outwrites a pipe carries on.
        // A watcher also sees the trace before the run has ended.
        let stdout = keeping(
            child.stdout.take(),
            work.trace,
            self.definition.answer.reader,
        );
        let stderr = reading(child.stderr.take());

        let status = child
            .wait()
            .map_err(|e| Unavailable::new(format!("{program}: {e}")))?;

        // A grandchild outlives the agent holding the writing end of these pipes.
        // The group goes before the reading is waited on.
        self.gone(work.task);
        end(group);

        let (stdout, stderr) = (said_by(stdout), said_by(stderr));

        // Read once.
        // How the run ended and what it consumed are two questions about one object.
        // One of them failing must not lose the other.
        let answer = serde_json::from_slice::<Value>(&stdout).ok();

        let outcome = self.outcome_of(status.success(), answer.as_ref());
        Ok(Ended {
            outcome,
            reason: match outcome {
                Outcome::Finished => None,
                _ => Some(self.said(&status, &stderr, answer.as_ref())),
            },
            observed: self.observed(answer.as_ref()),
        })
    }

    fn stop(&self, task: &str) {
        let Some(group) = self.gone(task) else {
            return;
        };
        end(group);
    }
}

impl ProgramAgent {
    /// How the run came to an end.
    ///
    /// The name the program gives for stopping is what tells a ceiling from a failure, and
    /// which names mean a ceiling is the definition's to say.
    fn outcome_of(&self, finished: bool, answer: Option<&Value>) -> Outcome {
        if finished {
            return Outcome::Finished;
        }
        match self.stopping_word(answer) {
            Some(word) if self.definition.answer.at_ceiling.contains_key(&word) => {
                Outcome::AtCeiling
            }
            _ => Outcome::Failed,
        }
    }

    fn stopping_word(&self, answer: Option<&Value>) -> Option<String> {
        path::text(answer?, &self.definition.answer.outcome)
    }

    /// What the program said about a run that failed.
    ///
    /// A run cut off at a ceiling says nothing on standard error.
    /// Handing back the whole of its answer would put an object where a sentence belongs.
    fn said(&self, status: &ExitStatus, stderr: &[u8], answer: Option<&Value>) -> String {
        let complained = String::from_utf8_lossy(stderr);
        let complained = complained.trim();
        if !complained.is_empty() {
            return complained.to_owned();
        }

        if let Some(answer) = answer {
            if let Some(said) = path::text(answer, &self.definition.answer.said)
                && !said.trim().is_empty()
            {
                return said.trim().to_owned();
            }
            if let Some(word) = self.stopping_word(Some(answer)) {
                return self.why_for(&word);
            }
        }
        format!("the agent {status} and said nothing")
    }

    /// A sentence for how the program says it stopped.
    ///
    /// A run cut off at a ceiling answers with no text of its own, so the definition carries
    /// a sentence for each word it may stop with.
    fn why_for(&self, word: &str) -> String {
        match self.definition.answer.at_ceiling.get(word) {
            Some(sentence) => fill(sentence, &self.its_own()),
            None => format!("the agent stopped with {word}"),
        }
    }

    /// What the program said it consumed.
    ///
    /// The answer is read whole before any figure is, so that one this cannot read leaves
    /// the sentence about how the run ended where it was.
    fn observed(&self, answer: Option<&Value>) -> Observed {
        let Some(answer) = answer else {
            return unreadable("the agent answered with nothing this could read");
        };
        let held = &self.definition.answer;

        let Some(cost) = path::total(answer, &held.cost) else {
            return unreadable("the agent's answer put no figure on what it consumed");
        };
        let counted = [
            &held.input,
            &held.output,
            &held.cache_written,
            &held.cache_read,
        ]
        .map(|at| path::total(answer, at));
        if counted.iter().any(Option::is_none) {
            return unreadable("what the agent counted is not in the shape this reads");
        }

        let whole = |of: Option<f64>| of.unwrap_or_default().round().max(0.0) as u64;
        Observed::Spent(Spent {
            input: whole(counted[0]).to_string(),
            output: whole(counted[1]).to_string(),
            cache_written: whole(counted[2]).to_string(),
            cache_read: whole(counted[3]).to_string(),
            cost: whole(Some(cost * held.cost_scale)).to_string(),
        })
    }
}

fn unreadable(why: &str) -> Observed {
    Observed::Unreadable {
        why: why.to_owned(),
    }
}

/// Ends a process group, and makes sure of it.
///
/// The first signal is one a process may act on, and a group with nothing left is done there.
/// Whatever survives it is not going to act on it, and whoever asked is waiting.
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
/// Not through the `kill` program.
/// The two systems this runs on disagree about what a negative number on its command line means.
/// On one of them nothing was signalled and nothing said so.
/// No signal only asks.
fn signal(with: impl Into<Option<Signal>>, group: u32) -> bool {
    let Ok(group) = i32::try_from(group) else {
        return false;
    };
    killpg(Pid::from_raw(group), with).is_ok()
}

/// Reads what the run says on a thread of its own, keeping each line as it arrives.
///
/// Every line goes to the trace whatever the shape is. What is handed back is the part the
/// answer sits in, which is the one thing the shape decides.
fn keeping<R: std::io::Read + Send + 'static>(
    held: Option<R>,
    mut into: Keeping,
    reader: Reader,
) -> thread::JoinHandle<Vec<u8>> {
    use std::io::{BufRead, BufReader};

    thread::spawn(move || {
        let Some(held) = held else {
            return Vec::new();
        };
        let mut answer = String::new();

        for line in BufReader::new(held).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            into(&line);
            match reader {
                Reader::LastJsonLine => answer = line,
            }
        }
        answer.into_bytes()
    })
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

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// The agent that stands in for a vendor's.
    /// A shell program, kept as one rather than as a string this file would have to escape.
    const STANDING_IN: &str = include_str!("standing-agent.sh");

    /// An answer a vendor actually sent, kept as it arrived apart from a shortened `result`.
    ///
    /// It holds the fields the definition names and a dozen it does not.
    /// That is what makes a test say that a vendor adding one changes nothing.
    const AN_ANSWER: &str = include_str!("an-answer.json");

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

    fn working<'a>(at: &'a str, instruction: &'a str) -> Work<'a> {
        Work {
            task: "1",
            at,
            trace: Box::new(|_line: &str| {}),
            instruction,
            model: None,
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
}
