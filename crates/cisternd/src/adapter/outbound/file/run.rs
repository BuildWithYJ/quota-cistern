//! The ledger of runs.
//!
//! The only place that knows the path and the file format.
//! One line for each run, appended and never rewritten, so the file grows and nothing in it moves.
//! Read and written whole is what the other stores do; this one is not, because a run that ended
//! must not be able to displace a run that ended before it.

use std::{
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::port::outbound::{Run, Runs, StoredConsumption, Unavailable};

use super::{as_number, as_value};

/// What the file is called, beside the two the other stores keep.
const NAMED: &str = "runs.jsonl";

/// The ledger, kept as one JSON object to a line at a path fixed when this is built.
pub struct FileRuns {
    at: PathBuf,
}

/// One line, as JSON sees it.
///
/// A value is held as whatever JSON found rather than as what the field is supposed to take,
/// which is what the other JSON store does and for the same reason.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    task: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    session: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    model: Value,
    started_at: Value,
    ended_at: Value,
    outcome: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    reason: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spent: Option<Counted>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    unreadable: Value,
}

/// What one run consumed, as the file holds it.
///
/// The same shape the backlog keeps, so the two read alike.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Counted {
    input: Value,
    output: Value,
    cache_written: Value,
    cache_read: Value,
    cost: Value,
}

impl FileRuns {
    /// Takes the path it is given.
    /// This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileRuns { at: path }
    }

    /// Beside the backlog, under the directory `docs/cli.md` names.
    pub fn in_data_home() -> Option<Self> {
        let base = match env::var_os("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env::var_os("HOME")?)
                .join(".local")
                .join("share"),
        };
        Some(FileRuns::at(base.join("cistern").join(NAMED)))
    }

    fn failing(&self, at: &Path, e: impl std::fmt::Display) -> Unavailable {
        Unavailable::new(format!("{}: {e}", at.display()))
    }
}

impl Runs for FileRuns {
    fn append(&self, run: Run) -> Result<(), Unavailable> {
        let line = serde_json::to_string(&written(run)).map_err(|e| self.failing(&self.at, e))?;

        if let Some(dir) = self.at.parent() {
            fs::create_dir_all(dir).map_err(|e| self.failing(dir, e))?;
        }
        let mut held = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.at)
            .map_err(|e| self.failing(&self.at, e))?;

        // One write of one line, so that two runs ending at once do not interleave.
        writeln!(held, "{line}").map_err(|e| self.failing(&self.at, e))
    }
}

/// What the core hands over, as the file holds it.
fn written(run: Run) -> Entry {
    Entry {
        task: Value::String(run.task),
        session: as_value(run.session),
        model: as_value(run.model),
        started_at: as_number(&run.started_at),
        ended_at: as_number(&run.ended_at),
        outcome: Value::String(run.outcome),
        reason: as_value(run.reason),
        spent: run.spent.map(counted),
        unreadable: as_value(run.unreadable),
    }
}

fn counted(spent: StoredConsumption) -> Counted {
    Counted {
        input: as_number(&spent.input),
        output: as_number(&spent.output),
        cache_written: as_number(&spent.cache_written),
        cache_read: as_number(&spent.cache_read),
        cost: as_number(&spent.cost),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn in_a_temporary_directory() -> (TempDir, FileRuns) {
        let dir = TempDir::new().unwrap();
        let runs = FileRuns::at(dir.path().join("cistern").join(NAMED));
        (dir, runs)
    }

    fn a_run(task: &str) -> Run {
        Run {
            task: task.to_owned(),
            session: Some("1".to_owned()),
            model: Some("opus".to_owned()),
            started_at: "1786349931".to_owned(),
            ended_at: "1786350090".to_owned(),
            outcome: "completed".to_owned(),
            reason: None,
            spent: Some(StoredConsumption {
                input: "10".to_owned(),
                output: "20".to_owned(),
                cache_written: "30".to_owned(),
                cache_read: "40".to_owned(),
                cost: "50".to_owned(),
            }),
            unreadable: None,
        }
    }

    fn lines(runs: &FileRuns) -> Vec<Value> {
        fs::read_to_string(&runs.at)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn a_run_is_one_line() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(a_run("1")).unwrap();

        assert_eq!(
            lines(&runs),
            [serde_json::json!({
                "task": "1",
                "session": "1",
                "model": "opus",
                "started_at": 1_786_349_931u64,
                "ended_at": 1_786_350_090u64,
                "outcome": "completed",
                "spent": {
                    "input": 10, "output": 20,
                    "cache_written": 30, "cache_read": 40, "cost": 50,
                },
            })]
        );
    }

    /// The whole reason this file is not read and written whole.
    /// `Backlog::record` keeps one run to a task; a second run must not displace the first.
    #[test]
    fn a_second_run_of_a_task_leaves_the_first_where_it_is() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(a_run("1")).unwrap();
        runs.append(Run {
            ended_at: "1786360000".to_owned(),
            ..a_run("1")
        })
        .unwrap();

        let written = lines(&runs);
        assert_eq!(written.len(), 2);
        assert_eq!(written[0]["ended_at"], 1_786_350_090u64);
        assert_eq!(written[1]["ended_at"], 1_786_360_000u64);
    }

    /// A task on its way back to waiting names no session, and what it spent is still its own.
    #[test]
    fn a_run_that_names_no_session_leaves_the_field_out() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(Run {
            session: None,
            model: None,
            ..a_run("2")
        })
        .unwrap();

        let written = &lines(&runs)[0];
        assert!(written.get("session").is_none(), "{written}");
        assert!(written.get("model").is_none(), "{written}");
    }

    /// A run whose answer could not be read is not a run that consumed nothing.
    #[test]
    fn a_run_nobody_could_read_says_so_and_counts_nothing() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(Run {
            outcome: "error".to_owned(),
            reason: Some("the agent was killed".to_owned()),
            spent: None,
            unreadable: Some("no last line".to_owned()),
            ..a_run("3")
        })
        .unwrap();

        let written = &lines(&runs)[0];
        assert!(written.get("spent").is_none(), "{written}");
        assert_eq!(written["unreadable"], "no last line");
        assert_eq!(written["reason"], "the agent was killed");
    }

    #[test]
    fn the_directory_is_made_when_it_is_not_there() {
        let (_dir, runs) = in_a_temporary_directory();
        assert!(!runs.at.exists());
        runs.append(a_run("1")).unwrap();
        assert!(runs.at.exists());
    }
}
