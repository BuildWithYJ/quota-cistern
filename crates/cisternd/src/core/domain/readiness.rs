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
            || is_a_path(token)
            || is_a_name_a_repository_keeps(token)
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

/// A path written with `/`, such as `src/utils`, told apart from a pair of words like `and/or`.
///
/// A directory names a place as plainly as a file does, and it carries no extension to say so.
/// Every part has to read as a name, and two parts alone do not settle it, because prose writes a
/// pair that way too: what settles it is a third part, or a part naming a directory a repository
/// keeps its source in.
fn is_a_path(token: &str) -> bool {
    if !token.contains('/') {
        return false;
    }
    let parts: Vec<&str> = token.split('/').collect();
    if parts.len() < 2 {
        return false;
    }
    let each_is_a_name = parts.iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    });
    let one_names_a_source_directory = parts
        .iter()
        .any(|part| KEPT_IN.contains(&part.to_ascii_lowercase().as_str()));
    each_is_a_name && (parts.len() > 2 || one_names_a_source_directory)
}

/// The directories a repository keeps its source in, as a first set.
const KEPT_IN: [&str; 11] = [
    "src", "crates", "lib", "app", "pkg", "cmd", "internal", "tests", "docs", "scripts", "bin",
];

/// A file a repository keeps by its name rather than by an extension, such as `Makefile`.
fn is_a_name_a_repository_keeps(token: &str) -> bool {
    const NAMES: [&str; 7] = [
        "makefile",
        "dockerfile",
        "readme",
        "changelog",
        "license",
        "justfile",
        "procfile",
    ];
    let lowered = token.to_ascii_lowercase();
    NAMES.contains(&lowered.as_str())
}

/// Does the instruction give a way to tell the work is done?
///
/// Three rules, read over the lowered text: a mark that opens a transcript, a word that names a
/// check, and a command that runs one. They are a first set. A later change may widen them.
fn gives_a_check(lowered: &str) -> bool {
    opens_a_transcript(lowered)
        || names_a_check(lowered)
        || names_a_check_in_korean(lowered)
        || runs_a_command(lowered)
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

/// A word that names a check, written in Korean.
///
/// This repository keeps its own documentation in two languages, so an instruction arrives in
/// either. A place reads the same in both -- a path is written the same way whatever surrounds it
/// -- but every cue for a check was English, which left an author writing in Korean reaching for
/// `--force` on instructions that named a check plainly.
///
/// Korean attaches its particles to the word, so a cue is read as a substring rather than as a
/// word opening: the cue for a test is inside the word that carries the subject particle. The
/// cues are written as escapes because a source file here holds ASCII only, the way
/// `crates/cistern/src/task.rs` writes the mark it prints beside a waiting task.
fn names_a_check_in_korean(lowered: &str) -> bool {
    const CUES: [&str; 5] = [
        "\u{d14c}\u{c2a4}\u{d2b8}", // test
        "\u{d1b5}\u{acfc}",         // pass
        "\u{c7ac}\u{d604}",         // reproduce
        "\u{ae30}\u{b300}",         // expect
        "\u{ac80}\u{c99d}",         // verify
    ];
    CUES.iter().any(|cue| lowered.contains(cue))
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

    /// A directory names a place as plainly as a file does.
    #[test]
    fn a_directory_path_is_a_place() {
        assert!(points_at_a_place(
            "rewrite crates/cisternd/src/core/service until cargo test passes"
        ));
        assert!(points_at_a_place("tidy up src/utils"));
        assert!(points_at_a_place("read docs/cli.md"));
        // A pair of words written with a slash is prose, not a path.
        assert!(!points_at_a_place("do it one way and/or the other"));
        // A path a scheme opens is not one of this repository's.
        assert!(!is_a_path("https://example.com"));
    }

    /// Some files a repository keeps by name, with no extension to say what they are.
    #[test]
    fn a_name_a_repository_keeps_is_a_place() {
        assert!(points_at_a_place("add a flag to the Makefile"));
        assert!(points_at_a_place("the Dockerfile is stale"));
        assert!(points_at_a_place("update the README"));
        assert!(!points_at_a_place("make the file better"));
    }

    /// An instruction written in Korean names its check in Korean.
    #[test]
    fn a_check_written_in_korean_is_a_check() {
        // "fix src/util.rs, the tests must pass"
        let read = Readiness::read(
            "src/util.rs \u{b97c} \u{ace0}\u{ccd0}\u{b77c}, \u{d14c}\u{c2a4}\u{d2b8}\u{ac00} \u{d1b5}\u{acfc}\u{d574}\u{c57c} \u{d55c}\u{b2e4}",
        );
        assert!(read.place);
        assert!(read.check);
        assert!(read.ready());

        // A wish in Korean names no place and no check, the way a wish in English does not.
        let wish = Readiness::read("\u{ac1c}\u{c120}\u{d574}\u{c918}");
        assert!(!wish.ready());
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
