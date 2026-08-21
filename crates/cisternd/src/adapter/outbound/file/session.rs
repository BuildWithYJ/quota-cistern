//! The sessions file.
//!
//! The only place that knows the path and the file format.
//! Neither reaches the core.

use std::{env, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::port::outbound::{SessionStore, StoredSession, StoredSessions, Unavailable};

use super::{
    kept::Kept,
    {as_number, as_optional, as_text, as_value},
};

/// The sessions, kept as JSON at a path fixed when this is built.
pub struct FileSessions {
    kept: Kept,
}

/// The whole file, as JSON sees it.
///
/// A value is held as whatever JSON found rather than as what the field is supposed to take.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
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
    #[serde(default, skip_serializing_if = "Value::is_null")]
    limit_last_seen: Value,
    consumed: Value,
    updated_at: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    resets_at: Value,
}

/// What the file is called.
const NAMED: &str = "sessions.json";

impl FileSessions {
    /// Takes the path it is given.
    /// This is how a test reaches a temporary one.
    pub fn at(path: PathBuf) -> Self {
        FileSessions {
            kept: Kept::at(path),
        }
    }

    /// The path beside the backlog, or nothing when there is nowhere for it.
    pub fn in_data_home() -> Option<Self> {
        Kept::in_data_home(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"), NAMED)
            .map(FileSessions::at)
    }

    fn read(&self) -> Result<StoredSessions, Unavailable> {
        // Nothing has run yet is an empty set rather than a failure.
        let Some(written) = self.kept.read()? else {
            return Ok(StoredSessions {
                next_id: "1".to_owned(),
                sessions: Vec::new(),
            });
        };

        let document: Written = serde_json::from_str(&written).map_err(|e| self.kept.failing(e))?;
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
                    limit_last_seen: as_optional(entry.limit_last_seen),
                    consumed: as_text(entry.consumed),
                    updated_at: as_text(entry.updated_at),
                    resets_at: as_optional(entry.resets_at),
                })
                .collect(),
        })
    }

    fn write(&self, sessions: &StoredSessions) -> Result<(), Unavailable> {
        let document = Written {
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
                    limit_last_seen: session
                        .limit_last_seen
                        .as_deref()
                        .map_or(Value::Null, as_number),
                    consumed: as_number(&session.consumed),
                    updated_at: as_number(&session.updated_at),
                    resets_at: session.resets_at.as_deref().map_or(Value::Null, as_number),
                })
                .collect(),
        };
        let written = serde_json::to_string_pretty(&document).map_err(|e| self.kept.failing(e))?;
        self.kept.write(&written)
    }
}

impl SessionStore for FileSessions {
    fn update(
        &self,
        change: &mut dyn FnMut(&mut StoredSessions) -> bool,
    ) -> Result<(), Unavailable> {
        let _writing = self.kept.holding();

        let mut sessions = self.read()?;
        match change(&mut sessions) {
            true => self.write(&sessions),
            false => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

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
            limit_last_seen: Some("1100".to_owned()),
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

    /// Where the file goes is `kept`'s; which file it is belongs here.
    #[test]
    fn the_file_is_the_one_the_specification_names() {
        let path =
            Kept::in_data_home(Some(std::ffi::OsString::from("/x/.share")), None, NAMED).unwrap();
        assert_eq!(path, PathBuf::from("/x/.share/cistern/sessions.json"));
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
