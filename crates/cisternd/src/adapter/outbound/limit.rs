//! Where the vendor's limit stands, read off its own status line.
//!
//! The only place that knows there is a status line, a terminal, or a screen
//! to read words off. The core is handed a percentage and a time.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::Deserialize;
use serde_json::Value;

use crate::core::port::outbound::{Limit, Reading, Unavailable};

/// How the vendor is asked. Content, for the reason `adapter::agent` gives.
const INVOCATION: &str = include_str!("claude-limit.json");

/// How long to wait for the status line to carry the limit.
///
/// A session has to start, be trusted, take a prompt, and hear back, so this
/// is longer than any one of those and shorter than a person's patience.
const GIVE_UP_AFTER: Duration = Duration::from_secs(90);

/// How long to leave the session alone before typing at it.
const SETTLES_IN: Duration = Duration::from_secs(4);

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
        };
        let failing = |e: std::io::Error| Unavailable::new(format!("{}: {e}", self.at.display()));

        fs::create_dir_all(&held.work).map_err(failing)?;
        let _ = fs::remove_file(&held.written);
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

        let found = watch(&mut screen, &mut typing, &self.invocation, &held.written);

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
    screen: &mut Box<dyn Read + Send>,
    typing: &mut Box<dyn std::io::Write + Send>,
    invocation: &Invocation,
    written: &Path,
) -> Result<Reading, Unavailable> {
    let started = Instant::now();
    let mut seen = Vec::new();
    let mut asked = false;

    while started.elapsed() < GIVE_UP_AFTER {
        let mut chunk = [0u8; 8192];
        match screen.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => seen.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }

        let said = plainly(&seen);
        if !asked && said.contains(&invocation.trusts) && !said.contains(&invocation.ready) {
            let _ = typing.write_all(b"\r");
            let _ = typing.flush();
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

    kept(written)
        .ok_or_else(|| Unavailable::new("the vendor's status line said nothing about its limit"))
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

/// The five-hour limit out of one status line.
///
/// Section 2.2 measures a session against the limit that starts over every
/// five hours. The longer one arrives alongside and is not read.
fn reading_in(one: &Value) -> Option<Reading> {
    let window = one.get("rate_limits")?.get("five_hour")?;
    Some(Reading {
        used: window.get("used_percentage")?.as_u64()?.to_string(),
        resets_at: window.get("resets_at")?.as_u64()?.to_string(),
    })
}
