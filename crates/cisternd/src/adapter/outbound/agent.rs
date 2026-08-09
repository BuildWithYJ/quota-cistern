//! Claude Code, run without anyone to answer it.
//!
//! The only place that knows the program, its arguments, and what it writes.
//! None of that reaches the core.

use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::core::port::outbound::{Agent, Ended, Unavailable, Work};

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

/// The format the answer arrives in.
///
/// This stands here rather than beside the other arguments because [`said`]
/// reads that one format. Two places holding one agreement is two places to
/// forget.
const FORMAT: [&str; 2] = ["--output-format", "json"];

/// Runs the agent as a child process and waits for it.
pub struct ClaudeAgent {
    program: String,
    /// The arguments as they were written, with the places still in them.
    args: Vec<Vec<String>>,
    goal: String,
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

/// What the agent answers with, of which this reads only how it ended.
///
/// Reading what it consumed and keeping what it did are their own issues, and
/// both read the same object.
#[derive(Debug, Deserialize)]
struct Answered {
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    result: Option<String>,
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
            .stderr(Stdio::piped());

        // Both pipes are read while the child writes, so a child that writes
        // more than a pipe holds carries on rather than stopping where it is.
        let done = running
            .output()
            .map_err(|e| Unavailable::new(format!("{}: {e}", self.program)))?;

        Ok(match done.status.success() {
            true => Ended {
                done: true,
                reason: None,
            },
            false => Ended {
                done: false,
                reason: Some(said(&done.status, &done.stderr, &done.stdout)),
            },
        })
    }
}

/// What the agent said about a run that failed.
///
/// A run cut off at a guard fails with its answer on standard output and
/// nothing on standard error, so reading the whole of that answer back would
/// put an object nobody can read where a sentence belongs.
fn said(status: &std::process::ExitStatus, stderr: &[u8], stdout: &[u8]) -> String {
    let complained = String::from_utf8_lossy(stderr);
    let complained = complained.trim();
    if !complained.is_empty() {
        return complained.to_owned();
    }

    let answered = String::from_utf8_lossy(stdout);
    if let Ok(answered) = serde_json::from_str::<Answered>(&answered) {
        if let Some(said) = answered.result.filter(|said| !said.trim().is_empty()) {
            return said.trim().to_owned();
        }
        if let Some(why) = answered.subtype {
            return why_for(&why);
        }
    }

    let answered = answered.trim();
    match answered.is_empty() {
        false => answered.to_owned(),
        true => format!("the agent {status} and said nothing"),
    }
}

/// A sentence for how the agent says it stopped.
///
/// A run cut off at a guard answers with no text of its own, and the name it
/// gives instead is the only thing there is to report.
fn why_for(subtype: &str) -> String {
    match subtype {
        "error_max_turns" => format!("the agent was cut off after {TURNS} turns"),
        "error_max_budget" => format!("the agent was cut off at {SPEND} dollars"),
        other => format!("the agent stopped with {other}"),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;

    /// An agent that is not the vendor's, so that what this file does can be
    /// checked without running one.
    ///
    /// It takes the arguments this file passes, ignores all of them but the
    /// instruction, and runs that instruction as a shell command. A program
    /// that refused the arguments would fail every one of these for the same
    /// reason and prove nothing about any of them.
    fn standing_in(held: &TempDir) -> ClaudeAgent {
        let program = held.path().join("agent");
        let saw = held.path().join("prompt");
        fs::write(
            &program,
            // The prompt leads with the goal and the instruction follows a
            // blank line, so the last line is what a test asked for. Where the
            // prompt is written is fixed here, since a test cannot see the
            // arguments a child was given any other way.
            format!(
                "#!/bin/sh\n\
                 while [ $# -gt 0 ]; do\n\
                 \x20 if [ \"$1\" = -p ]; then\n\
                 \x20   shift\n\
                 \x20   printf '%s' \"$1\" > '{saw}'\n\
                 \x20   exec /bin/sh -c \"$(printf '%s' \"$1\" | tail -n 1)\"\n\
                 \x20 fi\n\
                 \x20 shift\n\
                 done\n\
                 exit 0\n",
                saw = saw.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();

        let mut standing = ClaudeAgent::new().unwrap();
        standing.program = program.display().to_string();
        standing
    }

    fn prompt(held: &TempDir) -> String {
        fs::read_to_string(held.path().join("prompt")).unwrap()
    }

    fn working<'a>(at: &'a str, instruction: &'a str) -> Work<'a> {
        Work {
            at,
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

        assert_eq!(
            ended,
            Ended {
                done: true,
                reason: None
            }
        );
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

        assert!(!ended.done);
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

        assert!(ended.done, "{ended:?}");
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

        assert!(!ended.done);
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
