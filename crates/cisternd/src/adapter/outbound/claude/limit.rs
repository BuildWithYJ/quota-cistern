//! Where the vendor's limit stands, read off its own status line.
//!
//! The only place that knows there is a status line, a terminal, or a screen
//! to read words off. The core is handed a percentage and a time.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Deserialize;
use serde_json::Value;

use crate::core::port::outbound::{Limit, Reading, Unavailable};

/// How the vendor is asked. Content, for the reason `claude::agent` gives.
const INVOCATION: &str = include_str!("claude-limit.json");

/// How long to wait for the status line to carry the limit.
///
/// A session has to start, be trusted, take a prompt, and hear back, so this
/// is longer than any one of those and shorter than a person's patience.
const GIVE_UP_AFTER: Duration = Duration::from_secs(90);

/// How long to leave the session alone before typing at it.
const SETTLES_IN: Duration = Duration::from_secs(4);

/// How long to wait for the screen to say something more before looking at
/// what it has said so far.
///
/// A terminal that has finished drawing says nothing until it is typed at, so
/// waiting for the next character would be waiting for one this has to send.
const BETWEEN_LOOKS: Duration = Duration::from_millis(200);

/// A terminal wide enough that the vendor lays its screen out as usual.
const SCREEN: PtySize = PtySize {
    rows: 40,
    cols: 120,
    pixel_width: 0,
    pixel_height: 0,
};

/// Reads the vendor's limit by running it with a terminal attached.
pub struct ClaudeLimit {
    invocation: Invocation,
    /// Where the session is run, and where what it writes is kept.
    ///
    /// One place rather than a new one each time, because the vendor asks
    /// whether a directory is trusted the first time it sees one and
    /// remembers the answer.
    at: PathBuf,
}

/// The invocation as its file holds it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    #[serde(rename = "_", default)]
    _about: Vec<String>,
    program: String,
    args: Vec<Vec<String>>,
    prompt: String,
    /// What the screen says while it is asking whether the directory is
    /// trusted.
    trusts: String,
    /// What the screen says once it is waiting to be typed at.
    ready: String,
}

impl ClaudeLimit {
    /// Reads how the vendor is asked, and takes the place to ask from.
    pub fn at(at: PathBuf) -> Result<Self, Unavailable> {
        let invocation: Invocation = serde_json::from_str(INVOCATION)
            .map_err(|e| Unavailable::new(format!("claude-limit.json: {e}")))?;
        Ok(ClaudeLimit { invocation, at })
    }

    /// Beside the sessions, under the same directory `docs/cli.md` names.
    pub fn in_data_home() -> Option<Result<Self, Unavailable>> {
        let base = match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var_os("HOME")?)
                .join(".local")
                .join("share"),
        };
        Some(ClaudeLimit::at(base.join("cistern").join("limit")))
    }

    /// Lays out the place the session runs in and what it writes to.
    ///
    /// The status line is a command the vendor runs and hands a JSON object
    /// on standard input. This writes one that keeps every object it is
    /// given, which is the only way the figure comes out of the session.
    fn laid_out(&self) -> Result<Held, Unavailable> {
        let held = Held {
            work: self.at.join("work"),
            reader: self.at.join("status-line.sh"),
            settings: self.at.join("settings.json"),
            written: self.at.join("status-lines.jsonl"),
            screen: self.at.join("screen.txt"),
        };
        let failing = |e: std::io::Error| Unavailable::new(format!("{}: {e}", self.at.display()));

        fs::create_dir_all(&held.work).map_err(failing)?;
        let _ = fs::remove_file(&held.written);
        let _ = fs::remove_file(&held.screen);
        write_runnable(
            &held.reader,
            &format!(
                "#!/bin/sh\ncat >> '{}'\nprintf '\\n' >> '{}'\nprintf ' '\n",
                held.written.display(),
                held.written.display()
            ),
        )
        .map_err(failing)?;
        fs::write(
            &held.settings,
            serde_json::json!({
                "statusLine": { "type": "command", "command": held.reader.display().to_string() }
            })
            .to_string(),
        )
        .map_err(failing)?;
        Ok(held)
    }
}

/// The places one reading uses.
struct Held {
    work: PathBuf,
    reader: PathBuf,
    settings: PathBuf,
    written: PathBuf,
    /// Where the screen is kept when nothing was read off it.
    screen: PathBuf,
}

fn write_runnable(at: &Path, content: &str) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(at, content)?;
    fs::set_permissions(at, fs::Permissions::from_mode(0o755))
}

impl Limit for ClaudeLimit {
    fn read(&self) -> Result<Reading, Unavailable> {
        let held = self.laid_out()?;
        let failing = |e: &dyn std::fmt::Display| {
            Unavailable::new(format!("{}: {e}", self.invocation.program))
        };

        let pty = native_pty_system()
            .openpty(SCREEN)
            .map_err(|e| failing(&e))?;
        let mut command = CommandBuilder::new(&self.invocation.program);
        for group in &self.invocation.args {
            for token in group {
                command.arg(token.replace("{settings}", &held.settings.display().to_string()));
            }
        }
        command.cwd(&held.work);
        command.env("TERM", "xterm-256color");

        let mut running = pty.slave.spawn_command(command).map_err(|e| failing(&e))?;
        // The slave end is the child's. Holding it open here would keep the
        // read below waiting after the child is gone.
        drop(pty.slave);
        let mut screen = pty.master.try_clone_reader().map_err(|e| failing(&e))?;
        let mut typing = pty.master.take_writer().map_err(|e| failing(&e))?;

        // Reading a terminal blocks until it has something to say, and a
        // terminal waiting to be typed at has nothing. The reading happens
        // beside the watching so that neither waits on the other.
        let (said, arriving) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(read) = screen.read(&mut chunk) {
                if read == 0 || said.send(chunk[..read].to_vec()).is_err() {
                    break;
                }
            }
        });

        let found = watch(&arriving, &mut typing, &self.invocation, &held);

        let _ = running.kill();
        let _ = running.wait();
        found
    }
}

/// Reads the screen until the status line has carried the limit.
///
/// Two moments need something typed. The vendor asks whether the directory is
/// trusted, which the first answer accepts, and then it waits, which is when
/// the prompt goes in. The limit is empty until an answer has come back, so
/// the prompt is what fills it.
fn watch(
    arriving: &Receiver<Vec<u8>>,
    typing: &mut Box<dyn std::io::Write + Send>,
    invocation: &Invocation,
    held: &Held,
) -> Result<Reading, Unavailable> {
    let written = &held.written;
    let started = Instant::now();
    let mut seen = Vec::new();
    let mut trusted = false;
    let mut asked = false;

    while started.elapsed() < GIVE_UP_AFTER {
        match arriving.recv_timeout(BETWEEN_LOOKS) {
            Ok(chunk) => seen.extend_from_slice(&chunk),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let said = plainly(&seen);
        if !trusted && said.contains(&invocation.trusts) {
            let _ = typing.write_all(b"\r");
            let _ = typing.flush();
            trusted = true;
            continue;
        }
        if !asked && said.contains(&invocation.ready) && started.elapsed() > SETTLES_IN {
            let _ = typing.write_all(format!("{}\r", invocation.prompt).as_bytes());
            let _ = typing.flush();
            asked = true;
        }
        if asked && let Some(reading) = kept(written) {
            return Ok(reading);
        }
    }

    // What the session put on the screen, for when nothing came back.
    let _ = fs::write(&held.screen, &seen);
    kept(written).ok_or_else(|| {
        Unavailable::new(format!(
            "the vendor's status line said nothing about its limit; the screen is at {}",
            held.screen.display()
        ))
    })
}

/// What the screen says, with the control characters and the spacing between
/// the letters taken out.
///
/// A terminal writes a word one letter at a time with positioning in between,
/// so the word is not in what arrives as a word.
fn plainly(seen: &[u8]) -> String {
    let mut said = String::new();
    let mut left = String::from_utf8_lossy(seen).into_owned();
    while let Some(at) = left.find('\u{1b}') {
        said.push_str(&left[..at]);
        let rest = &left[at + 1..];
        let ends = rest
            .find(|c: char| c.is_ascii_alphabetic() || c == '\u{7}')
            .map_or(rest.len(), |end| end + 1);
        left = rest[ends..].to_owned();
    }
    said.push_str(&left);
    said.retain(|c| !c.is_whitespace());
    said.to_lowercase()
}

/// The last status line that carried the limit, if one has.
fn kept(written: &Path) -> Option<Reading> {
    let held = fs::read_to_string(written).ok()?;
    held.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|one| reading_in(&one))
        .next_back()
}

/// A percentage as hundredths of one.
const HUNDREDTHS: f64 = 100.0;

/// The five-hour limit out of one status line.
///
/// Section 2.2 measures a session against the limit that starts over every
/// five hours. The longer one arrives alongside and is not read.
///
/// The vendor writes the percentage as a fraction, and one that lands on a
/// whole number lands a hair off it. It crosses as hundredths of a percent,
/// which is finer than a person is shown and holds every figure seen so far.
fn reading_in(one: &Value) -> Option<Reading> {
    let window = one.get("rate_limits")?.get("five_hour")?;
    let used = window.get("used_percentage")?.as_f64()?;
    Some(Reading {
        used: ((used * HUNDREDTHS).round().max(0.0) as u64).to_string(),
        resets_at: window.get("resets_at")?.as_u64()?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn the_invocation_that_ships_is_readable() {
        let held = TempDir::new().unwrap();
        assert!(ClaudeLimit::at(held.path().to_path_buf()).is_ok());
    }

    #[test]
    fn the_control_characters_a_terminal_writes_are_not_part_of_what_it_said() {
        let written = b"\x1b[1mYes, I \x1b[32mtrust\x1b[0m this folder\x1b[0m";
        assert_eq!(plainly(written), "yes,itrustthisfolder");
    }

    #[test]
    fn the_five_hour_limit_is_what_is_read_out_of_a_status_line() {
        let line = serde_json::json!({
            "rate_limits": {
                "five_hour": { "used_percentage": 7.000000000000001, "resets_at": 1786285800u64 },
                "seven_day": { "used_percentage": 44, "resets_at": 1786316400u64 }
            }
        });
        assert_eq!(
            reading_in(&line),
            Some(Reading {
                used: "700".to_owned(),
                resets_at: "1786285800".to_owned()
            })
        );
    }

    #[test]
    fn a_status_line_that_says_nothing_about_the_limit_is_not_a_reading() {
        assert_eq!(reading_in(&serde_json::json!({ "cost": {} })), None);
    }

    /// Runs the vendor. Not part of `cargo test`, since it costs a turn.
    #[test]
    #[ignore = "reaches the vendor"]
    fn the_vendor_says_where_its_limit_stands() {
        let held = TempDir::new().unwrap();
        let reading = ClaudeLimit::at(held.path().to_path_buf())
            .unwrap()
            .read()
            .unwrap();

        // Hundredths of a percent, so a full limit reads as ten thousand.
        let used: u64 = reading.used.parse().unwrap();
        assert!(used <= 10_000, "{used}");
        assert!(reading.resets_at.parse::<u64>().unwrap() > 0);
        println!(
            "used {}.{:02}%, resets at {}",
            used / 100,
            used % 100,
            reading.resets_at
        );
    }
}
