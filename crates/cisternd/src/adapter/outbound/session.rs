//! The sessions file.
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

use crate::core::port::outbound::{SessionStore, StoredSession, StoredSessions, Unavailable};

/// The sessions, kept as JSON at a path fixed when this is built.
pub struct FileSessions {
    path: PathBuf,
    /// Held from the read to the write that follows it, for the reason
    /// `SessionStore::update` gives.
    writing: Mutex<()>,
}

/// The file, as JSON sees it.
///
/// A value is held as whatever JSON found rather than as what the field is
/// supposed to take, for the reason `adapter::backlog` gives.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    next_id: Value,
    sessions: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: Value,
    state: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    stopped_reason: Value,
    usage: Value,
    time: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    model: Value,
    started_at: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    limit_at_start: Value,
    consumed: Value,
    updated_at: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    resets_at: Value,
}

impl FileSessions {
    /// Takes the path it is given. This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileSessions {
            path,
            writing: Mutex::new(()),
        }
    }

    /// The path beside the backlog, or nothing when there is nowhere for it.
    pub fn in_data_home() -> Option<Self> {
        path_of(env::var_os("XDG_DATA_HOME"), env::var_os("HOME")).map(FileSessions::at)
    }

    fn failing(&self, e: impl std::fmt::Display) -> Unavailable {
        Unavailable::new(format!("{}: {e}", self.path.display()))
    }

    fn read(&self) -> Result<StoredSessions, Unavailable> {
        let written = match fs::read_to_string(&self.path) {
            Ok(written) => written,
            // Nothing has run yet. Any other read failure is worth saying.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredSessions {
                    next_id: "1".to_owned(),
                    sessions: Vec::new(),
                });
            }
            Err(e) => return Err(self.failing(e)),
        };

        let document: Document = serde_json::from_str(&written).map_err(|e| self.failing(e))?;
        Ok(StoredSessions {
            next_id: as_text(document.next_id),
            sessions: document
                .sessions
                .into_iter()
                .map(|entry| StoredSession {
                    id: as_text(entry.id),
                    state: as_text(entry.state),
                    stopped_reason: as_optional(entry.stopped_reason),
                    usage: as_text(entry.usage),
                    time: as_text(entry.time),
                    model: as_optional(entry.model),
                    started_at: as_text(entry.started_at),
                    limit_at_start: as_optional(entry.limit_at_start),
                    consumed: as_text(entry.consumed),
                    updated_at: as_text(entry.updated_at),
                    resets_at: as_optional(entry.resets_at),
                })
                .collect(),
        })
    }

    /// Writes beside the file and renames it into place, for the reason
    /// `adapter::settings` gives.
    fn write(&self, sessions: &StoredSessions) -> Result<(), Unavailable> {
        let document = Document {
            next_id: as_number(&sessions.next_id),
            sessions: sessions
                .sessions
                .iter()
                .map(|session| Entry {
                    id: as_number(&session.id),
                    state: Value::String(session.state.clone()),
                    stopped_reason: as_value(session.stopped_reason.clone()),
                    usage: Value::String(session.usage.clone()),
                    time: Value::String(session.time.clone()),
                    model: as_value(session.model.clone()),
                    started_at: as_number(&session.started_at),
                    limit_at_start: session
                        .limit_at_start
                        .as_deref()
                        .map_or(Value::Null, as_number),
                    consumed: as_number(&session.consumed),
                    updated_at: as_number(&session.updated_at),
                    resets_at: session.resets_at.as_deref().map_or(Value::Null, as_number),
                })
                .collect(),
        };
        let written = serde_json::to_string_pretty(&document).map_err(|e| self.failing(e))?;

        let staged = self.path.with_extension("json.new");
        replace(&self.path, &staged, &written).map_err(|e| self.failing(e))
    }
}

impl SessionStore for FileSessions {
    fn update(
        &self,
        change: &mut dyn FnMut(&mut StoredSessions) -> bool,
    ) -> Result<(), Unavailable> {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);

        let mut sessions = self.read()?;
        match change(&mut sessions) {
            true => self.write(&sessions),
            false => Ok(()),
        }
    }
}

/// `$XDG_DATA_HOME/cistern/sessions.json`, or
/// `~/.local/share/cistern/sessions.json`.
///
/// Beside the backlog, for the reason `adapter::backlog` gives.
fn path_of(data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match data_home {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home?).join(".local").join("share"),
    };
    Some(base.join("cistern").join("sessions.json"))
}

fn as_text(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

fn as_optional(value: Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(as_text(other)),
    }
}

fn as_number(text: &str) -> Value {
    match text.parse::<u64>() {
        Ok(number) => Value::from(number),
        Err(_) => Value::String(text.to_owned()),
    }
}

fn as_value(text: Option<String>) -> Value {
    text.map_or(Value::Null, Value::String)
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

    fn in_a_temporary_directory() -> (TempDir, FileSessions) {
        let dir = TempDir::new().unwrap();
        let sessions = FileSessions::at(dir.path().join("sessions.json"));
        (dir, sessions)
    }

    fn a_session() -> StoredSession {
        StoredSession {
            id: "1".to_owned(),
            state: "running".to_owned(),
            stopped_reason: None,
            usage: "50%".to_owned(),
            time: "8h".to_owned(),
            model: None,
            started_at: "1000".to_owned(),
            limit_at_start: Some("1100".to_owned()),
            consumed: "0".to_owned(),
            updated_at: "1000".to_owned(),
            resets_at: None,
        }
    }

    fn some_sessions() -> StoredSessions {
        StoredSessions {
            next_id: "2".to_owned(),
            sessions: vec![a_session()],
        }
    }

    fn put(sessions: &FileSessions, held: &StoredSessions) {
        sessions
            .update(&mut |there| {
                *there = held.clone();
                true
            })
            .unwrap();
    }

    #[test]
    fn the_data_directory_wins() {
        assert_eq!(
            path_of(some("/x/.share"), some("/home/a")),
            Some(PathBuf::from("/x/.share/cistern/sessions.json"))
        );
    }

    #[test]
    fn home_stands_in_where_there_is_no_data_directory() {
        assert_eq!(
            path_of(None, some("/home/a")),
            Some(PathBuf::from("/home/a/.local/share/cistern/sessions.json"))
        );
    }

    #[test]
    fn neither_leaves_nowhere_to_put_it() {
        assert_eq!(path_of(None, None), None);
    }

    #[test]
    fn nothing_stored_reads_as_a_machine_nothing_has_run_on() {
        let (_dir, sessions) = in_a_temporary_directory();
        let read = sessions.read().unwrap();
        assert!(read.sessions.is_empty());
        assert_eq!(read.next_id, "1");
    }

    #[test]
    fn what_was_written_is_there_for_the_next_process_to_read() {
        let (dir, sessions) = in_a_temporary_directory();
        put(&sessions, &some_sessions());

        let restarted = FileSessions::at(dir.path().join("sessions.json"));
        assert_eq!(restarted.read(), Ok(some_sessions()));
    }

    #[test]
    fn a_file_that_is_not_json_fails_rather_than_reading_as_empty() {
        let (dir, sessions) = in_a_temporary_directory();
        fs::write(dir.path().join("sessions.json"), "this is not json").unwrap();
        assert!(sessions.read().is_err());
    }

    #[test]
    fn a_field_the_specification_does_not_have_fails() {
        let (dir, sessions) = in_a_temporary_directory();
        fs::write(
            dir.path().join("sessions.json"),
            r#"{"next_id":1,"sessions":[],"colour":"red"}"#,
        )
        .unwrap();
        assert!(sessions.read().is_err());
    }

    #[test]
    fn a_reason_that_is_absent_and_one_that_is_null_read_the_same() {
        let (dir, sessions) = in_a_temporary_directory();
        fs::write(
            dir.path().join("sessions.json"),
            r#"{"next_id":2,"sessions":[{"id":1,"state":"running","stopped_reason":null,
                "usage":"50%","time":"8h","started_at":1000,"consumed":0,"updated_at":1000}]}"#,
        )
        .unwrap();

        let read = sessions.read().unwrap();
        assert_eq!(read.sessions[0].stopped_reason, None);
        assert_eq!(read.sessions[0].model, None);
    }

    #[test]
    fn identifiers_go_back_into_the_file_as_numbers() {
        let (dir, sessions) = in_a_temporary_directory();
        put(&sessions, &some_sessions());

        let written = fs::read_to_string(dir.path().join("sessions.json")).unwrap();
        assert!(written.contains("\"next_id\": 2"), "{written}");
        assert!(written.contains("\"id\": 1"), "{written}");
    }

    #[test]
    fn the_staged_file_does_not_outlive_the_write() {
        let (dir, sessions) = in_a_temporary_directory();
        put(&sessions, &some_sessions());
        assert!(!dir.path().join("sessions.json.new").exists());
    }

    #[test]
    fn two_writers_at_once_do_not_write_over_each_other() {
        let (_dir, sessions) = in_a_temporary_directory();
        let sessions = &sessions;
        let writers = 8;

        std::thread::scope(|threads| {
            for n in 0..writers {
                threads.spawn(move || {
                    sessions
                        .update(&mut |held| {
                            held.sessions.push(StoredSession {
                                id: n.to_string(),
                                ..a_session()
                            });
                            true
                        })
                        .unwrap();
                });
            }
        });

        assert_eq!(sessions.read().unwrap().sessions.len(), writers);
    }
}
