//! What one vendor is, as a file says it.
//!
//! Everything a vendor calls its own is here rather than in the code: the program, its
//! arguments, the words it uses for a run that hit a ceiling, and where in its answer each
//! figure is found. Adding a vendor is a file, not a build.
//!
//! What stays in the code is the means. Running a child process and reading a path are the
//! same whoever the vendor is, and `reader` picks between the few shapes an answer arrives
//! in by name.

use std::{collections::BTreeMap, env, ffi::OsString, fs, path::PathBuf};

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
    pub limit: Limit,
    pub trace: Trace,
}

/// How much of the vendor's allowance is left, and how to find out.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    /// Which of the ways of asking this vendor answers to.
    pub reader: LimitReader,
    pub program: String,
    /// The arguments, with `{settings}` standing for the file the reader writes.
    pub args: Vec<Vec<String>>,
    /// What the reader writes for the vendor to load, with `{script}` standing for the
    /// program it should call.
    pub settings: String,
    /// What to type once the session is waiting, since the figure is empty until an answer
    /// has come back.
    pub prompt: String,
    /// What the screen says at the two moments something has to be typed.
    pub trusts: String,
    pub ready: String,
    /// Where the figure and the moment it starts over are, and what to multiply the figure
    /// by to reach hundredths of a percent.
    pub used: String,
    pub used_scale: f64,
    pub resets_at: String,
    /// How long to wait altogether, how long to leave the session alone before typing, and
    /// how long to wait for the screen to say more, in seconds and milliseconds.
    pub give_up_after: u64,
    pub settles_in: u64,
    pub between_looks_ms: u64,
    /// A screen wide enough that the vendor lays it out as usual.
    pub rows: u16,
    pub cols: u16,
}

/// The ways of asking a vendor where its allowance stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum LimitReader {
    /// Run it with a terminal attached and read the figure off its status line.
    #[serde(rename = "status-line")]
    StatusLine,
}

/// What one line of what a run wrote amounts to.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    /// The kind of line carrying what the agent said, and the kind carrying what came back
    /// from something it reached for.
    pub said: String,
    pub came_back: String,
    /// Where the blocks are inside a line.
    pub blocks: String,
    /// The kinds of block: what it said, what it reached for, and what came back.
    pub text: String,
    pub reached_for: String,
    pub result: String,
    /// The flag on a result that did not work.
    pub errored: String,
    /// Which argument of a tool call names what it acted on, most telling first.
    pub subject: Vec<String>,
    /// The same, for arguments holding a path, which is shown from the work area down.
    pub subject_path: Vec<String>,
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

/// Where a user puts a definition of their own.
///
/// Under the configuration home rather than beside what ships, so that an upgrade has
/// nothing of theirs to overwrite and they have nothing of ours to keep in step.
fn placed_in(config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match config_home {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(home?).join(".config"),
    };
    Some(base.join("cistern").join("vendors"))
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

    /// The vendor of this name, taking a file the user placed over the one that ships.
    pub fn found(name: &str) -> Result<Self, Unavailable> {
        Definition::of(name, placed(name)?.as_deref())
    }

    /// Every name there is a definition for, the user's and the ones that ship.
    ///
    /// What the configuration may be set to. A name with no definition behind it would be
    /// accepted and then fail on the next task, which is later than it needs to be.
    pub fn known() -> Vec<String> {
        let mut names: Vec<String> = SHIPPED.iter().map(|(name, _)| (*name).to_owned()).collect();
        let Some(at) = placed_in(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME")) else {
            return names;
        };
        let Ok(held) = fs::read_dir(at) else {
            return names;
        };
        for one in held.flatten() {
            let path = one.path();
            if path.extension().is_none_or(|end| end != "toml") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|name| name.to_str())
                && !names.iter().any(|known| known == name)
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        names
    }
}

/// What the user wrote for this vendor, if they wrote anything.
///
/// A directory that is not there is nobody having written one. A file that is there and
/// cannot be read is not, since carrying on with the shipped default would quietly ignore
/// what they asked for.
fn placed(name: &str) -> Result<Option<String>, Unavailable> {
    let Some(at) = placed_in(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME")) else {
        return Ok(None);
    };
    let at = at.join(format!("{name}.toml"));
    match fs::read_to_string(&at) {
        Ok(written) => Ok(Some(written)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Unavailable::new(format!("{}: {e}", at.display()))),
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
        let theirs = SHIPPED[0]
            .1
            .replace(r#"program = "claude""#, r#"program = "elsewhere""#);
        let read = Definition::of("claude", Some(&theirs)).unwrap();
        assert_eq!(read.program, "elsewhere");
    }

    #[test]
    fn a_name_nobody_defined_is_refused() {
        let refused = Definition::of("codex", None).unwrap_err();
        assert!(refused.reason.contains("codex"), "{}", refused.reason);
    }

    fn some(s: &str) -> Option<OsString> {
        Some(OsString::from(s))
    }

    #[test]
    fn the_configuration_home_is_where_a_user_puts_one() {
        assert_eq!(
            placed_in(some("/x/.config"), some("/home/a")),
            Some(PathBuf::from("/x/.config/cistern/vendors"))
        );
        assert_eq!(
            placed_in(None, some("/home/a")),
            Some(PathBuf::from("/home/a/.config/cistern/vendors"))
        );
        assert_eq!(placed_in(None, None), None);
    }

    /// A file naming a field this does not have is a file written against another version.
    #[test]
    fn a_field_the_format_does_not_have_fails() {
        let odd = format!("colour = \"red\"\n{}", SHIPPED[0].1);
        assert!(Definition::parse("theirs.toml", &odd).is_err());
    }
}
