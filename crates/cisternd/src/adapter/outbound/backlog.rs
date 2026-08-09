//! The backlog file.
//!
//! The only place that knows the path and the file format. Neither reaches the
//! core.

use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::port::outbound::{BacklogStore, StoredBacklog, StoredTask, Unavailable};

/// The backlog, kept as JSON at a path fixed when this is built.
pub struct FileBacklog {
    path: PathBuf,
    /// Held from the read to the write that follows it, so that two tasks
    /// recording their state at the same moment do not write over each other.
    writing: Mutex<()>,
}

/// The file, as JSON sees it.
///
/// A value is held as whatever JSON found rather than as what the field is
/// supposed to take. Which values a field takes is the core's to decide, so a
/// file holding the wrong sort of one reaches it and is refused there.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    next_id: Value,
    tasks: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: Value,
    title: Value,
    instruction: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    branch: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    after: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    model: Value,
    repository: Value,
    state: Value,
}

impl FileBacklog {
    /// Takes the path it is given. This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileBacklog {
            path,
            writing: Mutex::new(()),
        }
    }

    /// The path `docs/cli.md` names, or nothing when there is nowhere for it.
    pub fn in_data_home() -> Option<Self> {
        path_of(env::var_os("XDG_DATA_HOME"), env::var_os("HOME")).map(FileBacklog::at)
    }

    fn failing(&self, e: impl std::fmt::Display) -> Unavailable {
        Unavailable::new(format!("{}: {e}", self.path.display()))
    }
}

/// `$XDG_DATA_HOME/cistern/backlog.json`, or
/// `~/.local/share/cistern/backlog.json`.
///
/// Data rather than state, because the XDG specification keeps state for what
/// is not worth carrying between machines, and a task a user registered is not
/// that.
///
/// The two are arguments rather than reads, so that the choice between them can
/// be tested without setting a variable the whole process sees.
fn path_of(data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match data_home {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home?).join(".local").join("share"),
    };
    Some(base.join("cistern").join("backlog.json"))
}

/// The text a user would have typed for what JSON holds.
///
/// A string keeps its contents; everything else is rendered as the file writes
/// it, so a number, a boolean, and an object all reach the core as something it
/// can read and refuse.
fn as_text(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

/// A field that may be absent. JSON null and a missing key mean the same here.
fn as_optional(value: Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(as_text(other)),
    }
}

/// A number as the file holds one.
///
/// What the file looks like is this module's business, so an identifier goes
/// back as a JSON number rather than as the text the core handed over. Text
/// that is not one is written as it stands, since the core refuses such a value
/// long before it reaches here.
fn as_number(text: &str) -> Value {
    match text.parse::<u64>() {
        Ok(number) => Value::from(number),
        Err(_) => Value::String(text.to_owned()),
    }
}

fn as_value(text: Option<String>) -> Value {
    text.map_or(Value::Null, Value::String)
}

impl FileBacklog {
    fn read(&self) -> Result<StoredBacklog, Unavailable> {
        let written = match fs::read_to_string(&self.path) {
            Ok(written) => written,
            // Nothing registered yet. Any other read failure is worth saying.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredBacklog {
                    next_id: "1".to_owned(),
                    tasks: Vec::new(),
                });
            }
            Err(e) => return Err(self.failing(e)),
        };

        let document: Document = serde_json::from_str(&written).map_err(|e| self.failing(e))?;
        Ok(StoredBacklog {
            next_id: as_text(document.next_id),
            tasks: document
                .tasks
                .into_iter()
                .map(|entry| StoredTask {
                    id: as_text(entry.id),
                    title: as_text(entry.title),
                    instruction: as_text(entry.instruction),
                    branch: as_optional(entry.branch),
                    after: as_optional(entry.after),
                    model: as_optional(entry.model),
                    repository: as_text(entry.repository),
                    state: as_text(entry.state),
                })
                .collect(),
        })
    }

    /// Writes beside the file and renames it into place, for the reason
    /// `adapter::settings` gives.
    fn write(&self, backlog: &StoredBacklog) -> Result<(), Unavailable> {
        let document = Document {
            next_id: as_number(&backlog.next_id),
            tasks: backlog
                .tasks
                .iter()
                .map(|task| Entry {
                    id: as_number(&task.id),
                    title: Value::String(task.title.clone()),
                    instruction: Value::String(task.instruction.clone()),
                    branch: as_value(task.branch.clone()),
                    after: task.after.as_deref().map_or(Value::Null, as_number),
                    model: as_value(task.model.clone()),
                    repository: Value::String(task.repository.clone()),
                    state: Value::String(task.state.clone()),
                })
                .collect(),
        };
        let written = serde_json::to_string_pretty(&document).map_err(|e| self.failing(e))?;

        let staged = self.path.with_extension("json.new");
        replace(&self.path, &staged, &written).map_err(|e| self.failing(e))
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
        // A thread that panicked under this lock left the file alone, since the
        // write is the last thing that happens under it.
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);

        let mut backlog = self.read()?;
        match change(&mut backlog) {
            true => self.write(&backlog),
            false => Ok(()),
        }
    }
}

fn replace(path: &Path, staged: &Path, written: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Err(e) = fs::write(staged, written) {
        let _ = fs::remove_file(staged);
        return Err(e);
    }
    fs::rename(staged, path)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn some(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

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
            branch: None,
            after: None,
            model: None,
            repository: "/work/api".to_owned(),
            state: "Pending".to_owned(),
        }
    }

    fn a_backlog() -> StoredBacklog {
        StoredBacklog {
            next_id: "2".to_owned(),
            tasks: vec![a_task()],
        }
    }

    /// Puts a backlog in place, which is what a change does when it replaces
    /// everything it was handed.
    fn put(tasks: &FileBacklog, backlog: &StoredBacklog) {
        tasks
            .update(&mut |held| {
                *held = backlog.clone();
                true
            })
            .unwrap();
    }

    #[test]
    fn the_data_directory_wins() {
        assert_eq!(
            path_of(some("/x/.share"), some("/home/a")),
            Some(PathBuf::from("/x/.share/cistern/backlog.json"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_data_directory() {
        assert_eq!(
            path_of(None, some("/home/a")),
            Some(PathBuf::from("/home/a/.local/share/cistern/backlog.json"))
        );
    }

    #[test]
    fn neither_leaves_nowhere_to_put_it() {
        assert_eq!(path_of(None, None), None);
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

    /// Which values a field takes is the core's to decide, so a file holding
    /// the wrong sort of one has to arrive rather than fail on the way.
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

    /// A number crosses the port as text and goes back as a number, so the file
    /// stays JSON a person would have written.
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

    #[test]
    fn the_staged_file_does_not_outlive_the_write() {
        let (dir, tasks) = in_a_temporary_directory();
        put(&tasks, &a_backlog());
        assert!(!dir.path().join("backlog.json.new").exists());
    }

    /// A refused command reads the backlog and changes nothing, and the file it
    /// read should be the file that is still there.
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

    /// Two tasks ending at the same moment both record their state. Reading and
    /// writing as two calls would let the later write drop the earlier one, so
    /// this counts what survived.
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
