//! The backlog file.
//!
//! The only place that knows the path and the file format.
//! Neither reaches the core.

use std::{env, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::port::outbound::{
    BacklogStore, StoredBacklog, StoredConsumption, StoredTask, Unavailable,
};

use super::{
    kept::Kept,
    {as_number, as_optional, as_text, as_value},
};

/// The backlog, kept as JSON at a path fixed when this is built.
pub struct FileBacklog {
    kept: Kept,
}

/// The whole file, as JSON sees it.
///
/// A value is held as whatever JSON found rather than as what the field is supposed to take.
/// Which values a field takes is the core's to decide.
/// A file holding the wrong sort of one reaches it and is refused there.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    next_id: Value,
    tasks: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: Value,
    title: Value,
    instruction: Value,
    /// Absent for a backlog written before a filled-in instruction kept what the author wrote.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    original: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    branch: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    after: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    model: Value,
    repository: Value,
    state: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    session: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    worktree: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    started_at: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    ended_at: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    reason: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consumed: Option<Counted>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    unreadable: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    disposition: Value,
}

/// What a task consumed, as the file holds it.
///
/// An object of its own rather than five fields beside the others.
/// That way a task that never ran carries no counts at all.
/// A reader can see which of the three states a task is in without comparing five keys.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Counted {
    input: Value,
    output: Value,
    cache_written: Value,
    cache_read: Value,
    cost: Value,
}

/// What the file is called where `docs/cli.md` says it is kept.
const NAMED: &str = "backlog.json";

impl FileBacklog {
    /// Takes the path it is given.
    /// This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileBacklog {
            kept: Kept::at(path),
        }
    }

    /// The path `docs/cli.md` names, or nothing when there is nowhere for it.
    pub fn in_data_home() -> Option<Self> {
        Kept::in_data_home(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"), NAMED)
            .map(FileBacklog::at)
    }
}

impl FileBacklog {
    fn read(&self) -> Result<StoredBacklog, Unavailable> {
        // Nothing registered yet is an empty backlog rather than a failure.
        let Some(written) = self.kept.read()? else {
            return Ok(StoredBacklog {
                next_id: "1".to_owned(),
                tasks: Vec::new(),
            });
        };

        let document: Written = serde_json::from_str(&written).map_err(|e| self.kept.failing(e))?;
        Ok(StoredBacklog {
            next_id: as_text(document.next_id),
            tasks: document
                .tasks
                .into_iter()
                .map(|entry| StoredTask {
                    id: as_text(entry.id),
                    title: as_text(entry.title),
                    instruction: as_text(entry.instruction),
                    original: as_optional(entry.original),
                    branch: as_optional(entry.branch),
                    after: as_optional(entry.after),
                    model: as_optional(entry.model),
                    repository: as_text(entry.repository),
                    state: as_text(entry.state),
                    session: as_optional(entry.session),
                    worktree: as_optional(entry.worktree),
                    started_at: as_optional(entry.started_at),
                    ended_at: as_optional(entry.ended_at),
                    reason: as_optional(entry.reason),
                    consumed: entry.consumed.map(|counted| StoredConsumption {
                        input: as_text(counted.input),
                        output: as_text(counted.output),
                        cache_written: as_text(counted.cache_written),
                        cache_read: as_text(counted.cache_read),
                        cost: as_text(counted.cost),
                    }),
                    unreadable: as_optional(entry.unreadable),
                    disposition: as_optional(entry.disposition),
                })
                .collect(),
        })
    }

    fn write(&self, backlog: &StoredBacklog) -> Result<(), Unavailable> {
        let document = Written {
            next_id: as_number(&backlog.next_id),
            tasks: backlog
                .tasks
                .iter()
                .map(|task| Entry {
                    id: as_number(&task.id),
                    title: Value::String(task.title.clone()),
                    instruction: Value::String(task.instruction.clone()),
                    original: as_value(task.original.clone()),
                    branch: as_value(task.branch.clone()),
                    after: task.after.as_deref().map_or(Value::Null, as_number),
                    model: as_value(task.model.clone()),
                    repository: Value::String(task.repository.clone()),
                    state: Value::String(task.state.clone()),
                    session: task.session.as_deref().map_or(Value::Null, as_number),
                    worktree: as_value(task.worktree.clone()),
                    started_at: task.started_at.as_deref().map_or(Value::Null, as_number),
                    ended_at: task.ended_at.as_deref().map_or(Value::Null, as_number),
                    reason: as_value(task.reason.clone()),
                    consumed: task.consumed.as_ref().map(|counted| Counted {
                        input: as_number(&counted.input),
                        output: as_number(&counted.output),
                        cache_written: as_number(&counted.cache_written),
                        cache_read: as_number(&counted.cache_read),
                        cost: as_number(&counted.cost),
                    }),
                    unreadable: as_value(task.unreadable.clone()),
                    disposition: as_value(task.disposition.clone()),
                })
                .collect(),
        };
        let written = serde_json::to_string_pretty(&document).map_err(|e| self.kept.failing(e))?;
        self.kept.write(&written)
    }
}

impl BacklogStore for FileBacklog {
    fn load(&self) -> Result<StoredBacklog, Unavailable> {
        self.read()
    }

    fn update(
        &self,
        change: &mut dyn FnMut(&mut StoredBacklog) -> bool,
    ) -> Result<(), Unavailable> {
        let _writing = self.kept.holding();

        let mut backlog = self.read()?;
        match change(&mut backlog) {
            true => self.write(&backlog),
            false => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn in_a_temporary_directory() -> (TempDir, FileBacklog) {
        let dir = TempDir::new().unwrap();
        let tasks = FileBacklog::at(dir.path().join("backlog.json"));
        (dir, tasks)
    }

    fn a_task() -> StoredTask {
        StoredTask {
            id: "1".to_owned(),
            title: "refactor X".to_owned(),
            instruction: "tidy up src/utils".to_owned(),
            original: None,
            branch: None,
            after: None,
            model: None,
            repository: "/work/api".to_owned(),
            state: "Pending".to_owned(),
            session: None,
            worktree: None,
            started_at: None,
            ended_at: None,
            reason: None,
            consumed: None,
            unreadable: None,
            disposition: None,
        }
    }

    fn a_backlog() -> StoredBacklog {
        StoredBacklog {
            next_id: "2".to_owned(),
            tasks: vec![a_task()],
        }
    }

    /// Puts a backlog in place, which is what a change does when it replaces everything it was handed.
    fn put(tasks: &FileBacklog, backlog: &StoredBacklog) {
        tasks
            .update(&mut |held| {
                *held = backlog.clone();
                true
            })
            .unwrap();
    }

    /// Where the file goes is `kept`'s; which file it is belongs here.
    #[test]
    fn the_backlog_is_the_file_section_two_one_names() {
        let path =
            Kept::in_data_home(Some(std::ffi::OsString::from("/x/.share")), None, NAMED).unwrap();
        assert_eq!(path, PathBuf::from("/x/.share/cistern/backlog.json"));
    }

    #[test]
    fn nothing_stored_reads_as_a_backlog_nobody_has_added_to() {
        let (_dir, tasks) = in_a_temporary_directory();
        let read = tasks.load().unwrap();
        assert!(read.tasks.is_empty());
        assert_eq!(read.next_id, "1");
    }

    #[test]
    fn what_was_written_is_there_for_the_next_process_to_read() {
        let (dir, tasks) = in_a_temporary_directory();
        put(&tasks, &a_backlog());

        // A second reader over the same path is what a restarted core is.
        let restarted = FileBacklog::at(dir.path().join("backlog.json"));
        assert_eq!(restarted.load(), Ok(a_backlog()));
    }

    /// The two times are what a later reading of how long tasks take is worked out from,
    /// so a restart that lost them would leave nothing to work it out from.
    #[test]
    fn when_a_run_started_and_stopped_survives_a_restart() {
        let (dir, tasks) = in_a_temporary_directory();
        let mut ran = a_task();
        ran.state = "Completed".to_owned();
        ran.session = Some("1".to_owned());
        ran.started_at = Some("1786316972".to_owned());
        ran.ended_at = Some("1786317418".to_owned());
        put(
            &tasks,
            &StoredBacklog {
                next_id: "2".to_owned(),
                tasks: vec![ran.clone()],
            },
        );

        let restarted = FileBacklog::at(dir.path().join("backlog.json"));
        let read = restarted.load().unwrap();
        assert_eq!(read.tasks[0].started_at.as_deref(), Some("1786316972"));
        assert_eq!(read.tasks[0].ended_at.as_deref(), Some("1786317418"));
    }

    /// A task that has not run carries neither, and the file says so by leaving them out.
    #[test]
    fn a_task_that_never_ran_writes_no_times() {
        let (dir, tasks) = in_a_temporary_directory();
        put(&tasks, &a_backlog());

        let written = fs::read_to_string(dir.path().join("backlog.json")).unwrap();
        assert!(!written.contains("started_at"), "{written}");
        assert!(!written.contains("ended_at"), "{written}");
    }

    #[test]
    fn a_file_that_is_not_json_fails_rather_than_reading_as_empty() {
        let (dir, tasks) = in_a_temporary_directory();
        fs::write(dir.path().join("backlog.json"), "this is not json").unwrap();
        assert!(tasks.load().is_err());
    }

    #[test]
    fn a_field_the_specification_does_not_have_fails() {
        let (dir, tasks) = in_a_temporary_directory();
        fs::write(
            dir.path().join("backlog.json"),
            r#"{"next_id":1,"tasks":[],"colour":"red"}"#,
        )
        .unwrap();
        assert!(tasks.load().is_err());
    }

    /// Which values a field takes is the core's to decide.
    /// A file holding the wrong sort of one has to arrive rather than fail on the way.
    #[test]
    fn a_value_of_another_type_reads_as_the_text_it_was_written_as() {
        let (dir, tasks) = in_a_temporary_directory();
        fs::write(
            dir.path().join("backlog.json"),
            r#"{"next_id":"soon","tasks":[{"id":true,"title":"x","instruction":"y",
                "repository":"/work/api","state":7}]}"#,
        )
        .unwrap();

        let read = tasks.load().unwrap();
        assert_eq!(read.next_id, "soon");
        assert_eq!(read.tasks[0].id, "true");
        assert_eq!(read.tasks[0].state, "7");
    }

    #[test]
    fn a_field_that_is_absent_and_one_that_is_null_read_the_same() {
        let (dir, tasks) = in_a_temporary_directory();
        fs::write(
            dir.path().join("backlog.json"),
            r#"{"next_id":2,"tasks":[{"id":1,"title":"x","instruction":"y","after":null,
                "repository":"/work/api","state":"Pending"}]}"#,
        )
        .unwrap();

        let read = tasks.load().unwrap();
        assert_eq!(read.tasks[0].after, None);
        assert_eq!(read.tasks[0].branch, None);
    }

    /// A backlog written before a task kept what its author wrote reads back, rather than failing.
    #[test]
    fn a_backlog_written_without_an_original_still_reads() {
        let (dir, tasks) = in_a_temporary_directory();
        fs::write(
            dir.path().join("backlog.json"),
            r#"{"next_id":2,"tasks":[{"id":1,"title":"x","instruction":"y",
                "repository":"/work/api","state":"Pending"}]}"#,
        )
        .unwrap();

        let read = tasks.load().unwrap();
        assert_eq!(read.tasks[0].instruction, "y");
        assert_eq!(read.tasks[0].original, None);
    }

    /// A number crosses the port as text and goes back as a number, so the file stays JSON a person would have written.
    #[test]
    fn identifiers_go_back_into_the_file_as_numbers() {
        let (dir, tasks) = in_a_temporary_directory();
        let path = dir.path().join("backlog.json");
        put(
            &tasks,
            &StoredBacklog {
                next_id: "3".to_owned(),
                tasks: vec![StoredTask {
                    after: Some("1".to_owned()),
                    id: "2".to_owned(),
                    ..a_task()
                }],
            },
        );

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"next_id\": 3"), "{written}");
        assert!(written.contains("\"id\": 2"), "{written}");
        assert!(written.contains("\"after\": 1"), "{written}");
    }

    /// The figures are numbers, and a task that never ran carries neither the object nor the reason.
    ///
    /// That is what tells the three states apart in a file a person reads.
    #[test]
    fn what_a_task_consumed_goes_back_into_the_file_as_numbers() {
        let (dir, tasks) = in_a_temporary_directory();
        let path = dir.path().join("backlog.json");
        put(
            &tasks,
            &StoredBacklog {
                next_id: "2".to_owned(),
                tasks: vec![StoredTask {
                    consumed: Some(StoredConsumption {
                        input: "77".to_owned(),
                        output: "3377".to_owned(),
                        cache_written: "28879".to_owned(),
                        cache_read: "263483".to_owned(),
                        cost: "92170".to_owned(),
                    }),
                    ..a_task()
                }],
            },
        );

        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"input\": 77"), "{written}");
        assert!(written.contains("\"cost\": 92170"), "{written}");

        // A second reader over the same path is what a restarted core is.
        let restarted = FileBacklog::at(path);
        let held = restarted.load().unwrap().tasks[0].clone();
        assert_eq!(held.consumed.unwrap().cache_read, "263483");
        assert_eq!(held.unreadable, None);
    }

    #[test]
    fn a_task_nobody_could_count_keeps_the_reason_and_no_figures() {
        let (dir, tasks) = in_a_temporary_directory();
        let path = dir.path().join("backlog.json");
        put(
            &tasks,
            &StoredBacklog {
                next_id: "2".to_owned(),
                tasks: vec![StoredTask {
                    unreadable: Some("the answer said nothing about it".to_owned()),
                    ..a_task()
                }],
            },
        );

        let held = FileBacklog::at(path).load().unwrap().tasks[0].clone();
        assert_eq!(held.consumed, None);
        assert_eq!(
            held.unreadable.as_deref(),
            Some("the answer said nothing about it")
        );
    }

    /// A task registered before this core counted anything has neither key.
    /// A file written by an older one still reads.
    #[test]
    fn a_task_with_no_count_at_all_writes_neither_key() {
        let (dir, tasks) = in_a_temporary_directory();
        put(&tasks, &a_backlog());

        let written = fs::read_to_string(dir.path().join("backlog.json")).unwrap();
        assert!(!written.contains("consumed"), "{written}");
        assert!(!written.contains("unreadable"), "{written}");
    }

    #[test]
    fn the_staged_file_does_not_outlive_the_write() {
        let (dir, tasks) = in_a_temporary_directory();
        put(&tasks, &a_backlog());
        assert!(!dir.path().join("backlog.json.new").exists());
    }

    /// A refused command reads the backlog and changes nothing.
    /// The file it read should be the file that is still there.
    #[test]
    fn a_change_that_changed_nothing_leaves_the_file_where_it_was() {
        let (dir, tasks) = in_a_temporary_directory();
        let path = dir.path().join("backlog.json");
        put(&tasks, &a_backlog());
        let before = fs::metadata(&path).unwrap().modified().unwrap();

        tasks
            .update(&mut |held| {
                held.tasks.clear();
                false
            })
            .unwrap();

        assert_eq!(tasks.load(), Ok(a_backlog()));
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
    }

    /// Two tasks ending at the same moment both record their state.
    ///
    /// Reading and writing as two calls would let the later write drop the earlier one, so this counts what survived.
    #[test]
    fn two_writers_at_once_do_not_write_over_each_other() {
        let (_dir, tasks) = in_a_temporary_directory();
        let tasks = &tasks;
        let writers = 8;

        std::thread::scope(|threads| {
            for n in 0..writers {
                threads.spawn(move || {
                    tasks
                        .update(&mut |held| {
                            held.tasks.push(StoredTask {
                                id: n.to_string(),
                                ..a_task()
                            });
                            true
                        })
                        .unwrap();
                });
            }
        });

        assert_eq!(tasks.load().unwrap().tasks.len(), writers);
    }
}
