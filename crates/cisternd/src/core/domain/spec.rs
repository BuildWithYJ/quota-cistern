//! What a run is given to work from, in the parts an unattended run needs each of.
//!
//! An unattended run cannot stop to ask, so every decision the spec leaves open is one the agent
//! makes on its own. Naming the parts is what makes those decisions countable: a part nobody has
//! settled is a decision still to be made, and a spec with none left is one a run can be given.
//!
//! Nothing here reads a repository or asks a model. Which parts a spec has, and which of them are
//! still open, is all this decides.

use std::fmt::{self, Display};

/// Who settled a part of a spec.
///
/// The three are told apart because a reader is owed a different thing by each: what they said
/// themselves needs no showing, what was drawn from the repository needs showing before it is
/// taken, and what nobody settled needs answering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The author said it. It is theirs, and is only checked to be real.
    Given,
    /// Drawn from the repository on the author's behalf, and not yet seen by them.
    Inferred,
    /// Nobody has settled it.
    Open,
}

impl Display for Settled {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(match self {
            Settled::Given => "given",
            Settled::Inferred => "inferred",
            Settled::Open => "open",
        })
    }
}

/// One part of a spec, and how it came to say what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// What the part says, where anything does.
    pub said: Option<String>,
    pub settled: Settled,
    /// What it was drawn from, for a reader deciding whether to take it.
    ///
    /// A path on its own tells a reader nothing. What tells them whether to take it is where it
    /// came from -- the file they have open, the line the count is doubled on -- so an inference
    /// carries that with it or is not worth showing.
    pub drawn_from: Option<String>,
    /// The others the repository allows, for a reader who does not take this one.
    ///
    /// Also what says an inference was uncertain. A model asked how sure it is answers with a
    /// number nobody can check; a repository that allows three places is uncertain in a way that
    /// can be counted, so this is read for it rather than a figure the model reports.
    pub others: Vec<String>,
    /// What to ask a person about it, in the words the author wrote in.
    ///
    /// Written by whoever could not settle the part, because they are the one who knows what is
    /// missing and in which language to say it. A surface that wrote the question itself would
    /// have to know both, and would ask it in the same words whatever the author had typed.
    pub asks: Option<String>,
}

impl Part {
    /// A part the author wrote themselves.
    pub fn given(said: &str) -> Self {
        Part {
            said: Some(said.to_owned()),
            settled: Settled::Given,
            drawn_from: None,
            others: Vec::new(),
            asks: None,
        }
    }

    /// A part drawn from the repository, with what it was drawn from.
    pub fn inferred(said: &str, drawn_from: &str) -> Self {
        Part {
            said: Some(said.to_owned()),
            settled: Settled::Inferred,
            drawn_from: Some(drawn_from.to_owned()),
            others: Vec::new(),
            asks: None,
        }
    }

    /// A part nobody has settled.
    pub fn open() -> Self {
        Part {
            said: None,
            settled: Settled::Open,
            drawn_from: None,
            others: Vec::new(),
            asks: None,
        }
    }

    /// Whether anybody has settled it.
    ///
    /// A part carrying no text is open whatever it was marked, so that a model answering with an
    /// empty string cannot leave a decision behind that nothing counts.
    pub fn is_open(&self) -> bool {
        self.settled == Settled::Open
            || self
                .said
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
    }
}

/// Which part of a spec, for naming one in a question or a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    Goal,
    Place,
    DoneWhen,
    OnFailure,
    Why,
    Scope,
}

impl Named {
    /// Every part, in the order a spec is read in.
    pub const ALL: [Named; 6] = [
        Named::Goal,
        Named::Place,
        Named::DoneWhen,
        Named::OnFailure,
        Named::Why,
        Named::Scope,
    ];

    /// The same, written the way a name is written rather than the way a phrase is.
    ///
    /// `on failure` is two words, and a document naming it as a heading or a field writes it as
    /// one. Both are read, so a spec kept in a file is read whichever way its author wrote it.
    pub fn written(self) -> &'static str {
        match self {
            Named::OnFailure => "on_failure",
            Named::DoneWhen => "done_when",
            other => other.label(),
        }
    }

    /// What the part is called, in the words the spec is written in.
    pub fn label(self) -> &'static str {
        match self {
            Named::Goal => "goal",
            Named::Place => "place",
            Named::DoneWhen => "done when",
            Named::OnFailure => "on failure",
            Named::Why => "why",
            Named::Scope => "scope",
        }
    }

    /// What is left undecided while this part is open, for telling an author what an agent would
    /// otherwise settle for them.
    pub fn left_to_decide(self) -> &'static str {
        match self {
            Named::Goal => "what the work is",
            Named::Place => "where the work is",
            Named::DoneWhen => "whether it is done",
            Named::OnFailure => "what to do when it fails",
            Named::Why => "what the work is for",
            Named::Scope => "how far to reach",
        }
    }
}

impl Display for Named {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.label())
    }
}

/// What a run is given to work from.
///
/// Six parts, each of which somebody has to have settled before a run is given it. `why` is the
/// one that only warns: it is what a reviewer reads afterwards rather than what a run needs to
/// start, and refusing for want of it turns back work that could have gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub goal: Part,
    pub place: Part,
    /// What has to be true, and runnable, for the work to be finished.
    pub done_when: Part,
    pub on_failure: Part,
    pub why: Part,
    pub scope: Part,
}

impl Spec {
    /// A spec with nothing settled.
    pub fn open() -> Self {
        Spec {
            goal: Part::open(),
            place: Part::open(),
            done_when: Part::open(),
            on_failure: Part::open(),
            why: Part::open(),
            scope: Part::open(),
        }
    }

    /// One part by name.
    pub fn part(&self, named: Named) -> &Part {
        match named {
            Named::Goal => &self.goal,
            Named::Place => &self.place,
            Named::DoneWhen => &self.done_when,
            Named::OnFailure => &self.on_failure,
            Named::Why => &self.why,
            Named::Scope => &self.scope,
        }
    }

    /// The same, to write into.
    pub fn part_mut(&mut self, named: Named) -> &mut Part {
        match named {
            Named::Goal => &mut self.goal,
            Named::Place => &mut self.place,
            Named::DoneWhen => &mut self.done_when,
            Named::OnFailure => &mut self.on_failure,
            Named::Why => &mut self.why,
            Named::Scope => &mut self.scope,
        }
    }

    /// Every part with its name, in reading order.
    pub fn parts(&self) -> impl Iterator<Item = (Named, &Part)> {
        Named::ALL
            .into_iter()
            .map(|named| (named, self.part(named)))
    }

    /// The parts nobody has settled and a run cannot start without.
    ///
    /// `why` is left out: it is read after a run rather than by one, so it warns instead.
    pub fn undecided(&self) -> Vec<Named> {
        self.parts()
            .filter(|(named, part)| *named != Named::Why && part.is_open())
            .map(|(named, _)| named)
            .collect()
    }

    /// Reads a spec out of text that names its parts.
    ///
    /// What a surface hands back after asking is the spec it was shown, so the two travel as one
    /// text and nothing else has to cross. Every part read this way is the author's own: they saw
    /// it and sent it back, whoever first worked it out.
    ///
    /// And what an author wrote themselves, however they wrote it. A specification is a document
    /// before it is an argument -- kept in a file, edited, read by other people -- so the forms a
    /// document is written in are read: a heading, a list, a name in bold, a name followed by a
    /// colon. Nobody should have to strip the markup off a file to hand it over.
    ///
    /// Nothing where the text names no part at all, which is what an ordinary instruction does.
    pub fn read(text: &str) -> Option<Self> {
        let mut spec = Spec::open();
        let mut named_one = false;
        let mut last: Option<Named> = None;

        for line in text.lines() {
            match named(line) {
                Some((named, said)) => {
                    // The marks a document sets a value apart with come off it too.
                    *spec.part_mut(named) = Part::given(said.trim().trim_matches(DECORATES).trim());
                    named_one = true;
                    last = Some(named);
                }
                // A part that runs to more than a line keeps the rest of it.
                None => {
                    let Some(named) = last else { continue };
                    let line = line.trim_end();
                    // A blank line ends what was being said rather than being part of it: a
                    // document puts one between a section and the next.
                    if line.trim().is_empty() {
                        last = None;
                        continue;
                    }
                    let Some(said) = spec.part_mut(named).said.as_mut() else {
                        continue;
                    };
                    if !said.is_empty() {
                        said.push('\n');
                    }
                    said.push_str(line);
                }
            }
        }

        named_one.then_some(spec)
    }

    /// The spec as a run is given it: one part per line, and the empty ones left out.
    ///
    /// What is stored and what the agent reads are the same text, so that what a run was told is
    /// what a reader of the backlog sees.
    pub fn written(&self) -> String {
        let mut written = String::new();
        for (named, part) in self.parts() {
            let Some(said) = part
                .said
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            written.push_str(named.label());
            written.push_str(": ");
            written.push_str(said);
            written.push('\n');
        }
        written.trim_end().to_owned()
    }
}

/// Which part the line names, and what it says about it.
///
/// The line is read with what a document is written with taken off it first: the marks that open
/// a heading or a list item, and the ones that set a word in bold or in code. What is left has to
/// be the part's name and nothing else, so that a line about a goal is not read as the goal.
fn named(line: &str) -> Option<(Named, &str)> {
    let bare = line
        .trim_start()
        .trim_start_matches(OPENS_A_LINE)
        .trim_start();
    // A name and what it says on one line, or a name on a line of its own with what it says
    // under it, which is how a heading names a section.
    let (label, said) = bare.split_once(':').unwrap_or((bare, ""));
    let label = label
        .trim()
        .trim_matches(DECORATES)
        .trim()
        .to_ascii_lowercase();
    let named = Named::ALL
        .into_iter()
        .find(|named| named.label() == label || named.written() == label)?;
    Some((named, said))
}

/// What opens a line in a document without being part of what the line says.
///
/// A heading, a list item, a quotation, and the numbers a list is sometimes written with.
const OPENS_A_LINE: [char; 8] = ['#', '-', '*', '>', '+', ' ', '\t', '.'];

/// What sets a word apart in a document without being part of the word.
const DECORATES: [char; 3] = ['*', '_', '`'];

#[cfg(test)]
mod tests;
