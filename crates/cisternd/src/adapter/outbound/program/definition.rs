//! What one vendor is, as a file says it.
//!
//! Everything a vendor calls its own is here rather than in the code: the program, its
//! arguments, the words it uses for a run that hit a ceiling, and where in its answer each
//! figure is found. Adding a vendor is a file, not a build.
//!
//! What stays in the code is the means. Running a child process and reading a path are the
//! same whoever the vendor is, and `reader` picks between the few shapes an answer arrives
//! in by name.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::core::port::outbound::Unavailable;

/// The default this ships with.
///
/// It travels in the binary rather than being written to disk. Nothing on disk means
/// nothing for an upgrade to overwrite and nothing for a reader to wonder who owns.
const SHIPPED: &[(&str, &str)] = &[("claude", include_str!("claude.toml"))];

/// One vendor, read from a file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub program: String,
    /// The arguments as they are written, with the places still in them.
    ///
    /// Grouped so that a group holding a place nobody filled is dropped whole. A task that
    /// named no model has to lose `--model` along with the value.
    pub args: Vec<Vec<String>>,
    /// What leads the prompt.
    pub goal: String,
    /// How many turns one run may take, and how much it may spend, before it is cut off.
    ///
    /// A guard against a run that goes nowhere rather than the session's own ceiling.
    pub turns: String,
    pub spend: String,
    pub answer: Answer,
}

/// Where each figure is found in what the program answered.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    /// Which of the shapes an answer arrives in this one is.
    ///
    /// A name rather than a description, because reading a shape is code. A shape nobody
    /// has written yet is the one thing here that a file cannot add.
    pub reader: Reader,
    /// Where the word for how the run ended is.
    pub outcome: String,
    /// The words that mean the run hit a ceiling, and what to report for each.
    ///
    /// A sentence rather than the word alone. A run cut off at a ceiling answers with no
    /// text of its own, so the word it gives instead is all there is to report, and handing
    /// back the word would put a vendor's name where a sentence belongs.
    pub at_ceiling: BTreeMap<String, String>,
    /// Where what the run said about itself is, for a run that failed.
    pub said: String,
    /// Where the price is, and what to multiply it by to reach millionths.
    pub cost: String,
    pub cost_scale: f64,
    pub input: String,
    pub output: String,
    pub cache_written: String,
    pub cache_read: String,
}

/// The shapes an answer arrives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Reader {
    /// One object per line, the last of which is the answer and the rest the trace.
    #[serde(rename = "last-json-line")]
    LastJsonLine,
}

impl Definition {
    /// Reads one, and says which file it could not read rather than only what was wrong.
    pub fn parse(named: &str, written: &str) -> Result<Self, Unavailable> {
        toml::from_str(written).map_err(|e| Unavailable::new(format!("{named}: {e}")))
    }

    /// The vendor of this name, from what the user wrote or from what ships.
    ///
    /// The user's wins. A file they placed is theirs to keep, and an upgrade that replaced
    /// it would take away the only reason to place one.
    pub fn of(name: &str, written: Option<&str>) -> Result<Self, Unavailable> {
        if let Some(written) = written {
            return Definition::parse(name, written);
        }
        let shipped = SHIPPED
            .iter()
            .find(|(shipped, _)| *shipped == name)
            .ok_or_else(|| Unavailable::new(format!("no definition for vendor {name}")))?;
        Definition::parse(name, shipped.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file travels in the binary, so a build that broke it would fail every task.
    #[test]
    fn the_definition_that_ships_is_readable() {
        let claude = Definition::of("claude", None).unwrap();
        assert_eq!(claude.program, "claude");
        assert_eq!(claude.answer.reader, Reader::LastJsonLine);
        assert!(claude.goal.starts_with("/goal "), "{}", claude.goal);
    }

    /// A task that named no model must not hand the program a flag with nothing after it,
    /// which is why the arguments are written in groups.
    #[test]
    fn the_definition_that_ships_holds_the_places_it_must() {
        let claude = Definition::of("claude", None).unwrap();
        let written: String = claude.args.iter().flatten().cloned().collect();
        for place in ["{goal}", "{instruction}", "{model}", "{turns}", "{spend}"] {
            assert!(written.contains(place), "{place} is not in claude.toml");
        }
    }

    #[test]
    fn a_file_the_user_placed_is_read_instead_of_the_one_that_ships() {
        let theirs = r#"
            program = "elsewhere"
            args = [["-p", "{instruction}"]]
            goal = "/goal done"
            turns = "10"
            spend = "1"
            [answer]
            reader = "last-json-line"
            outcome = "subtype"
            said = "result"
            cost = "cost"
            cost_scale = 1.0
            input = "in"
            output = "out"
            cache_written = "cw"
            cache_read = "cr"
            [answer.at_ceiling]
        "#;
        let read = Definition::of("claude", Some(theirs)).unwrap();
        assert_eq!(read.program, "elsewhere");
    }

    #[test]
    fn a_name_nobody_defined_is_refused() {
        let refused = Definition::of("codex", None).unwrap_err();
        assert!(refused.reason.contains("codex"), "{}", refused.reason);
    }

    /// A file naming a field this does not have is a file written against another version.
    #[test]
    fn a_field_the_format_does_not_have_fails() {
        let odd = r#"
            program = "x"
            colour = "red"
            args = []
            goal = ""
            turns = "1"
            spend = "1"
            [answer]
            reader = "last-json-line"
            outcome = "s"
            said = "r"
            cost = "c"
            cost_scale = 1.0
            input = "i"
            output = "o"
            cache_written = "cw"
            cache_read = "cr"
            [answer.at_ceiling]
        "#;
        assert!(Definition::parse("theirs.toml", odd).is_err());
    }
}
