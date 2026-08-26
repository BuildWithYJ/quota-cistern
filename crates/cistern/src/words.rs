//! The words this command says on its own behalf, in the language the author wrote in.
//!
//! What a part should say, and what to ask about one nobody settled, are the model's and arrive
//! already written in the author's language. These are the rest: the line that says the core is
//! working, the mark beside a part nobody settled, the question that ends it. A screen that asks
//! in Korean inside an English frame is one a person reads twice.
//!
//! Two languages, told apart by what the author typed. Nothing is configured, because there is
//! nothing to configure: a person writing in Korean is answered in Korean, and the same command
//! answers the next person in theirs.
//!
//! Written as escapes because a source file here holds ASCII only, the way
//! `crates/cistern/src/task.rs` writes the mark it prints beside a waiting task.

/// The language a person is answered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Korean,
}

impl Language {
    /// The language the author wrote in.
    ///
    /// Korean where they wrote any of it. A line mixing the two -- which is how code is written
    /// about in Korean, since the code itself is not -- is Korean, because the words around the
    /// code are what a person reads.
    pub fn of(wrote: &str) -> Self {
        match wrote.chars().any(is_korean) {
            true => Language::Korean,
            false => Language::English,
        }
    }

    /// The line printed while the core works out what an instruction meant.
    pub fn working_out(self, seconds: u64) -> String {
        match self {
            Language::English => format!("working out what that means... {seconds}s"),
            Language::Korean => format!(
                "\u{bb34}\u{c2a8} \u{b73b}\u{c778}\u{c9c0} \u{c54c}\u{c544}\u{b0b4}\u{b294} \u{c911}... {seconds}\u{cd08}"
            ),
        }
    }

    /// What is written beside a part nobody has settled.
    pub fn unsettled(self) -> &'static str {
        match self {
            Language::English => "   <- nobody has settled this",
            Language::Korean => {
                "   <- \u{c544}\u{c9c1} \u{c815}\u{d574}\u{c9c0}\u{c9c0} \u{c54a}\u{c74c}"
            }
        }
    }

    /// What stands over the list of what is still unsettled.
    pub fn left_over(self, count: usize) -> String {
        match self {
            Language::English => format!(
                "{count} {} nobody has settled. Left as {}, the agent decides {} by itself:",
                match count {
                    1 => "thing",
                    _ => "things",
                },
                match count {
                    1 => "it is",
                    _ => "they are",
                },
                match count {
                    1 => "it",
                    _ => "each of them",
                },
            ),
            Language::Korean => format!(
                "\u{c544}\u{c9c1} \u{c815}\u{d574}\u{c9c0}\u{c9c0} \u{c54a}\u{c740} \u{ac83}\u{c774} {count}\u{ac1c} \u{c788}\u{c2b5}\u{b2c8}\u{b2e4}. \u{c774}\u{b300}\u{b85c} \u{b450}\u{ba74} \u{c5d0}\u{c774}\u{c804}\u{d2b8}\u{ac00} \u{c54c}\u{c544}\u{c11c} \u{c815}\u{d569}\u{b2c8}\u{b2e4}:"
            ),
        }
    }

    /// What is said when nobody wrote a question about a part nobody settled.
    pub fn asking(self) -> &'static str {
        match self {
            Language::English => "Nothing settles this. What should it be?",
            Language::Korean => {
                "\u{c774} \u{d56d}\u{baa9}\u{c774} \u{c815}\u{d574}\u{c9c0}\u{c9c0} \u{c54a}\u{c558}\u{c2b5}\u{b2c8}\u{b2e4}. \u{bb34}\u{c5c7}\u{c73c}\u{b85c} \u{d560}\u{ae4c}\u{c694}?"
            }
        }
    }

    /// The last thing on a list of answers: none of them.
    pub fn type_your_own(self) -> &'static str {
        match self {
            Language::English => "type your own",
            Language::Korean => "\u{c9c1}\u{c811} \u{c785}\u{b825}",
        }
    }

    /// What is said to a number nobody offered.
    pub fn no_such(self, at: usize) -> String {
        match self {
            Language::English => format!("there is no {at} on the list"),
            Language::Korean => format!(
                "\u{baa9}\u{b85d}\u{c5d0} {at}\u{bc88}\u{c740} \u{c5c6}\u{c2b5}\u{b2c8}\u{b2e4}"
            ),
        }
    }

    /// The question that ends it, and what the answers to it mean.
    pub fn register(self, parts: usize) -> String {
        match self {
            Language::English => {
                format!("Register it? [enter=yes / 1-{parts}=change that line / n=no] ")
            }
            Language::Korean => format!(
                "\u{c774}\u{b300}\u{b85c} \u{b4f1}\u{b85d}\u{d560}\u{ae4c}\u{c694}? [\u{c5d4}\u{d130}=\u{c608} / 1-{parts}=\u{d574}\u{b2f9} \u{c904} \u{c218}\u{c815} / n=\u{c544}\u{b2c8}\u{c624}] "
            ),
        }
    }

    /// What is said where nobody is at the terminal to settle anything.
    pub fn nobody_here(self, count: usize) -> String {
        match self {
            Language::English => format!(
                "nobody is here to settle {}, so nothing was registered. --force registers the instruction as written",
                match count {
                    1 => "it".to_owned(),
                    many => format!("{many} of them"),
                }
            ),
            Language::Korean => format!(
                "\u{c815}\u{d574} \u{c904} \u{c0ac}\u{b78c}\u{c774} \u{c5c6}\u{c5b4}\u{c11c} \u{c544}\u{bb34}\u{ac83}\u{b3c4} \u{b4f1}\u{b85d}\u{d558}\u{c9c0} \u{c54a}\u{c558}\u{c2b5}\u{b2c8}\u{b2e4}. \u{b0a8}\u{c740} \u{ac83} {count}\u{ac1c}. --force \u{b97c} \u{c8fc}\u{ba74} \u{c801}\u{d78c} \u{adf8}\u{b300}\u{b85c} \u{b4f1}\u{b85d}\u{d569}\u{b2c8}\u{b2e4}"
            ),
        }
    }

    /// What stands before the other places a part could have named.
    pub fn also(self) -> &'static str {
        match self {
            Language::English => "also",
            Language::Korean => "\u{b610}\u{b294}",
        }
    }
}

/// Whether the character is one Korean is written in.
///
/// The syllables, and the letters they are built from, which is what an unfinished word holds.
fn is_korean(c: char) -> bool {
    matches!(c, '\u{ac00}'..='\u{d7a3}' | '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_person_is_answered_in_the_language_they_wrote_in() {
        assert_eq!(Language::of("make it faster"), Language::English);
        assert_eq!(
            Language::of("\u{c774}\u{ac70} \u{c880} \u{ace0}\u{ccd0}\u{c918}"),
            Language::Korean
        );
    }

    /// Code is written about in Korean; the code itself is not. What a person reads is the rest.
    #[test]
    fn a_line_mixing_the_two_is_the_one_the_words_are_in() {
        assert_eq!(
            Language::of("src/search.rs \u{b97c} \u{ace0}\u{ccd0}\u{c918}"),
            Language::Korean
        );
    }

    #[test]
    fn nothing_written_at_all_is_answered_in_english() {
        assert_eq!(Language::of(""), Language::English);
        assert_eq!(Language::of("src/search.rs"), Language::English);
    }

    /// Every line has both, so that a screen is never half one and half the other.
    #[test]
    fn both_languages_say_all_of_it() {
        for language in [Language::English, Language::Korean] {
            assert!(!language.working_out(3).is_empty());
            assert!(!language.unsettled().is_empty());
            assert!(!language.left_over(1).is_empty());
            assert!(!language.left_over(3).is_empty());
            assert!(!language.asking().is_empty());
            assert!(!language.type_your_own().is_empty());
            assert!(!language.no_such(9).is_empty());
            assert!(!language.register(6).is_empty());
            assert!(!language.nobody_here(2).is_empty());
            assert!(!language.also().is_empty());
        }
    }

    /// A count reaches the reader, since one thing left and three read differently.
    #[test]
    fn what_is_counted_is_said() {
        assert!(Language::Korean.left_over(3).contains('3'));
        assert!(Language::English.left_over(3).contains('3'));
        assert!(Language::English.left_over(1).contains("1 thing "));
        assert!(Language::Korean.no_such(9).contains('9'));
        assert!(Language::English.register(6).contains("1-6"));
        assert!(Language::Korean.register(6).contains("1-6"));
    }
}
