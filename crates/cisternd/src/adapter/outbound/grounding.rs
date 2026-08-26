//! The repository, asked whether what a spec says about it is so.
//!
//! Two different outsides answer the one port. Where a place reaches is git's to say, and whether
//! a success condition passes is the command's, so this holds a git invocation and a run of
//! something that is not git. It sits here rather than under `git` because that module promises
//! every git invocation and nothing else.
//!
//! **What is run, and what is not.** The command is split on whitespace and the program is started
//! directly. No shell stands between, so `;`, `&&`, a redirect, and a glob are all just arguments
//! that no program takes. The first word has to name something already on this machine before
//! anything starts, so a command a model invented does not run at all.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::core::port::outbound::{Grounding, Ran};

/// How often a command still going is looked at again.
///
/// A test that finishes quickly is waited on for about this long and no longer; one that runs to
/// its whole limit is looked at a few hundred times, which costs nothing next to the test.
const LOOKS_EVERY: Duration = Duration::from_millis(50);

/// Whether a spec names things this repository and this machine actually have.
pub struct RepositoryGrounding;

impl Grounding for RepositoryGrounding {
    fn reaches(&self, repository: &str, place: &str) -> Option<usize> {
        // Asked of what the repository tracks rather than of the filesystem: a path that exists
        // and is ignored is not somewhere a run's work would survive.
        let done = Command::new("git")
            .args(["-C", repository, "--no-pager", "ls-files", "--", place])
            .output()
            .ok()?;
        if !done.status.success() {
            return None;
        }
        let held = String::from_utf8_lossy(&done.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        // Nothing tracked is a place that was invented, which is told apart from one that is
        // there and empty by there being no such thing as a tracked empty directory.
        (held > 0).then_some(held)
    }

    fn runnable(&self, repository: &str, command: &str) -> bool {
        program(repository, command).is_some()
    }

    fn run(&self, repository: &str, command: &str, within: Duration) -> Ran {
        let Some(program) = program(repository, command) else {
            return Ran::Unknown;
        };
        let started = Command::new(program)
            .args(command.split_whitespace().skip(1))
            .current_dir(repository)
            // It is being asked a question, not given a turn at the terminal.
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut running) = started else {
            return Ran::Unknown;
        };

        let until = Instant::now() + within;
        loop {
            match running.try_wait() {
                Ok(Some(ended)) => {
                    return match ended.success() {
                        true => Ran::Passed,
                        false => Ran::Failed,
                    };
                }
                Err(_) => return Ran::Unknown,
                Ok(None) => {}
            }
            if Instant::now() >= until {
                // A run of the gate is not a run of the work. What it would have said had it been
                // left to finish is not worth the wait, and is not worth leaving behind either.
                let _ = running.kill();
                let _ = running.wait();
                return Ran::Unknown;
            }
            thread::sleep(LOOKS_EVERY);
        }
    }
}

/// The program a command starts, where this machine already has one.
///
/// A path inside the repository, or a name on the path. Nothing else: a command whose first word
/// names nothing that is already here was invented, and inventing it is the answer.
fn program(repository: &str, command: &str) -> Option<PathBuf> {
    let first = command.split_whitespace().next()?;
    if first.contains('/') {
        let inside = Path::new(repository).join(first);
        return runs(&inside).then_some(inside);
    }
    env::var_os("PATH")
        .iter()
        .flat_map(env::split_paths)
        .map(|dir| dir.join(first))
        .find(|held| runs(held))
}

/// Whether the file at the path is one this machine would start.
fn runs(path: &Path) -> bool {
    #[cfg(unix)]
    let runs = {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|held| held.is_file() && held.permissions().mode() & 0o111 != 0)
    };
    // A file is started by its extension there rather than by a bit on the file.
    #[cfg(not(unix))]
    let runs = path.is_file();
    runs
}

#[cfg(test)]
mod tests;
