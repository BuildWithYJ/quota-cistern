//! Claude asked what a loose instruction is missing.
//!
//! A one-shot call, not a run: the model is given the instruction and what surrounds it and asked
//! only to name a place and a check. What it says is written into the instruction and checked by
//! rule before any run, so this reads and proposes and nothing else. A cheaper model answers
//! first; a stronger one is asked only when the cheaper found no place.

use std::process::{Command, Stdio};

use crate::core::port::outbound::{Draft, Drafted, Drafter};

/// How many of the repository's files to show the model, so a prompt stays small.
const FILES_SHOWN: usize = 300;

/// How many turns one ask may take. A guard against a run that goes nowhere, not a task's ceiling.
const TURNS: &str = "5";

/// Proposes what a loose instruction is missing, by asking Claude.
pub struct ClaudeDrafter {
    program: String,
    /// The model asked first. Cheap, because this runs before every task a rule could not ready.
    cheaper: String,
    /// The model asked when the cheaper one found no place. Set to a stronger name as they land.
    stronger: String,
}

impl Default for ClaudeDrafter {
    fn default() -> Self {
        ClaudeDrafter {
            program: "claude".to_owned(),
            cheaper: "haiku".to_owned(),
            stronger: "sonnet".to_owned(),
        }
    }
}

impl Drafter for ClaudeDrafter {
    fn draft(&self, ask: Draft<'_>) -> Option<Drafted> {
        let prompt = prompt(&ask, &files_in(ask.repository));

        let cheap = ask_model(&self.program, &self.cheaper, ask.repository, &prompt);
        if cheap
            .as_ref()
            .is_some_and(|drafted| drafted.place.is_some())
        {
            return cheap;
        }
        // The cheaper model found no place. A stronger one may, and only then is it worth its cost.
        ask_model(&self.program, &self.stronger, ask.repository, &prompt).or(cheap)
    }
}

/// The files the repository tracks, capped so the prompt stays small.
fn files_in(repository: &str) -> Vec<String> {
    let done = Command::new("git")
        .args(["-C", repository, "--no-pager", "ls-files"])
        .output();
    match done {
        Ok(done) if done.status.success() => String::from_utf8_lossy(&done.stdout)
            .lines()
            .take(FILES_SHOWN)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// What the model is asked.
fn prompt(ask: &Draft<'_>, files: &[String]) -> String {
    let changed = match ask.changed.is_empty() {
        true => "(none)".to_owned(),
        false => ask.changed.join("\n"),
    };
    let listing = match files.is_empty() {
        true => "(unavailable)".to_owned(),
        false => files.join("\n"),
    };
    format!(
        "You are preparing a coding task to run unattended, with no one to answer questions.\n\n\
         Task instruction:\n{}\n\n\
         Files changed but not committed:\n{}\n\n\
         Files in the repository:\n{}\n\n\
         Name where the work should happen and how to tell it is done. Answer with exactly two \
         lines and nothing else:\n\
         PLACE: <one file path from the list above, or the word none>\n\
         CHECK: <a shell command or test that verifies the work, or the word none>",
        ask.instruction, changed, listing
    )
}

/// Runs Claude once and reads what it proposed, or nothing when it could not be reached.
fn ask_model(program: &str, model: &str, repository: &str, prompt: &str) -> Option<Drafted> {
    let done = Command::new(program)
        .current_dir(repository)
        // It reads only its arguments. Closing stdin keeps it from waiting on input that,
        // asked as a one-shot from the daemon, never comes.
        .stdin(Stdio::null())
        .args(["-p", prompt, "--model", model, "--max-turns", TURNS])
        .output()
        .ok()?;

    done.status
        .success()
        .then(|| read(&String::from_utf8_lossy(&done.stdout)))
}

/// Reads a place and a check out of the model's two lines.
fn read(answer: &str) -> Drafted {
    Drafted {
        place: field(answer, "PLACE:"),
        check: field(answer, "CHECK:"),
    }
}

/// The value on the line that starts with the key, unless it is empty or "none".
fn field(answer: &str, key: &str) -> Option<String> {
    for line in answer.lines() {
        if let Some(rest) = line.trim().strip_prefix(key) {
            let value = rest.trim().trim_matches('`').trim();
            if value.is_empty() || value.eq_ignore_ascii_case("none") {
                return None;
            }
            return Some(value.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_and_a_check_are_read_from_two_lines() {
        let drafted = read("PLACE: src/login.rs\nCHECK: cargo test login");
        assert_eq!(drafted.place.as_deref(), Some("src/login.rs"));
        assert_eq!(drafted.check.as_deref(), Some("cargo test login"));
    }

    #[test]
    fn none_and_backticks_and_stray_lines_are_handled() {
        let drafted = read("Sure, here you go:\nPLACE: `src/api.rs`\nCHECK: none\n");
        assert_eq!(drafted.place.as_deref(), Some("src/api.rs"));
        assert_eq!(drafted.check, None);
    }

    #[test]
    fn an_answer_without_the_keys_proposes_nothing() {
        let drafted = read("I could not tell from what I was given.");
        assert_eq!(drafted.place, None);
        assert_eq!(drafted.check, None);
    }
}
