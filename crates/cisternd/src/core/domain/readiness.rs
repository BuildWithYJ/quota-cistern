//! Whether an instruction carries enough for a run to take it on unattended.
//!
//! A run cannot stop to ask, so an instruction that points at no place to work and gives no way
//! to tell the work is done spends its budget on a guess. These signals are read by rule, not by
//! a model: a rule answers the same way every time and can name what it did not find. A paired
//! experiment, over full instructions and the same instructions with their detail removed, found
//! these two signals told the two apart almost cleanly where a model did not.

/// What an instruction carries toward running unattended.
///
/// Each field is present-or-absent, read from the text alone. Scope is not read here yet: an open
/// wish is the least certain of the three to judge by rule, and turning good tasks away is the
/// cost of judging it too early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Readiness {
    /// The instruction points at where to work: a path, a call, a `::` path, or backticks.
    pub place: bool,
    /// The instruction gives a way to tell the work is done: a code block, a command, a
    /// reproduction, or an expected result.
    pub check: bool,
}

impl Readiness {
    /// Reads the signals out of an instruction.
    pub fn read(instruction: &str) -> Self {
        let lowered = instruction.to_ascii_lowercase();
        Readiness {
            place: points_at_a_place(instruction),
            check: gives_a_check(&lowered),
        }
    }

    /// Whether the task carries enough to run unattended.
    pub fn ready(&self) -> bool {
        self.place && self.check
    }

    /// What the instruction does not say, as a phrase, for one that is not ready.
    ///
    /// It reads back the signals that are absent, so the author is told what to add rather than
    /// to write the instruction over.
    pub fn missing(&self) -> String {
        match (self.place, self.check) {
            (false, false) => "where to work, or how to tell it is done".to_owned(),
            (false, true) => "where to work".to_owned(),
            (true, false) => "how to tell it is done".to_owned(),
            // Not reached while `ready` is false, but a total function needs the arm.
            (true, true) => "what it is missing".to_owned(),
        }
    }
}

/// Does the instruction point at where to work?
///
/// Backticks answer on their own only when what they quote is a name. Otherwise a word carries
/// the signal: a file with a code extension, a call, or a path written with `.` or `::`.
fn points_at_a_place(instruction: &str) -> bool {
    if quotes_a_name(instruction) {
        return true;
    }
    instruction.split(char::is_whitespace).any(|word| {
        let token = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        is_a_call(word)
            || token.contains("::")
            || has_a_code_extension(token)
            || is_a_dotted_name(token)
    })
}

/// Does a pair of backticks quote a name?
///
/// Quoting is how a name is set apart from the prose around it, so a span that holds no
/// whitespace is read as one. A quoted phrase is prose still, and falls through to the word test
/// like any other text. Only a span a second backtick closes is read; a lone backtick quotes
/// nothing.
fn quotes_a_name(instruction: &str) -> bool {
    let spans: Vec<&str> = instruction.split('`').collect();
    // The spans alternate outside, inside, outside; the last is never closed by a backtick.
    spans
        .iter()
        .enumerate()
        .take(spans.len().saturating_sub(1))
        .any(|(at, span)| {
            at % 2 == 1
                && !span.contains(char::is_whitespace)
                && span.chars().any(|c| c.is_ascii_alphanumeric())
        })
}

/// A word that names a function, such as `parse(` or `search()`.
fn is_a_call(word: &str) -> bool {
    word.as_bytes()
        .windows(2)
        .any(|pair| pair[0].is_ascii_alphanumeric() && pair[1] == b'(')
}

/// A file name whose extension says it is code.
fn has_a_code_extension(token: &str) -> bool {
    const EXTENSIONS: [&str; 22] = [
        ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".rb", ".java", ".c", ".cc", ".cpp",
        ".h", ".hpp", ".cs", ".php", ".sh", ".toml", ".json", ".yaml", ".yml", ".sql",
    ];
    let lowered = token.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .any(|ext| lowered.ends_with(ext) && lowered.len() > ext.len())
}

/// A name written with `.`, such as `store.tasks`, told apart from prose like `e.g`.
///
/// Every part must read as an identifier, at least one must be longer than an abbreviation, and at
/// least one must hold a letter, so a version like `3.14` does not count.
fn is_a_dotted_name(token: &str) -> bool {
    if !token.contains('.') {
        return false;
    }
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    let each_is_an_identifier = parts.iter().all(|part| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    });
    let one_is_long_enough = parts.iter().any(|part| part.len() >= 3);
    let one_holds_a_letter = parts
        .iter()
        .any(|part| part.chars().any(|c| c.is_ascii_alphabetic()));
    each_is_an_identifier && one_is_long_enough && one_holds_a_letter
}

/// Does the instruction give a way to tell the work is done?
///
/// Three rules, read over the lowered text: a mark that opens a transcript, a word that names a
/// check, and a command that runs one. They are a first set. A later change may widen them.
fn gives_a_check(lowered: &str) -> bool {
    opens_a_transcript(lowered) || names_a_check(lowered) || runs_a_command(lowered)
}

/// A mark that opens a block of output to compare against.
fn opens_a_transcript(lowered: &str) -> bool {
    const MARKS: [&str; 2] = ["```", ">>>"];
    MARKS.iter().any(|mark| lowered.contains(mark))
}

/// A word that names a check.
///
/// The cue has to open a word, so that what surrounds the word cannot hide it: `expected:` and a
/// `Tests` that opens a line both count, while `latest` does not.
fn names_a_check(lowered: &str) -> bool {
    const OPENINGS: [&str; 6] = [
        "test",
        "assert",
        "reproduc",
        "expected",
        "pytest",
        "traceback",
    ];
    lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| OPENINGS.iter().any(|opening| word.starts_with(opening)))
}

/// A command that tells the work is done, such as `scripts/check.sh` or `cargo check`.
///
/// A script answers by its extension. The rest are named, because the word they are built on --
/// `check`, `build` -- is too common in prose to read on its own.
fn runs_a_command(lowered: &str) -> bool {
    const COMMANDS: [&str; 8] = [
        "cargo check",
        "cargo clippy",
        "cargo build",
        "npm run",
        "make check",
        "make test",
        "go build",
        "go vet",
    ];
    if COMMANDS.iter().any(|command| lowered.contains(command)) {
        return true;
    }
    lowered
        .split(|c: char| c.is_ascii_whitespace() || c == '`')
        .any(|word| {
            let token = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            token.len() > 3 && token.ends_with(".sh")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_and_a_check_read_ready() {
        let read = Readiness::read("fix parse() in src/util.rs; run cargo test util");
        assert!(read.place);
        assert!(read.check);
        assert!(read.ready());
    }

    #[test]
    fn no_place_is_not_ready_and_says_where() {
        let read = Readiness::read("the thing is broken; the tests should pass");
        assert!(!read.place);
        assert!(!read.ready());
        assert!(read.missing().contains("where"), "{}", read.missing());
    }

    #[test]
    fn no_check_is_not_ready_and_says_how() {
        let read = Readiness::read("edit the parser in src/parse.rs");
        assert!(read.place);
        assert!(!read.check);
        assert!(!read.ready());
        assert!(read.missing().contains("done"), "{}", read.missing());
    }

    #[test]
    fn a_dotted_name_or_a_quoted_name_is_a_place_but_prose_is_not() {
        assert!(points_at_a_place("look at store.tasks"));
        assert!(points_at_a_place("look at `TaskStore`"));
        assert!(!points_at_a_place("do it, e.g. the obvious way"));
    }

    /// Backticks around a phrase quote no name, so they do not stand in for a place.
    #[test]
    fn a_quoted_phrase_is_not_a_place() {
        assert!(!points_at_a_place("`please make the thing faster`"));
        assert!(!Readiness::read("`please make the thing faster`; add a test").ready());
        // A lone backtick closes nothing, so it quotes nothing.
        assert!(!points_at_a_place("make it faster `"));
    }

    /// A command is a way to tell the work is done, quoted or bare.
    #[test]
    fn a_command_is_a_check() {
        assert!(Readiness::read("fix `src/lib.rs`; run `scripts/check.sh`").ready());
        assert!(Readiness::read("fix src/lib.rs; run scripts/check.sh").ready());
        assert!(Readiness::read("fix src/lib.rs until cargo check passes").ready());
    }

    /// A cue is read as a whole word, so punctuation and line breaks cannot hide it.
    #[test]
    fn a_cue_is_read_as_a_word() {
        assert!(Readiness::read("fix src/util.rs; expected: no panic").check);
        assert!(Readiness::read("fix src/util.rs\nTests must pass").check);
        // The cue has to open the word: `latest` is not a test.
        assert!(!gives_a_check("ship the latest src/util.rs"));
    }

    /// The instructions the documentation shows are ones the gate lets through.
    ///
    /// The examples are what a reader types first, so a gate that turns them back turns back the
    /// first command of the tool. Any change here is a change to `README.md` and `docs/cli.md`.
    #[test]
    fn the_documented_examples_are_ready() {
        // The Korean pages show the same examples, around the same path and command.
        for instruction in [
            "tidy up src/utils/format.rs; cargo test utils passes",
            "add a --json flag to src/cli.rs; scripts/check.sh passes",
        ] {
            assert!(
                Readiness::read(instruction).ready(),
                "the documentation shows: {instruction}"
            );
        }
    }

    /// The presence of concrete detail decides, not the length of the text.
    #[test]
    fn length_does_not_decide_readiness() {
        let short = Readiness::read("`api.rs`: add a --json flag; `cargo test api` passes");
        assert!(short.ready());

        let long = Readiness::read(
            "We have been thinking for a long while about the overall quality of the experience \
             and it would be really nice if things felt faster and cleaner and generally better \
             for everyone involved, somehow, when they get a chance to look at it.",
        );
        assert!(!long.ready());
    }
}
