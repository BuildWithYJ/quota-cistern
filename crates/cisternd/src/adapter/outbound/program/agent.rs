//! An agent run as a child process, with nobody to answer it.
//!
//! Which program, what to hand it, and where each figure sits in its answer all come from
//! the definition. What is here is the part that does not change with the vendor: starting
//! the child, ending its process group, and reading its pipes while it writes.

use std::{
    collections::HashMap,
    os::unix::process::CommandExt,
    process::{Command, ExitStatus, Stdio},
    sync::{Mutex, PoisonError},
    thread,
    time::Duration,
};

use nix::{
    sys::signal::{Signal, killpg},
    unistd::Pid,
};
use serde_json::Value;

use crate::core::port::outbound::{
    Agent, Ended, Keeping, Observed, Outcome, Spent, Unavailable, Work,
};

use super::{
    Definition,
    definition::{Marks, Reader},
    path,
};

/// How long a run has to end on its own before it is made to.
///
/// A run commits as it goes, so there is little for it to tidy up.
/// Whoever asked for it to stop is waiting for the command to come back.
const TIDIES_UP_IN: Duration = Duration::from_secs(2);

/// Runs the program a definition describes and waits for it.
pub struct ProgramAgent {
    definition: Definition,
    /// The process group each running task was given, by task.
    ///
    /// A run is ended from the thread that took the command, not the one waiting on the run.
    /// Both have to reach this.
    running: Mutex<HashMap<String, u32>>,
}

impl ProgramAgent {
    pub fn new(definition: Definition) -> Self {
        ProgramAgent {
            definition,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// The arguments with every place filled, and every group that holds an empty one dropped.
    ///
    /// A task that named no model loses `--model` along with the value.
    /// That is why the definition groups a flag with what follows it.
    fn arguments(&self, filling: &[(&str, &str)]) -> Vec<String> {
        super::arguments(&self.definition.args, filling)
    }

    /// What the definition says to fill in, apart from the task's own words.
    fn its_own(&self) -> [(&str, &str); 3] {
        [
            ("goal", self.definition.goal.trim()),
            ("turns", &self.definition.turns),
            ("spend", &self.definition.spend),
        ]
    }

    /// Remembers which group a task's run was given.
    fn went(&self, task: &str, group: u32) {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(task.to_owned(), group);
    }

    /// Forgets it, and says what it was.
    fn gone(&self, task: &str) -> Option<u32> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(task)
    }
}

impl Agent for ProgramAgent {
    fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable> {
        let program = &self.definition.program;
        let mut filling = self.its_own().to_vec();
        filling.push(("instruction", work.instruction));
        filling.push(("model", work.model.unwrap_or_default()));
        // A group holding a place nobody filled is dropped whole, so a run with no
        // conversation to carry on loses the argument that would have named one.
        filling.push(("conversation", work.conversation.unwrap_or_default()));

        let mut running = Command::new(program);
        running
            .current_dir(work.at)
            .args(self.arguments(&filling))
            // A child that inherited this could read what a surface is sending the core.
            // It would wait on it forever if it tried.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A group of its own.
            // The agent starts children and those start their own, and ending only the one this holds leaves the rest.
            .process_group(0);

        let mut child = running
            .spawn()
            .map_err(|e| Unavailable::new(format!("{program}: {e}")))?;
        let group = child.id();
        self.went(work.task, group);

        // Read while the child writes, so one that outwrites a pipe carries on.
        // A watcher also sees the trace before the run has ended.
        let stdout = keeping(
            child.stdout.take(),
            work.trace,
            self.definition.answer.reader,
            self.definition.answer.marks.clone(),
        );
        let stderr = reading(child.stderr.take());

        let status = child
            .wait()
            .map_err(|e| Unavailable::new(format!("{program}: {e}")))?;

        // A grandchild outlives the agent holding the writing end of these pipes.
        // The group goes before the reading is waited on.
        self.gone(work.task);
        end(group);

        let (stdout, stderr) = (said_by(stdout), said_by(stderr));

        // Read once.
        // How the run ended and what it consumed are two questions about one object.
        // One of them failing must not lose the other.
        let answer = serde_json::from_slice::<Value>(&stdout).ok();

        // What the run consumed is read before how it ended, since a run that consumed
        // nothing did not do the work whatever it exited with.
        let observed = self.observed(answer.as_ref());
        let outcome = self.outcome_of(status.success(), answer.as_ref(), &observed);
        Ok(Ended {
            outcome,
            reason: match outcome {
                Outcome::Finished => None,
                _ => Some(self.said(&status, &stderr, answer.as_ref())),
            },
            observed,
            conversation: self.conversation(answer.as_ref()),
            turns: self.turns(answer.as_ref()),
        })
    }

    fn stop(&self, task: &str) {
        let Some(group) = self.gone(task) else {
            return;
        };
        end(group);
    }
}

impl ProgramAgent {
    /// How the run came to an end.
    ///
    /// The name the program gives for stopping is what tells a ceiling from a failure, and
    /// which names mean a ceiling is the definition's to say.
    ///
    /// A run that exited well but counted nothing is not one that finished. A vendor turns
    /// away a prompt it will not take by answering as a success that did no work, and taking
    /// that word leaves a task stored as done on a branch with nothing on it.
    fn outcome_of(&self, finished: bool, answer: Option<&Value>, observed: &Observed) -> Outcome {
        if finished {
            return match observed {
                Observed::Spent(spent) if counted_nothing(spent) => Outcome::Failed,
                _ => Outcome::Finished,
            };
        }
        match self.stopping_word(answer) {
            Some(word) if self.definition.answer.at_ceiling.contains_key(&word) => {
                Outcome::AtCeiling
            }
            _ => Outcome::Failed,
        }
    }

    /// The conversation the run was in, where the definition says where to find one.
    fn conversation(&self, answer: Option<&Value>) -> Option<String> {
        let at = self.definition.answer.conversation.as_deref()?;
        path::text(answer?, at).filter(|named| !named.trim().is_empty())
    }

    /// How many turns the run took, where the definition says where to find the count.
    fn turns(&self, answer: Option<&Value>) -> Option<String> {
        let at = self.definition.answer.turns.as_deref()?;
        path::total(answer?, at).map(|counted| whole(counted).to_string())
    }

    fn stopping_word(&self, answer: Option<&Value>) -> Option<String> {
        path::text(answer?, &self.definition.answer.outcome)
    }

    /// What the program said about a run that failed.
    ///
    /// A run cut off at a ceiling says nothing on standard error.
    /// Handing back the whole of its answer would put an object where a sentence belongs.
    fn said(&self, status: &ExitStatus, stderr: &[u8], answer: Option<&Value>) -> String {
        let complained = String::from_utf8_lossy(stderr);
        let complained = complained.trim();
        if !complained.is_empty() {
            return complained.to_owned();
        }

        if let Some(answer) = answer {
            if let Some(said) = path::text(answer, &self.definition.answer.said)
                && !said.trim().is_empty()
            {
                return said.trim().to_owned();
            }
            if let Some(word) = self.stopping_word(Some(answer)) {
                return self.why_for(&word);
            }
        }
        format!("the agent {status} and said nothing")
    }

    /// A sentence for how the program says it stopped.
    ///
    /// A run cut off at a ceiling answers with no text of its own, so the definition carries
    /// a sentence for each word it may stop with.
    fn why_for(&self, word: &str) -> String {
        match self.definition.answer.at_ceiling.get(word) {
            Some(sentence) => super::fill(sentence, &self.its_own()),
            None => format!("the agent stopped with {word}"),
        }
    }

    /// What the program said it consumed.
    ///
    /// The answer is read whole before any figure is, so that one this cannot read leaves
    /// the sentence about how the run ended where it was.
    fn observed(&self, answer: Option<&Value>) -> Observed {
        let Some(answer) = answer else {
            return unreadable("the agent answered with nothing this could read");
        };
        let held = &self.definition.answer;

        let Some(cost) = path::total(answer, &held.cost) else {
            return unreadable("the agent's answer put no figure on what it consumed");
        };
        let counted = [
            &held.input,
            &held.output,
            &held.cache_written,
            &held.cache_read,
        ]
        .map(|at| path::total(answer, at));
        if counted.iter().any(Option::is_none) {
            return unreadable("what the agent counted is not in the shape this reads");
        }

        let whole = |of: Option<f64>| of.unwrap_or_default().round().max(0.0) as u64;
        Observed::Spent(Spent {
            input: whole(counted[0]).to_string(),
            output: whole(counted[1]).to_string(),
            cache_written: whole(counted[2]).to_string(),
            cache_read: whole(counted[3]).to_string(),
            cost: whole(Some(cost * held.cost_scale)).to_string(),
        })
    }
}

/// Whether the run counted nothing at all.
///
/// Not a run that was cheap. A call that left no message still reads what it was given, so a
/// run that reached the vendor counted something under one of these. All of them at nothing
/// is a run that never reached it.
///
/// A figure this cannot read is not a figure of nothing, and leaves the run counted as having
/// done something rather than as having done none.
fn counted_nothing(spent: &Spent) -> bool {
    [
        &spent.input,
        &spent.output,
        &spent.cache_written,
        &spent.cache_read,
    ]
    .iter()
    .all(|counted| counted.parse::<u64>().is_ok_and(|figure| figure == 0))
}

/// A figure the vendor wrote as a number, as a count.
fn whole(of: f64) -> u64 {
    of.round().max(0.0) as u64
}

fn unreadable(why: &str) -> Observed {
    Observed::Unreadable {
        why: why.to_owned(),
    }
}

/// Ends a process group, and makes sure of it.
///
/// The first signal is one a process may act on, and a group with nothing left is done there.
/// Whatever survives it is not going to act on it, and whoever asked is waiting.
fn end(group: u32) {
    signal(Signal::SIGTERM, group);
    if !still_there(group) {
        return;
    }
    thread::sleep(TIDIES_UP_IN);
    signal(Signal::SIGKILL, group);
}

/// Whether anything in the group is still running.
fn still_there(group: u32) -> bool {
    signal(None, group)
}

/// Signals a whole process group, and says whether there was one.
///
/// Not through the `kill` program.
/// The two systems this runs on disagree about what a negative number on its command line means.
/// On one of them nothing was signalled and nothing said so.
/// No signal only asks.
fn signal(with: impl Into<Option<Signal>>, group: u32) -> bool {
    let Ok(group) = i32::try_from(group) else {
        return false;
    };
    killpg(Pid::from_raw(group), with).is_ok()
}

/// Reads what the run says on a thread of its own, keeping each line as it arrives.
///
/// Every line goes to the trace whatever the shape is. What is handed back is the part the
/// answer sits in, which is the one thing the shape decides.
fn keeping<R: std::io::Read + Send + 'static>(
    held: Option<R>,
    mut into: Keeping,
    reader: Reader,
    marks: Option<Marks>,
) -> thread::JoinHandle<Vec<u8>> {
    use std::io::{BufRead, BufReader};

    thread::spawn(move || {
        let Some(held) = held else {
            return Vec::new();
        };
        let mut answer = String::new();

        for line in BufReader::new(held).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            into(&line);
            match reader {
                Reader::LastJsonLine if marked(&line, marks.as_ref()) => answer = line,
                Reader::LastJsonLine => {}
            }
        }
        answer.into_bytes()
    })
}

/// Whether this line is the one the answer is on.
///
/// A definition that does not say takes every line, so the last one written is the answer.
/// One that says takes only the lines saying so, and a line this cannot read says nothing.
fn marked(line: &str, marks: Option<&Marks>) -> bool {
    let Some(marks) = marks else {
        return true;
    };
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|one| path::text(&one, &marks.at))
        .is_some_and(|says| says == marks.is)
}

/// Reads one of the run's pipes on a thread of its own.
fn reading<R: std::io::Read + Send + 'static>(held: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut said = Vec::new();
        if let Some(mut held) = held {
            let _ = held.read_to_end(&mut said);
        }
        said
    })
}

/// What that pipe carried, once nobody is left to write to it.
fn said_by(reading: thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    reading.join().unwrap_or_default()
}

#[cfg(test)]
mod tests;
