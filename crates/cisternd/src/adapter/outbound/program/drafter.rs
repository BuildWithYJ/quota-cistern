//! The vendor asked to work out what a loose instruction meant, by running the program a
//! definition describes.
//!
//! An ask, not a run: the model is given the instruction and everything the author was looking at
//! when they wrote it, and is asked to write the parts of a spec. Every part it writes is checked
//! against the repository before a run is given anything, so this reads and proposes and nothing
//! else. A cheaper model answers first; a stronger one is asked only when the cheaper found no
//! place at all.
//!
//! Which program, which two models, what to hand them, and the words each part is read by are all
//! in the definition. What stays here is the part that is the same whoever the vendor is.

use std::{
    process::{Command, Stdio},
    time::Instant,
};

use crate::core::port::outbound::{Draft, Drafted, Drafter, Proposed};

use super::Definition;

/// Proposes what a spec should say, by asking the vendor.
pub struct ProgramDrafter {
    definition: Definition,
}

impl ProgramDrafter {
    pub fn new(definition: Definition) -> Self {
        ProgramDrafter { definition }
    }

    /// What the model is asked, as the definition writes it.
    fn asking(&self, ask: &Draft<'_>) -> String {
        let drafting = &self.definition.drafter;
        // Said rather than left blank, so that an empty heading is not read as an omission.
        let changes = said(ask.changes, "(nothing is uncommitted)");
        let lately = said(ask.lately, "(nothing has been committed)");
        let files = said(&ask.tracks.join("\n"), "(unavailable)");
        let branch = ask.branch.unwrap_or("(none)");

        // One pass, as the agent's arguments are: an instruction holding the text `{files}` must
        // not come out carrying the listing.
        super::fill(
            drafting.prompt.trim(),
            &[
                ("instruction", ask.instruction),
                ("changes", &changes),
                ("lately", &lately),
                ("branch", branch),
                ("files", &files),
                ("goal", &drafting.goal),
                ("place", &drafting.place),
                ("done_when", &drafting.done_when),
                ("on_failure", &drafting.on_failure),
                ("why", &drafting.why),
                ("scope", &drafting.scope),
                ("drawn_from", &drafting.drawn_from),
                ("others", &drafting.others),
                ("asks", &drafting.asks),
            ],
        )
    }

    /// Runs the program once and reads what it proposed, or nothing when it could not be reached.
    ///
    /// What came back is written to the daemon's log as it stands. A part that comes back empty
    /// is a question the author is asked, and without the answer beside it there is no telling a
    /// model that could not work something out from one that was asked the wrong thing. The
    /// prompt is not written: it is the definition and the repository, both of which are there
    /// to read, and it is a hundred times longer than what came back.
    fn asked(&self, model: &str, repository: &str, prompt: &str) -> Option<Drafted> {
        let drafting = &self.definition.drafter;
        let since = Instant::now();
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

        let took = since.elapsed().as_secs();
        if !done.status.success() {
            eprintln!(
                "cisternd: {model} did not answer after {took}s: {}",
                String::from_utf8_lossy(&done.stderr).trim()
            );
            return None;
        }
        let answer = String::from_utf8_lossy(&done.stdout);
        eprintln!(
            "cisternd: {model} answered in {took}s with:\n{}",
            answer.trim()
        );
        Some(self.read(&answer))
    }

    /// Reads a spec out of the model's lines, by the words the definition names.
    fn read(&self, answer: &str) -> Drafted {
        let drafting = &self.definition.drafter;
        let part = |key: &str| self.part(answer, key);
        Drafted {
            goal: part(&drafting.goal),
            place: part(&drafting.place),
            done_when: part(&drafting.done_when),
            on_failure: part(&drafting.on_failure),
            why: part(&drafting.why),
            scope: part(&drafting.scope),
        }
    }

    /// One part: what it should say, where that came from, what else it could be, and what to
    /// ask about it where nothing settled it.
    ///
    /// A part with nothing to say is still a part where the model wrote a question about it. That
    /// is the whole of what a person is shown for one nobody settled, so it does not come back as
    /// though the model had said nothing at all.
    fn part(&self, answer: &str, key: &str) -> Option<Proposed> {
        let drafting = &self.definition.drafter;
        let said = field(answer, key);
        let asks = field(answer, &format!("{key}{}", drafting.asks));
        let others: Vec<String> = field(answer, &format!("{key}{}", drafting.others))
            .map(|held| {
                held.split(',')
                    .map(str::trim)
                    .filter(|other| !other.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        (said.is_some() || asks.is_some() || !others.is_empty()).then(|| Proposed {
            said: said.unwrap_or_default(),
            drawn_from: field(answer, &format!("{key}{}", drafting.drawn_from)),
            others,
            asks,
        })
    }
}

impl Drafter for ProgramDrafter {
    fn draft(&self, ask: Draft<'_>) -> Option<Drafted> {
        let drafting = &self.definition.drafter;
        let prompt = self.asking(&ask);

        let cheap = self.asked(&drafting.cheaper, ask.repository, &prompt);
        if cheap
            .as_ref()
            .is_some_and(|drafted| drafted.place.is_some())
        {
            return cheap;
        }
        // The cheaper model worked out no place at all. A stronger one may, and only then is it
        // worth its cost.
        self.asked(&drafting.stronger, ask.repository, &prompt)
            .or(cheap)
    }

    fn draft_again(&self, ask: Draft<'_>, held: &Drafted, amiss: &[String]) -> Option<Drafted> {
        let drafting = &self.definition.drafter;
        // The first ask, what it answered, and what the repository said about it. All three,
        // because a model told only what was wrong writes the rest again from nothing.
        let prompt = format!(
            "{}\n\n{}\n\n{}",
            self.asking(&ask),
            written(held, drafting),
            super::fill(drafting.again.trim(), &[("amiss", &amiss.join("\n"))])
        );
        // Asked of the cheaper model. It is being told which of its answers did not hold up and
        // what the repository holds instead, which is a correction rather than a harder question.
        self.asked(&drafting.cheaper, ask.repository, &prompt)
    }
}

/// What a heading holds, or what to say where it holds nothing.
fn said(held: &str, when_empty: &str) -> String {
    match held.trim().is_empty() {
        true => when_empty.to_owned(),
        false => held.trim().to_owned(),
    }
}

/// A spec written back in the lines it was read from, for asking about it again.
fn written(held: &Drafted, drafting: &super::Drafting) -> String {
    [
        (&drafting.goal, &held.goal),
        (&drafting.place, &held.place),
        (&drafting.done_when, &held.done_when),
        (&drafting.on_failure, &held.on_failure),
        (&drafting.why, &held.why),
        (&drafting.scope, &held.scope),
    ]
    .into_iter()
    .filter_map(|(key, part)| part.as_ref().map(|part| format!("{key}: {}", part.said)))
    .collect::<Vec<_>>()
    .join("\n")
}

/// The value on the line that starts with the key, unless it is empty or "none".
///
/// The key has to be the whole of what stands before the colon, so that `PLACE` is not read off
/// the line `PLACE-FROM` that follows it.
fn field(answer: &str, key: &str) -> Option<String> {
    for line in answer.lines() {
        let (named, rest) = line.trim().split_once(':')?;
        if named.trim() != key {
            continue;
        }
        let value = rest.trim().trim_matches('`').trim();
        if value.is_empty() || value.eq_ignore_ascii_case("none") {
            return None;
        }
        return Some(value.to_owned());
    }
    None
}

#[cfg(test)]
mod tests;
