//! Claude Code, run without anyone to answer it.
//!
//! The only place that knows the program, its arguments, and what it writes.
//! None of that reaches the core.

use std::process::{Command, Stdio};

use crate::core::port::outbound::{Agent, Ended, Unavailable, Work};

/// The vendor agent `docs/cli.md` section 2.5 names.
const PROGRAM: &str = "claude";

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
            .args(["-p", work.instruction])
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
/// It writes what went wrong to standard error, and says on standard output
/// what it would have answered with, so both are read rather than reporting a
/// number nobody can act on.
fn said(status: &std::process::ExitStatus, stderr: &[u8], stdout: &[u8]) -> String {
    let complained = String::from_utf8_lossy(stderr);
    let complained = complained.trim();
    if !complained.is_empty() {
        return complained.to_owned();
    }

    let answered = String::from_utf8_lossy(stdout);
    let answered = answered.trim();
    match answered.is_empty() {
        false => answered.to_owned(),
        true => format!("the agent {status} and said nothing"),
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
        fs::write(
            &program,
            "#!/bin/sh\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = -p ]; then\n    shift\n    exec /bin/sh -c \"$1\"\n  fi\n  shift\ndone\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();

        ClaudeAgent {
            program: program.display().to_string(),
        }
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
