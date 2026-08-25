//! The vendor asked what a loose instruction is missing, by running the program a definition
//! describes.
//!
//! An ask, not a run: the model is given the instruction and what surrounds it and asked only to
//! name a place and a check. What it says is written into the instruction and checked by rule
//! before any run, so this reads and proposes and nothing else. A cheaper model answers first; a
//! stronger one is asked only when the cheaper found no place.
//!
//! Which program, which two models, what to hand them, and the words the answer is read by are
//! all in the definition. What stays here is the part that is the same whoever the vendor is.

use std::process::{Command, Stdio};

use crate::core::port::outbound::{Draft, Drafted, Drafter};

use super::Definition;

/// Proposes what a loose instruction is missing, by asking the vendor once.
pub struct ProgramDrafter {
    definition: Definition,
}

impl ProgramDrafter {
    pub fn new(definition: Definition) -> Self {
        ProgramDrafter { definition }
    }

    /// What the model is asked, as the definition writes it.
    fn asking(&self, ask: &Draft<'_>, files: &[String]) -> String {
        let drafting = &self.definition.drafter;
        let changed = match ask.changed.is_empty() {
            true => "(none)".to_owned(),
            false => ask.changed.join("\n"),
        };
        let listing = match files.is_empty() {
            true => "(unavailable)".to_owned(),
            false => files.join("\n"),
        };
        // One pass, as the agent's arguments are: an instruction holding the text `{files}`
        // must not come out carrying the listing.
        super::fill(
            drafting.prompt.trim(),
            &[
                ("instruction", ask.instruction),
                ("changed", &changed),
                ("files", &listing),
                ("place", &drafting.place),
                ("check", &drafting.check),
            ],
        )
    }

    /// Runs the program once and reads what it proposed, or nothing when it could not be reached.
    fn asked(&self, model: &str, repository: &str, prompt: &str) -> Option<Drafted> {
        let drafting = &self.definition.drafter;
        let done = Command::new(&drafting.program)
            .current_dir(repository)
            // It reads only its arguments. Closing stdin keeps it from waiting on input that,
            // asked as a one-shot from the daemon, never comes.
            .stdin(Stdio::null())
            .args(super::arguments(
                &drafting.args,
                &[
                    ("prompt", prompt),
                    ("model", model),
                    ("turns", &drafting.turns),
                ],
            ))
            .output()
            .ok()?;

        done.status.success().then(|| {
            read(
                &String::from_utf8_lossy(&done.stdout),
                &drafting.place,
                &drafting.check,
            )
        })
    }
}

impl Drafter for ProgramDrafter {
    fn draft(&self, ask: Draft<'_>) -> Option<Drafted> {
        let drafting = &self.definition.drafter;
        let prompt = self.asking(&ask, &files_in(ask.repository, drafting.files_shown));

        let cheap = self.asked(&drafting.cheaper, ask.repository, &prompt);
        if cheap
            .as_ref()
            .is_some_and(|drafted| drafted.place.is_some())
        {
            return cheap;
        }
        // The cheaper model found no place. A stronger one may, and only then is it worth its cost.
        self.asked(&drafting.stronger, ask.repository, &prompt)
            .or(cheap)
    }
}

/// The files the repository tracks, capped so the prompt stays small.
fn files_in(repository: &str, shown: usize) -> Vec<String> {
    let done = Command::new("git")
        .args(["-C", repository, "--no-pager", "ls-files"])
        .output();
    match done {
        Ok(done) if done.status.success() => String::from_utf8_lossy(&done.stdout)
            .lines()
            .take(shown)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// Reads a place and a check out of the model's two lines.
fn read(answer: &str, place: &str, check: &str) -> Drafted {
    Drafted {
        place: field(answer, place),
        check: field(answer, check),
    }
}

/// The value on the line that starts with the key, unless it is empty or "none".
fn field(answer: &str, key: &str) -> Option<String> {
    let key = format!("{key}:");
    for line in answer.lines() {
        if let Some(rest) = line.trim().strip_prefix(&key) {
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

    /// The words the shipped definition reads an answer by.
    const PLACE: &str = "PLACE";
    const CHECK: &str = "CHECK";

    fn shipped() -> Definition {
        Definition::of("claude", None).expect("the shipped definition parses")
    }

    #[test]
    fn a_place_and_a_check_are_read_from_two_lines() {
        let drafted = read("PLACE: src/login.rs\nCHECK: cargo test login", PLACE, CHECK);
        assert_eq!(drafted.place.as_deref(), Some("src/login.rs"));
        assert_eq!(drafted.check.as_deref(), Some("cargo test login"));
    }

    #[test]
    fn none_and_backticks_and_stray_lines_are_handled() {
        let drafted = read(
            "Sure, here you go:\nPLACE: `src/api.rs`\nCHECK: none\n",
            PLACE,
            CHECK,
        );
        assert_eq!(drafted.place.as_deref(), Some("src/api.rs"));
        assert_eq!(drafted.check, None);
    }

    #[test]
    fn an_answer_without_the_keys_proposes_nothing() {
        let drafted = read("I could not tell from what I was given.", PLACE, CHECK);
        assert_eq!(drafted.place, None);
        assert_eq!(drafted.check, None);
    }

    /// The words are the definition's, so an answer written in another vendor's is read too.
    #[test]
    fn the_keys_the_definition_names_are_the_ones_read() {
        let drafted = read("WHERE: src/api.rs\nHOW: cargo test api", "WHERE", "HOW");
        assert_eq!(drafted.place.as_deref(), Some("src/api.rs"));
        assert_eq!(drafted.check.as_deref(), Some("cargo test api"));
    }

    /// The vendor's names live in the definition and nowhere else, so what is asked and what
    /// runs both come from the file.
    #[test]
    fn what_is_asked_is_filled_in_from_the_definition() {
        let definition = shipped();
        let drafter = ProgramDrafter::new(definition);
        let asked = drafter.asking(
            &Draft {
                instruction: "make it faster",
                changed: &["src/search.rs".to_owned()],
                repository: "/work/api",
            },
            &["src/lib.rs".to_owned()],
        );

        assert!(asked.contains("make it faster"));
        assert!(asked.contains("src/search.rs"));
        assert!(asked.contains("src/lib.rs"));
        // The keys are written in from the same place the reader takes them from.
        assert!(asked.contains("PLACE:"));
        assert!(asked.contains("CHECK:"));
        // Nothing is left standing in the prompt.
        assert!(!asked.contains("{instruction}"));
        assert!(!asked.contains("{changed}"));
        assert!(!asked.contains("{files}"));
    }

    /// An instruction is filled in among the other places, so one that reads like a place of
    /// its own must not be filled in again.
    #[test]
    fn an_instruction_naming_a_place_is_written_as_it_stands() {
        let drafter = ProgramDrafter::new(shipped());
        let asked = drafter.asking(
            &Draft {
                instruction: "rename {files} to something else",
                changed: &[],
                repository: "/work/api",
            },
            &["src/lib.rs".to_owned()],
        );

        assert!(asked.contains("rename {files} to something else"));
    }

    /// Nothing uncommitted and no listing are both said rather than left blank, so the model
    /// is not left to read an empty heading as an omission.
    #[test]
    fn nothing_open_and_no_listing_are_both_said() {
        let drafter = ProgramDrafter::new(shipped());
        let asked = drafter.asking(
            &Draft {
                instruction: "it feels off",
                changed: &[],
                repository: "/work/api",
            },
            &[],
        );

        assert!(asked.contains("(none)"));
        assert!(asked.contains("(unavailable)"));
    }
}
