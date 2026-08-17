//! Where the vendor's allowance stands, read off its own status line.
//!
//! The only place that knows there is a terminal and a screen to read words off. Which
//! words, and where the figure sits once it arrives, come from the definition; the core is
//! handed a percentage and a time.

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

use super::{Definition, definition::LimitReader, path};

/// Reads the vendor's allowance the way its definition says to.
pub struct ProgramLimit {
    definition: Definition,
    /// Where the session is run, and where what it writes is kept.
    ///
    /// One place rather than a new one each time.
    /// The vendor asks whether a directory is trusted the first time it sees one.
    at: PathBuf,
}

impl ProgramLimit {
    /// Takes the place it is given.
    /// This is how a test reaches a temporary one.
    pub fn at(definition: Definition, at: PathBuf) -> Self {
        ProgramLimit { definition, at }
    }

    /// Beside the sessions, under the same directory `docs/cli.md` names.
    pub fn in_data_home(definition: Definition) -> Option<Self> {
        let base = match std::env::var_os("XDG_DATA_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(std::env::var_os("HOME")?)
                .join(".local")
                .join("share"),
        };
        Some(ProgramLimit::at(
            definition,
            base.join("cistern").join("limit"),
        ))
    }

    /// Lays out the place the session runs in and what it writes to.
    ///
    /// The status line is a program the vendor hands a JSON object on standard input. This
    /// writes one that keeps every object, which is the only way the figure leaves the
    /// session.
    fn laid_out(&self) -> Result<Held, Unavailable> {
        let held = Held {
            work: self.at.join("work"),
            script: self.at.join("status-line.sh"),
            settings: self.at.join("settings.json"),
            written: self.at.join("status-lines.jsonl"),
            screen: self.at.join("screen.txt"),
        };
        let failing = |e: std::io::Error| Unavailable::new(format!("{}: {e}", self.at.display()));

        fs::create_dir_all(&held.work).map_err(failing)?;
        let _ = fs::remove_file(&held.written);
        let _ = fs::remove_file(&held.screen);
        let writes_to = quoted(&held.written);
        write_runnable(
            &held.script,
            &format!("#!/bin/sh\ncat >> {writes_to}\nprintf '\\n' >> {writes_to}\nprintf ' '\n"),
        )
        .map_err(failing)?;
        fs::write(&held.settings, self.settings(&held.script)?).map_err(failing)?;
        Ok(held)
    }

    /// What the vendor is given to load, as JSON.
    ///
    /// The definition holds a table and a serializer writes it out, so a path with a quote or
    /// a backslash in it is the serializer's to escape rather than this file's to get right.
    fn settings(&self, script: &Path) -> Result<String, Unavailable> {
        let mut held = self.definition.limit.settings.clone();
        fill(&mut held, script);
        let as_json: Value = Deserialize::deserialize(held)
            .map_err(|e: toml::de::Error| Unavailable::new(format!("limit.settings: {e}")))?;
        serde_json::to_string(&as_json)
            .map_err(|e| Unavailable::new(format!("limit.settings: {e}")))
    }
}

/// Puts the reader's own script where the definition left a place for it.
fn fill(held: &mut toml::Value, script: &Path) {
    match held {
        toml::Value::String(written) => {
            *written = written.replace("{script}", &script.display().to_string());
        }
        toml::Value::Table(held) => held.iter_mut().for_each(|(_, one)| fill(one, script)),
        toml::Value::Array(held) => held.iter_mut().for_each(|one| fill(one, script)),
        _ => {}
    }
}

/// A path as one single-quoted word of a shell script.
///
/// A quote in the path would end the quoting and leave the rest of it to the shell. The path
/// comes from `XDG_DATA_HOME` or `HOME`, so what is in it is nobody's to promise here.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

/// The places one reading uses.
struct Held {
    work: PathBuf,
    script: PathBuf,
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

impl Limit for ProgramLimit {
    fn read(&self) -> Result<Reading, Unavailable> {
        let asking = &self.definition.limit;
        match asking.reader {
            LimitReader::StatusLine => (),
        }
        let held = self.laid_out()?;
        let failing = |e: &dyn std::fmt::Display| {
            Unavailable::new(format!("{}: {e}", self.definition.limit.program))
        };

        let screen_size = PtySize {
            rows: asking.rows,
            cols: asking.cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pty = native_pty_system()
            .openpty(screen_size)
            .map_err(|e| failing(&e))?;
        let mut command = CommandBuilder::new(&asking.program);
        for group in &asking.args {
            for token in group {
                command.arg(token.replace("{settings}", &held.settings.display().to_string()));
            }
        }
        command.cwd(&held.work);
        command.env("TERM", "xterm-256color");

        let mut running = pty.slave.spawn_command(command).map_err(|e| failing(&e))?;
        // The slave end is the child's.
        // Holding it open here would keep the read below waiting after the child is gone.
        drop(pty.slave);
        let mut screen = pty.master.try_clone_reader().map_err(|e| failing(&e))?;
        let mut typing = pty.master.take_writer().map_err(|e| failing(&e))?;

        // A terminal waiting to be typed at says nothing, and reading one blocks.
        // So the reading happens beside the watching.
        let (said, arriving) = mpsc::channel();
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(read) = screen.read(&mut chunk) {
                if read == 0 || said.send(chunk[..read].to_vec()).is_err() {
                    break;
                }
            }
        });

        let found = self.watch(&arriving, &mut typing, &held);

        let _ = running.kill();
        let _ = running.wait();
        found
    }
}

impl ProgramLimit {
    /// Reads the screen until the status line has carried the figure.
    ///
    /// Two moments need something typed: the vendor asks whether the directory is trusted,
    /// and then it waits for a prompt. The figure is empty until an answer has come back,
    /// so the prompt is what fills it.
    fn watch(
        &self,
        arriving: &Receiver<Vec<u8>>,
        typing: &mut Box<dyn std::io::Write + Send>,
        held: &Held,
    ) -> Result<Reading, Unavailable> {
        let asking = &self.definition.limit;
        let started = Instant::now();
        let give_up_after = Duration::from_secs(asking.give_up_after);
        let settles_in = Duration::from_secs(asking.settles_in);
        let between_looks = Duration::from_millis(asking.between_looks_ms);

        let mut seen = Vec::new();
        let mut trusted = false;
        let mut asked = false;

        while started.elapsed() < give_up_after {
            match arriving.recv_timeout(between_looks) {
                Ok(chunk) => seen.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            let said = plainly(&seen);
            if !trusted && said.contains(&asking.trusts) {
                let _ = typing.write_all(b"\r");
                let _ = typing.flush();
                trusted = true;
                continue;
            }
            if !asked && said.contains(&asking.ready) && started.elapsed() > settles_in {
                let _ = typing.write_all(format!("{}\r", asking.prompt).as_bytes());
                let _ = typing.flush();
                asked = true;
            }
            if asked && let Some(reading) = self.kept(&held.written) {
                return Ok(reading);
            }
        }

        // What the session put on the screen, for when nothing came back.
        let _ = fs::write(&held.screen, &seen);
        self.kept(&held.written).ok_or_else(|| {
            Unavailable::new(format!(
                "the vendor's status line said nothing about its limit; the screen is at {}",
                held.screen.display()
            ))
        })
    }

    /// The last status line that carried the figure, if one has.
    fn kept(&self, written: &Path) -> Option<Reading> {
        let held = fs::read_to_string(written).ok()?;
        held.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|one| self.reading_in(&one))
            .next_back()
    }

    /// What one status line says about the allowance, by the paths the definition names.
    fn reading_in(&self, one: &Value) -> Option<Reading> {
        let asking = &self.definition.limit;
        let used = path::total(one, &asking.used)?;
        let resets_at = path::total(one, &asking.resets_at)?;
        Some(Reading {
            used: ((used * asking.used_scale).round().max(0.0) as u64).to_string(),
            resets_at: (resets_at.round().max(0.0) as u64).to_string(),
        })
    }
}

/// What the screen says, with the control characters and the spacing between the letters taken out.
/// A terminal writes a word one letter at a time.
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

#[cfg(test)]
mod tests;
