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
/// Backticks quote a name whatever it is, so they answer on their own. Otherwise a word carries
/// the signal: a file with a code extension, a call, or a path written with `.` or `::`.
fn points_at_a_place(instruction: &str) -> bool {
    if instruction.contains('`') {
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
/// The cues are a first set, read over the lowered text. A later change may widen them.
fn gives_a_check(lowered: &str) -> bool {
    const CUES: [&str; 11] = [
        "```",
        ">>>",
        "traceback",
        "assert",
        "reproduc",
        "expected ",
        " test",
        "pytest",
        "cargo test",
        "npm test",
        "unit test",
    ];
    CUES.iter().any(|cue| lowered.contains(cue))
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
    fn a_dotted_name_or_a_backtick_is_a_place_but_prose_is_not() {
        assert!(points_at_a_place("look at store.tasks"));
        assert!(points_at_a_place("look at `TaskStore`"));
        assert!(!points_at_a_place("do it, e.g. the obvious way"));
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
