//! What a run is given to work from, in the parts an unattended run needs each of.
//!
//! An unattended run cannot stop to ask, so every decision the spec leaves open is one the agent
//! makes on its own. Naming the parts is what makes those decisions countable: a part nobody has
//! settled is a decision still to be made, and a spec with none left is one a run can be given.
//!
//! Nothing here reads a repository or asks a model. Which parts a spec has, and which of them are
//! still open, is all this decides.

// Nothing reads a spec yet, and nothing outside the domain names one. The gate is rebuilt on
// this over the commits that follow; this comes off, and `core::domain` re-exports it, with the
// commit that has the service read one.
#![allow(dead_code)]

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
}

impl Part {
    /// A part the author wrote themselves.
    pub fn given(said: &str) -> Self {
        Part {
            said: Some(said.to_owned()),
            settled: Settled::Given,
            drawn_from: None,
            others: Vec::new(),
        }
    }

    /// A part drawn from the repository, with what it was drawn from.
    pub fn inferred(said: &str, drawn_from: &str) -> Self {
        Part {
            said: Some(said.to_owned()),
            settled: Settled::Inferred,
            drawn_from: Some(drawn_from.to_owned()),
            others: Vec::new(),
        }
    }

    /// A part nobody has settled.
    pub fn open() -> Self {
        Part {
            said: None,
            settled: Settled::Open,
            drawn_from: None,
            others: Vec::new(),
        }
    }

    /// The same, with the others the repository allows beside it.
    pub fn beside(mut self, others: &[String]) -> Self {
        self.others = others.to_vec();
        self
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

    /// Whether the author still has to see it before a run is given it.
    pub fn wants_seeing(&self) -> bool {
        self.settled == Settled::Inferred && !self.is_open()
    }
}

/// Which part of a spec, for naming one in a question or a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    Goal,
    Place,
    Success,
    OnFailure,
    Why,
    Scope,
}

impl Named {
    /// Every part, in the order a spec is read in.
    pub const ALL: [Named; 6] = [
        Named::Goal,
        Named::Place,
        Named::Success,
        Named::OnFailure,
        Named::Why,
        Named::Scope,
    ];

    /// What the part is called, in the words the spec is written in.
    pub fn label(self) -> &'static str {
        match self {
            Named::Goal => "goal",
            Named::Place => "place",
            Named::Success => "success",
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
            Named::Success => "whether it is done",
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
    pub success: Part,
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
            success: Part::open(),
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
            Named::Success => &self.success,
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
            Named::Success => &mut self.success,
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

    /// The parts the author has not seen yet.
    pub fn unseen(&self) -> Vec<Named> {
        self.parts()
            .filter(|(_, part)| part.wants_seeing())
            .map(|(named, _)| named)
            .collect()
    }

    /// Takes every inference as seen, which is what accepting the spec does.
    pub fn seen(&mut self) {
        for named in Named::ALL {
            let part = self.part_mut(named);
            if part.wants_seeing() {
                part.settled = Settled::Given;
                part.others.clear();
            }
        }
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

#[cfg(test)]
mod tests;
