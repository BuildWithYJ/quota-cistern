//! Claude Code, run without anyone to answer it.
//!
//! The only place that knows the program, its arguments, and what it writes.
//! None of that reaches the core.

use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::core::port::outbound::{Agent, Ended, Unavailable, Work};

/// The vendor agent `docs/cli.md` section 2.5 names.
const PROGRAM: &str = "claude";

/// When the agent has finished, in the words its own evaluator judges.
///
/// The agent decides on its own when it is done, and nothing checks that
/// judgement. Stating the end as a condition puts a second model between the
/// agent and its own claim: after every turn it reads what the agent has
/// shown and says whether this holds, and the agent works again when it does
/// not. The condition has to be something the agent's own output can show,
/// which is why it names what git would report rather than what was asked for.
///
/// The task's instruction follows this. The command has to lead the prompt or
/// it is read as ordinary text and nothing gates anything.
const FINISHED: &str = "/goal The task described below is finished, its result is \
committed on the current branch, and git status reports a clean tree.";

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

/// Runs the agent as a child process and waits for it.
pub struct ClaudeAgent {
    program: String,
}

impl ClaudeAgent {
    pub fn new() -> Self {
        ClaudeAgent {
            program: PROGRAM.to_owned(),
        }
    }
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
            // Nobody is there to answer, so a prompt would hold the task open
            // until the session ended. What limits the damage is the work area
            // and the branch, which belong to this task alone.
            .args(["--permission-mode", "bypassPermissions"])
            // Read as one object at the end. Reading usage out of it and
            // keeping it as a trace are their own issues.
            .args(["--output-format", "json"])
            .args(["--max-turns", TURNS])
            .args(["--max-budget-usd", SPEND])
            .args(["-p", &format!("{FINISHED}\n\n{}", work.instruction)])
            // A child that inherited this could read what a surface is sending
            // the core, and would wait on it forever if it tried.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(model) = work.model {
            running.args(["--model", model]);
        }

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

        ClaudeAgent {
            program: program.display().to_string(),
        }
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

    #[test]
    fn a_program_that_is_not_installed_fails_rather_than_answering() {
        let held = TempDir::new().unwrap();
        let refused = ClaudeAgent {
            program: "no-such-agent-anywhere".to_owned(),
        }
        .work(working(&held.path().display().to_string(), "exit 0"))
        .unwrap_err();

        assert!(
            refused.reason.contains("no-such-agent-anywhere"),
            "{}",
            refused.reason
        );
    }
}
