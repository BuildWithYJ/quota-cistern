//! What a run wrote, kept as a file for each task.
//!
//! The only place that knows what the vendor's output looks like line by line.
//! The core is handed a time and a sentence.

use std::{
    env, fs,
    io::{self, Read as _, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::core::port::outbound::{Event, Read, Traces, Unavailable};

/// How much of a trace one read may answer with.
///
/// A run that worked for an hour wrote more than anyone wants at once, and
/// section 2.3 says the reader is not made to hold all of it. The cursor is
/// how the rest is asked for.
const AT_MOST: usize = 64 * 1024;

/// Traces kept as one file per task.
pub struct FileTraces {
    at: PathBuf,
}

impl FileTraces {
    /// Takes the directory it is given. This is how a test reaches a
    /// temporary one.
    pub fn at(at: PathBuf) -> Self {
        FileTraces { at }
    }

    /// Beside the sessions, under the directory `docs/cli.md` names.
    pub fn in_data_home() -> Option<Self> {
        let base = match env::var_os("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env::var_os("HOME")?)
                .join(".local")
                .join("share"),
        };
        Some(FileTraces::at(base.join("cistern").join("traces")))
    }

    /// Where the line running past `from` ends, or the end of what is there.
    fn past_the_line(&self, held: &mut fs::File, from: u64) -> Result<u64, Unavailable> {
        let mut at = from;
        let mut written = vec![0u8; AT_MOST];
        loop {
            let read = held
                .read(&mut written)
                .map_err(|e| self.failing(&self.at, e))?;
            if read == 0 {
                return Ok(at);
            }
            if let Some(ends) = written[..read].iter().position(|&byte| byte == b'\n') {
                return Ok(at + ends as u64 + 1);
            }
            at += read as u64;
        }
    }

    fn path_of(&self, task: &str) -> PathBuf {
        self.at.join(format!("{task}.jsonl"))
    }

    fn failing(&self, at: &Path, e: impl std::fmt::Display) -> Unavailable {
        Unavailable::new(format!("{}: {e}", at.display()))
    }
}

impl Traces for FileTraces {
    fn at(&self, task: &str) -> Result<String, Unavailable> {
        fs::create_dir_all(&self.at).map_err(|e| self.failing(&self.at, e))?;
        Ok(self.path_of(task).display().to_string())
    }

    fn read(&self, task: &str, from: &str) -> Result<Read, Unavailable> {
        let at = self.path_of(task);
        let mut held = match fs::File::open(&at) {
            Ok(held) => held,
            // Nothing written yet, which is every task before its run starts.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(Read {
                    events: Vec::new(),
                    cursor: cursor_at(0),
                });
            }
            Err(e) => return Err(self.failing(&at, e)),
        };

        let from = from.parse().unwrap_or(0);
        held.seek(SeekFrom::Start(from))
            .map_err(|e| self.failing(&at, e))?;

        let mut written = vec![0u8; AT_MOST];
        let read = held.read(&mut written).map_err(|e| self.failing(&at, e))?;
        written.truncate(read);

        // A read may stop in the middle of a line, and half a line is not an
        // event. What is left starts the next read.
        let Some(ends) = written.iter().rposition(|&byte| byte == b'\n') else {
            // A line longer than one read holds. Nothing here is an event and
            // starting the next read where this one did would ask the same
            // question forever, so it is stepped over. The line stays in the
            // file for a reading that can take it whole.
            let over = self.past_the_line(&mut held, from + read as u64)?;
            return Ok(Read {
                events: Vec::new(),
                cursor: cursor_at(over),
            });
        };
        let whole = ends + 1;

        let events = String::from_utf8_lossy(&written[..whole])
            .lines()
            .filter_map(event_in)
            .collect();

        Ok(Read {
            events,
            cursor: cursor_at(from + whole as u64),
        })
    }
}

/// A place in the file, written so that one cursor sorts beside another.
fn cursor_at(at: u64) -> String {
    format!("{at:012}")
}

/// One line of what was kept, as an event.
///
/// A line is the time it arrived, a tab, and what the vendor wrote. What the
/// vendor wrote is left as it was, so a reading written later can make more
/// of it than this one does.
fn event_in(line: &str) -> Option<Event> {
    let (at, said) = line.split_once('\t')?;
    Some(Event {
        at: at.to_owned(),
        said: sentence_in(said)?,
    })
}

/// What one line of the vendor's output amounts to.
///
/// What the agent said and what it reached for, one sentence each. Its
/// thinking is left out: it is long, and it is not addressed to anyone.
/// Everything else the vendor writes is book-keeping.
fn sentence_in(written: &str) -> Option<String> {
    let held: Value = serde_json::from_str(written).ok()?;
    match held.get("type").and_then(Value::as_str)? {
        "assistant" => said_in(&held),
        "user" => failed_in(&held),
        // The last line repeats the last thing the agent said, which is
        // already an event of its own.
        _ => None,
    }
}

/// What came back from something the run reached for, when it did not work.
///
/// What did work is what a file holds or a command printed, which is not
/// something a person reads a trace for. What did not is where a run turned,
/// and it is the first thing looked for.
fn failed_in(held: &Value) -> Option<String> {
    let blocks = held.get("message")?.get("content")?.as_array()?;
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        if block.get("is_error").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let said = match block.get("content") {
            Some(Value::String(said)) => said.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        return Some(format!("failed: {}", shortened(&said)));
    }
    None
}

/// What an assistant line amounts to.
fn said_in(held: &Value) -> Option<String> {
    let blocks = held.get("message")?.get("content")?.as_array()?;
    let mut said = Vec::new();

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    said.push(shortened(text));
                }
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str).unwrap_or("?");
                said.push(format!("{name} {}", shortened(&asked_for(block))));
            }
            _ => (),
        }
    }

    let said: Vec<String> = said.into_iter().filter(|one| !one.is_empty()).collect();
    (!said.is_empty()).then(|| said.join(" | "))
}

/// The one thing a tool call is most about.
///
/// A tool takes an object of arguments and one of them is what a person would
/// say it acted on. The rest is left in the file.
fn asked_for(block: &Value) -> String {
    let Some(given) = block.get("input").and_then(Value::as_object) else {
        return String::new();
    };
    for name in ["command", "pattern", "url", "description"] {
        if let Some(said) = given.get(name).and_then(Value::as_str) {
            return said.to_owned();
        }
    }
    for name in ["file_path", "path", "notebook_path"] {
        if let Some(said) = given.get(name).and_then(Value::as_str) {
            return within_the_work_area(said);
        }
    }
    String::new()
}

/// A path as it reads beside the work area it is in.
///
/// A run works in a directory of the core's making, several levels below a
/// temporary root, and every path it touches carries all of that. What tells
/// one file from another is the end of it.
fn within_the_work_area(path: &str) -> String {
    match path.split_once("/worktrees/") {
        Some((_, within)) => match within.split_once('/') {
            Some((_task, rest)) => rest.to_owned(),
            None => within.to_owned(),
        },
        None => path.to_owned(),
    }
}

/// How much of a line is worth a line.
const WIDE: usize = 100;

fn shortened(said: &str) -> String {
    let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
    match said.char_indices().nth(WIDE) {
        Some((at, _)) => format!("{}...", &said[..at]),
        None => said,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn in_a_temporary_directory() -> (TempDir, FileTraces) {
        let dir = TempDir::new().unwrap();
        let traces = FileTraces::at(dir.path().join("traces"));
        (dir, traces)
    }

    /// One line as a run writes it: when it arrived, a tab, and the object.
    fn line(at: u64, written: Value) -> String {
        format!("{at}\t{written}\n")
    }

    fn said(what: &str) -> Value {
        serde_json::json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": what }] }
        })
    }

    fn reached_for(name: &str, given: Value) -> Value {
        serde_json::json!({
            "type": "assistant",
            "message": { "content": [{ "type": "tool_use", "name": name, "input": given }] }
        })
    }

    fn kept(traces: &FileTraces, task: &str, written: &str) {
        let at = traces.at(task).unwrap();
        fs::write(at, written).unwrap();
    }

    #[test]
    fn a_task_that_has_written_nothing_reads_as_nothing() {
        let (_dir, traces) = in_a_temporary_directory();
        let read = traces.read("1", "").unwrap();
        assert_eq!(read.events, Vec::new());
    }

    #[test]
    fn what_the_agent_said_and_what_it_reached_for_are_the_events() {
        let (_dir, traces) = in_a_temporary_directory();
        kept(
            &traces,
            "1",
            &[
                line(1_700_000_001, said("I'll implement it.")),
                line(
                    1_700_000_002,
                    reached_for("Bash", serde_json::json!({ "command": "pytest" })),
                ),
            ]
            .concat(),
        );

        let read = traces.read("1", "").unwrap();
        assert_eq!(read.events.len(), 2);
        assert_eq!(read.events[0].at, "1700000001");
        assert_eq!(read.events[0].said, "I'll implement it.");
        assert_eq!(read.events[1].said, "Bash pytest");
    }

    /// Thinking is long and is not addressed to anyone.
    #[test]
    fn what_the_agent_thought_is_kept_and_not_reported() {
        let (_dir, traces) = in_a_temporary_directory();
        let thought = serde_json::json!({
            "type": "assistant",
            "message": { "content": [{ "type": "thinking", "thinking": "hmm" }] }
        });
        kept(&traces, "1", &line(1_700_000_001, thought));

        assert_eq!(traces.read("1", "").unwrap().events, Vec::new());
        // The line is still in the file, whatever this reading made of it.
        let at = traces.at("1").unwrap();
        assert!(fs::read_to_string(at).unwrap().contains("hmm"));
    }

    /// Book-keeping the vendor writes between the things it did.
    #[test]
    fn a_line_about_nothing_a_person_asked_for_is_not_an_event() {
        let (_dir, traces) = in_a_temporary_directory();
        let counting = serde_json::json!({ "type": "system", "subtype": "thinking_tokens" });
        kept(&traces, "1", &line(1_700_000_001, counting));

        assert_eq!(traces.read("1", "").unwrap().events, Vec::new());
    }

    /// A run that failed at something says so where it happened, and that is
    /// where a reader looks first.
    #[test]
    fn what_a_run_failed_at_is_an_event() {
        let (_dir, traces) = in_a_temporary_directory();
        let failed = serde_json::json!({
            "type": "user",
            "message": { "content": [{
                "type": "tool_result",
                "is_error": true,
                "content": "Exit code 1\n===== test session starts ====="
            }] }
        });
        kept(&traces, "1", &line(1_700_000_001, failed));

        let events = traces.read("1", "").unwrap().events;
        assert_eq!(events.len(), 1, "{events:?}");
        assert!(events[0].said.starts_with("failed:"), "{}", events[0].said);
    }

    /// Most of what comes back is what a file holds or a command printed, and
    /// a run that worked has nothing to answer for.
    #[test]
    fn what_a_run_did_not_fail_at_is_not_an_event() {
        let (_dir, traces) = in_a_temporary_directory();
        let worked = serde_json::json!({
            "type": "user",
            "message": { "content": [{
                "type": "tool_result",
                "is_error": false,
                "content": "1\tthe whole file\n2\tand more of it"
            }] }
        });
        kept(&traces, "1", &line(1_700_000_001, worked));

        assert_eq!(traces.read("1", "").unwrap().events, Vec::new());
    }

    #[test]
    fn a_cursor_carries_on_from_where_the_last_read_stopped() {
        let (_dir, traces) = in_a_temporary_directory();
        let first = line(1_700_000_001, said("one"));
        kept(&traces, "1", &first);

        let read = traces.read("1", "").unwrap();
        assert_eq!(read.events.len(), 1);

        let at = traces.at("1").unwrap();
        fs::write(&at, format!("{first}{}", line(1_700_000_002, said("two")))).unwrap();

        let more = traces.read("1", &read.cursor).unwrap();
        assert_eq!(more.events.len(), 1);
        assert_eq!(more.events[0].said, "two");
    }

    /// A line longer than one read holds is stepped over rather than asked
    /// for again, which would never end.
    #[test]
    fn a_line_too_long_to_read_at_once_does_not_stop_the_rest() {
        let (_dir, traces) = in_a_temporary_directory();
        let long = "x".repeat(AT_MOST);
        kept(
            &traces,
            "1",
            &[
                line(1_700_000_001, said(&long)),
                line(1_700_000_002, said("after")),
            ]
            .concat(),
        );

        let read = traces.read("1", "").unwrap();
        assert_eq!(read.events, Vec::new());

        let more = traces.read("1", &read.cursor).unwrap();
        assert_eq!(more.events.len(), 1, "{:?}", more.events);
        assert_eq!(more.events[0].said, "after");
    }

    /// Two reads that make no progress are a reader that never finishes.
    #[test]
    fn every_read_moves_the_cursor_on() {
        let (_dir, traces) = in_a_temporary_directory();
        kept(
            &traces,
            "1",
            &line(1_700_000_001, said(&"x".repeat(AT_MOST))),
        );

        let read = traces.read("1", "").unwrap();
        assert_ne!(read.cursor, cursor_at(0));
    }

    /// A run works several levels below a temporary root, and the whole of
    /// that in front of every path leaves no room for the file itself.
    #[test]
    fn a_path_reads_as_it_stands_in_the_work_area() {
        assert_eq!(
            within_the_work_area("/var/x/state/data/cistern/worktrees/1/app/scoring.py"),
            "app/scoring.py"
        );
        assert_eq!(within_the_work_area("/etc/hosts"), "/etc/hosts");
    }

    /// The last line the vendor writes repeats what the agent just said.
    #[test]
    fn the_answer_is_not_an_event_of_its_own() {
        let (_dir, traces) = in_a_temporary_directory();
        let answer = serde_json::json!({ "type": "result", "result": "All done!" });
        kept(&traces, "1", &line(1_700_000_001, answer));

        assert_eq!(traces.read("1", "").unwrap().events, Vec::new());
    }

    #[test]
    fn a_long_line_is_shortened_where_a_person_would_stop_reading() {
        assert_eq!(shortened("  one   two  "), "one two");
        assert!(shortened(&"x".repeat(500)).ends_with("..."));
        assert!(shortened(&"x".repeat(500)).len() < 120);
    }
}
