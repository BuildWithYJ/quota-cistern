//! The ledger of runs.
//!
//! The only place that knows the path and the file format.
//! One line for each run, appended and never rewritten, so the file grows and nothing in it moves.
//! Read and written whole is what the other stores do; this one is not, because a run that ended
//! must not be able to displace a run that ended before it.

use std::{
    env, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::port::outbound::{Run, Runs, Unavailable};

use super::{Counted, as_number, as_optional, as_text, as_value, counted, kept::Kept, spending};

/// What the file is called, beside the two the other stores keep.
const NAMED: &str = "runs.jsonl";

/// The ledger, kept as one JSON object to a line at a path fixed when this is built.
pub struct FileRuns {
    at: PathBuf,
    /// Held across a whole append, so that two runs ending at once cannot interleave.
    ///
    /// Appending is atomic in where it writes and not in how many writes it takes, so a record
    /// and the newline that ends it can be split by another writer's record. A line that comes
    /// out of that is not JSON, and reading drops it: two runs end and the ledger keeps
    /// neither. One core holds the socket, so one lock in this process is the whole of it.
    writing: Mutex<()>,
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
    /// What the vendor said about how it ended, where it said anything.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    said: Value,
    /// How many turns it took, where the vendor counted them.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    turns: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spent: Option<Counted>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    unreadable: Value,
    /// What the session allowed this run, in the unit the budget was declared in.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    ceiling: Value,
    /// How far the vendor's limit was spent when the run started and when it stopped.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    limit_before: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    limit_after: Value,
    /// When the reading that ends the run was taken, which is not when the run ended.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    limit_after_at: Value,
}

impl FileRuns {
    /// Takes the path it is given.
    /// This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileRuns {
            at: path,
            writing: Mutex::new(()),
        }
    }

    /// Beside the backlog, under the directory `docs/cli.md` names.
    pub fn in_data_home() -> Option<Self> {
        Kept::in_data_home(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"), NAMED)
            .map(FileRuns::at)
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

        // The record and the newline that ends it, as one buffer and one call, under a hold
        // kept until that call returns. Appending puts a write at the end of the file whatever
        // else is writing, but says nothing about a record that takes more than one write: the
        // next writer's record can land between this one and its newline, and the line that
        // comes out is not JSON. Reading drops it, so two runs end and neither is kept.
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut held = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.at)
            .map_err(|e| self.failing(&self.at, e))?;
        held.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| self.failing(&self.at, e))
    }

    fn read(&self) -> Result<Vec<Run>, Unavailable> {
        let written = match fs::read_to_string(&self.at) {
            Ok(written) => written,
            // Nothing has run yet, which is not a failure.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(self.failing(&self.at, e)),
        };
        Ok(written
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
            .map(held)
            .collect())
    }
}

/// What the file holds, as the core takes it.
fn held(entry: Entry) -> Run {
    Run {
        task: as_text(entry.task),
        session: as_optional(entry.session),
        model: as_optional(entry.model),
        started_at: as_text(entry.started_at),
        ended_at: as_text(entry.ended_at),
        outcome: as_text(entry.outcome),
        reason: as_optional(entry.reason),
        said: as_optional(entry.said),
        turns: as_optional(entry.turns),
        spent: entry.spent.map(spending),
        unreadable: as_optional(entry.unreadable),
        ceiling: as_optional(entry.ceiling),
        limit_before: as_optional(entry.limit_before),
        limit_after: as_optional(entry.limit_after),
        limit_after_at: as_optional(entry.limit_after_at),
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
        said: as_value(run.said),
        turns: run.turns.as_deref().map_or(Value::Null, as_number),
        spent: run.spent.as_ref().map(counted),
        unreadable: as_value(run.unreadable),
        ceiling: run.ceiling.as_deref().map_or(Value::Null, as_number),
        limit_before: run.limit_before.as_deref().map_or(Value::Null, as_number),
        limit_after: run.limit_after.as_deref().map_or(Value::Null, as_number),
        limit_after_at: run.limit_after_at.as_deref().map_or(Value::Null, as_number),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::core::port::outbound::StoredConsumption;

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
            said: None,
            turns: None,
            spent: Some(StoredConsumption {
                input: "10".to_owned(),
                output: "20".to_owned(),
                cache_written: "30".to_owned(),
                cache_read: "40".to_owned(),
                cost: "50".to_owned(),
            }),
            unreadable: None,
            ceiling: Some("900".to_owned()),
            limit_before: Some("1100".to_owned()),
            limit_after: Some("1400".to_owned()),
            limit_after_at: Some("1786350140".to_owned()),
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
                "ceiling": 900,
                "outcome": "completed",
                "spent": {
                    "input": 10, "output": 20,
                    "cache_written": 30, "cache_read": 40, "cost": 50,
                },
                "limit_before": 1_100u64,
                "limit_after": 1_400u64,
                "limit_after_at": 1_786_350_140u64,
            })]
        );
    }

    /// A session declared in tokens never asks the vendor how far its limit is spent, so a run
    /// of one has no reading either side of it. The line leaves the two out rather than
    /// holding a figure that would read as zero.
    #[test]
    fn a_run_with_no_reading_either_side_leaves_them_out() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(Run {
            limit_before: None,
            limit_after: None,
            limit_after_at: None,
            ..a_run("1")
        })
        .unwrap();

        let held = lines(&runs)[0].clone();
        assert!(held.get("limit_before").is_none(), "{held}");
        assert!(held.get("limit_after").is_none(), "{held}");
        assert!(held.get("limit_after_at").is_none(), "{held}");
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
            said: None,
            turns: None,
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

    /// What went in comes back, in the order it went in.
    #[test]
    fn every_run_comes_back_oldest_first() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(a_run("1")).unwrap();
        runs.append(a_run("2")).unwrap();

        let held = runs.read().unwrap();
        assert_eq!(
            held.iter().map(|run| run.task.as_str()).collect::<Vec<_>>(),
            ["1", "2"]
        );
        assert_eq!(held[0], a_run("1"));
    }

    /// Nothing has run yet, which is not a failure.
    #[test]
    fn a_ledger_nobody_has_written_reads_as_nothing() {
        let (_dir, runs) = in_a_temporary_directory();
        assert_eq!(runs.read().unwrap(), Vec::new());
    }

    /// The file is appended to by a process that can be killed part way through a line.
    /// One line nobody can read is not a reason to lose the rest.
    #[test]
    fn a_line_that_cannot_be_read_is_left_out_rather_than_losing_the_others() {
        let (_dir, runs) = in_a_temporary_directory();
        runs.append(a_run("1")).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&runs.at)
            .and_then(|mut held| writeln!(held, "{{\"task\": \"2\", \"ende"))
            .unwrap();
        runs.append(a_run("3")).unwrap();

        let held = runs.read().unwrap();
        assert_eq!(
            held.iter().map(|run| run.task.as_str()).collect::<Vec<_>>(),
            ["1", "3"]
        );
    }
}
