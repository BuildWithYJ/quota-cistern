//! What `task add`, `task rm`, `task show`, and `backlog` do.

use std::time::Duration;

use crate::core::{
    domain::{
        Backlog, Consumption, Disposition, Grounded, Instruction, Named, NotABacklog, Observation,
        Part, RemovalRefused, Repository, Restored, SessionId, Spec, Task, TaskId, TaskState,
        Undecided, left_to_decide,
    },
    port::{
        inbound::{
            Added, BacklogUseCase, Detail, Left, Listing, Made, Refusal, Registered, Registration,
            Removed, Shown, Unconfirmed, Waiting,
        },
        outbound::{
            BacklogStore, Between, Draft, Drafted, Drafter, Grounding, Ran, RepositoryRoots,
            Results, Room, StoredBacklog, StoredConsumption, StoredTask, Surroundings,
        },
    },
};

/// The commands over the backlog, and what they need from outside.
///
/// It holds the ports these commands use and no others.
/// A command over the configuration cannot reach the backlog store through it.
pub struct BacklogService<'a> {
    store: &'a dyn BacklogStore,
    roots: &'a dyn RepositoryRoots,
    results: &'a dyn Results,
    surroundings: &'a dyn Surroundings,
    grounding: &'a dyn Grounding,
    drafter: &'a dyn Drafter,
}

impl<'a> BacklogService<'a> {
    pub fn new(
        store: &'a dyn BacklogStore,
        roots: &'a dyn RepositoryRoots,
        results: &'a dyn Results,
        surroundings: &'a dyn Surroundings,
        grounding: &'a dyn Grounding,
        drafter: &'a dyn Drafter,
    ) -> Self {
        BacklogService {
            store,
            roots,
            results,
            surroundings,
            grounding,
            drafter,
        }
    }

    /// The spec a run is given, worked out from what the task is being added amid.
    ///
    /// A forced instruction is taken as written and nothing is asked of anybody. Otherwise a
    /// model reads what the author was looking at and writes a spec, every part of it is checked
    /// against the repository, and what is left for the agent to decide is counted. Nothing is
    /// registered while anything is left, and nothing the model worked out is registered before
    /// the author has seen it.
    fn readied(&self, given: &Registration<'_>, root: &str) -> Result<Readied, Refusal> {
        if given.force {
            return Ok(Readied::Given(given.instruction.to_owned()));
        }

        // A surface handing back the spec it showed sends it as the instruction, so the two asks
        // carry one text between them and the second is read like any other.
        if let Some(spec) = Spec::read(given.instruction) {
            let left = self.counted(&spec, given.instruction, root);
            return Ok(match left.is_empty() {
                true => Readied::Given(spec.written()),
                false => Readied::Confirm(Box::new(spec), left),
            });
        }

        // Everything the author was looking at when they wrote the line.
        let changes = self.surroundings.changes(root, CHANGES_SHOWN);
        let lately = self.surroundings.lately(root, COMMITS_SHOWN);
        let branch = self.surroundings.branch(root);
        let tracks = self.surroundings.tracks(root, FILES_TRACKED);
        let ask = || Draft {
            instruction: given.instruction,
            changes: &changes,
            lately: &lately,
            branch: branch.as_deref(),
            tracks: &tracks,
            repository: root,
        };

        let Some(drafted) = self.drafter.draft(ask()) else {
            // Nothing was reached, so nothing was worked out and every part is still open. The
            // author is asked all of it rather than told the model is down: what they can do
            // about it is the same either way.
            let spec = Spec::open();
            let left = spec
                .undecided()
                .into_iter()
                .map(Undecided::Unsettled)
                .collect();
            return Ok(Readied::Confirm(Box::new(spec), left));
        };

        let mut spec = spec_from(&drafted, given.instruction);
        let mut left = self.counted(&spec, given.instruction, root);

        // What the repository would not hold up is the model's to answer for rather than the
        // author's: it named a file that is not there, and it is the one that can look again.
        let amiss: Vec<String> = left
            .iter()
            .filter(|one| one.is_the_models())
            .map(Undecided::left_to_decide)
            .collect();
        if !amiss.is_empty()
            && let Some(again) = self.drafter.draft_again(ask(), &drafted, &amiss)
        {
            let second = spec_from(&again, given.instruction);
            let after = self.counted(&second, given.instruction, root);
            // Taken only where it is better. A second answer that is worse is a second guess.
            if after.len() < left.len() {
                spec = second;
                left = after;
            }
        }

        Ok(Readied::Confirm(Box::new(spec), left))
    }

    /// What the spec leaves for the agent, with the repository asked about the parts that name it.
    fn counted(&self, spec: &Spec, wrote: &str, root: &str) -> Vec<Undecided> {
        left_to_decide(spec, wrote, self.grounded(spec, root))
    }

    /// What the repository says about the parts of a spec that name something in it.
    fn grounded(&self, spec: &Spec, root: &str) -> Grounded {
        let files = spec
            .place
            .said
            .as_deref()
            .and_then(|place| self.grounding.reaches(root, place));
        // Asked before it is run: a command that is not there is the model's mistake, and a
        // command that fails is the task's whole point.
        let ran = spec
            .success
            .said
            .as_deref()
            .filter(|success| self.grounding.runnable(root, success))
            .map(|success| self.grounding.run(root, success, RUNS_WITHIN));
        Grounded {
            files,
            runnable: matches!(ran, Some(Ran::Failed | Ran::Passed)),
            already: ran == Some(Ran::Passed),
        }
    }

    /// What a task left on its branch, for a task whose run has ended.
    ///
    /// Section 2.1 answers with nothing while a task is still waiting, and nothing again for a
    /// branch the user has since deleted.
    fn left(&self, task: &Task) -> (Option<Vec<Made>>, Option<u64>) {
        if !task.state().ended() {
            return (None, None);
        }
        let Some(branch) = task.result_branch() else {
            return (None, None);
        };
        let repository = task.repository().to_string();
        let base = task.base_branch();
        let asked = || Between {
            repository: &repository,
            base: &base,
            branch: &branch,
        };

        (
            self.results.made(asked()).map(|made| {
                made.into_iter()
                    .map(|one| Made {
                        sha: one.sha,
                        subject: one.subject,
                        added: one.added.parse().ok(),
                        removed: one.removed.parse().ok(),
                    })
                    .collect()
            }),
            self.results
                .counts(asked())
                .and_then(|counts| counts.base_ahead.parse().ok()),
        )
    }
}

impl BacklogUseCase for BacklogService<'_> {
    fn add(&self, given: Registration<'_>) -> Result<Registered, Refusal> {
        if given.title.trim().is_empty() {
            return Err(Refusal::BadValue {
                key: "title".to_owned(),
                value: given.title.to_owned(),
            });
        }

        let after = given.after.map(identifier).transpose()?;

        // Asked before the backlog is read.
        // A command run outside a repository is refused whatever the backlog holds.
        let Some(root) = self.roots.root_of(given.cwd)? else {
            return Err(Refusal::NotARepository {
                at: given.cwd.to_owned(),
            });
        };

        // What the run is given to work from. An instruction that carries too little to run
        // unattended is filled in from what the author is in the middle of, and turned back only
        // when the repository cannot settle it. Nothing is written for a run that could not have
        // gone anywhere.
        let instruction = match self.readied(&given, &root)? {
            Readied::Given(instruction) => instruction,
            // Nothing is written. The surface shows the spec, and comes back with it as the
            // instruction it was given, which is then read like any other.
            Readied::Confirm(spec, left) => {
                return Ok(Registered::Unconfirmed(unconfirmed(&spec, &left)));
            }
        };
        // Kept only when the run is given something other than what the author typed, so that its
        // presence is what says a fill happened, and a task written whole carries nothing extra.
        // What the author typed is what arrived, unless a surface has already asked them a
        // question and is saying what they started from.
        let wrote = given.original.unwrap_or(given.instruction);
        let original = (instruction != wrote).then(|| wrote.to_owned());

        change(self.store, |backlog| {
            if let Some(after) = after
                && backlog.find(after).is_none()
            {
                return Err(Refusal::NoSuchTask {
                    id: after.labelled(),
                });
            }

            let registered = backlog.add(
                given.title.to_owned(),
                Instruction {
                    given: instruction,
                    original,
                },
                given.branch.map(str::to_owned),
                after,
                given.model.map(str::to_owned),
                Repository::new(root),
            );

            Ok(Registered::Added(Added {
                id: registered.id().labelled(),
                title: registered.title().to_owned(),
                base_branch: registered.base_branch(),
                after: registered.after().map(|after| after.labelled()),
                model: registered.model().map(str::to_owned),
                repository: registered.repository().to_string(),
                state: registered.state().to_string(),
            }))
        })
    }

    fn remove(&self, id: &str) -> Result<Removed, Refusal> {
        let wanted = identifier(id)?;

        change(self.store, |backlog| {
            let removed = backlog.remove(wanted).map_err(|why| match why {
                RemovalRefused::NoSuchTask => Refusal::NoSuchTask {
                    id: wanted.labelled(),
                },
                RemovalRefused::NotPending => Refusal::NotPending {
                    id: wanted.labelled(),
                },
            })?;

            Ok(Removed {
                id: removed.id().labelled(),
                title: removed.title().to_owned(),
            })
        })
    }

    fn show(&self, id: &str) -> Result<Detail, Refusal> {
        let wanted = identifier(id)?;
        let backlog = read(self.store)?;
        let Some(task) = backlog.find(wanted) else {
            return Err(Refusal::NoSuchTask {
                id: wanted.labelled(),
            });
        };

        let (commits, base_ahead) = self.left(task);
        Ok(Detail {
            id: task.id().labelled(),
            session: task.session().map(|id| id.labelled()),
            state: task.state().to_string(),
            title: task.title().to_owned(),
            instruction: task.instruction().to_owned(),
            original: task.original().map(str::to_owned),
            base_branch: task.base_branch(),
            after: task.after().map(|after| after.labelled()),
            model: task.model().map(str::to_owned),
            repository: task.repository().to_string(),
            branch: task.result_branch(),
            reason: task.reason().map(str::to_owned),
            worktree: task.worktree().map(str::to_owned),
            conversation: task.conversation().map(str::to_owned),
            disposition: task.disposition().map(|it| it.to_string()),
            commits,
            base_ahead,
        })
    }

    fn list(&self) -> Result<Listing, Refusal> {
        let backlog = read(self.store)?;
        Ok(Listing {
            items: backlog
                .pending()
                .into_iter()
                .map(|task| Waiting {
                    id: task.id().labelled(),
                    title: task.title().to_owned(),
                    base_branch: task.base_branch(),
                })
                .collect(),
        })
    }
}

fn identifier(id: &str) -> Result<TaskId, Refusal> {
    TaskId::parse(id).ok_or_else(|| Refusal::BadValue {
        key: "task".to_owned(),
        value: id.to_owned(),
    })
}

/// Reads the store and holds it to the same standard as an argument.
///
/// A backlog file can be edited by hand, so what a store hands back is a claim rather than a fact.
/// Unlike the configuration, nobody is meant to write this file.
/// A backlog that does not add up is a store this core cannot use, not something the user typed wrong.
pub(super) fn read(tasks: &dyn BacklogStore) -> Result<Backlog, Refusal> {
    read_from(tasks.load()?)
}

/// Reads the backlog a store handed over.
/// Held to the standard `read` names.
fn read_from(stored: StoredBacklog) -> Result<Backlog, Refusal> {
    let next_id = stored_number("next_id", &stored.next_id)?;

    let mut restored = Vec::with_capacity(stored.tasks.len());
    for held in stored.tasks {
        restored.push(restored_from(held)?);
    }

    Backlog::restore(next_id, restored).map_err(|e| Refusal::Unavailable {
        reason: unusable(&e),
    })
}

/// Reads the backlog, changes it, and writes it back as one step.
///
/// `service::execution` uses this too, so that one store has one reader.
/// The answer travels out in a value this holds rather than out of the port.
/// A port returning a refusal would have to name a word from the other edge.
pub(super) fn change<T>(
    tasks: &dyn BacklogStore,
    with: impl FnOnce(&mut Backlog) -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    let mut with = Some(with);
    let mut answer = None;

    tasks.update(&mut |stored| {
        // A store that ran the change twice would apply it twice.
        // Taking it leaves the second call nothing to run and nothing to write.
        let Some(with) = with.take() else {
            return false;
        };

        let done = read_from(stored.clone()).and_then(|mut backlog| {
            let got = with(&mut backlog)?;
            Ok((got, backlog))
        });
        match done {
            Ok((got, backlog)) => {
                *stored = written(&backlog);
                answer = Some(Ok(got));
                true
            }
            Err(e) => {
                answer = Some(Err(e));
                false
            }
        }
    })?;

    answer.unwrap_or_else(|| {
        Err(Refusal::Unavailable {
            reason: "the store did not run the change it was given".to_owned(),
        })
    })
}

/// Reads one task as a store handed it over.
///
/// The domain is given values it can take, never the text they were kept as, so reading them is this layer's work.
fn restored_from(held: StoredTask) -> Result<Restored, Refusal> {
    Ok(Restored {
        id: stored_id("id", &held.id)?,
        title: held.title,
        instruction: held.instruction,
        original: held.original,
        branch: held.branch,
        after: held
            .after
            .as_deref()
            .map(|after| stored_id("after", after))
            .transpose()?,
        model: held.model,
        repository: Repository::new(held.repository),
        state: TaskState::parse(&held.state).ok_or_else(|| unreadable("state", &held.state))?,
        session: held
            .session
            .as_deref()
            .map(|id| SessionId::parse(id).ok_or_else(|| unreadable("session", id)))
            .transpose()?,
        worktree: held.worktree,
        conversation: held.conversation,
        started_at: held
            .started_at
            .as_deref()
            .map(|at| stored_count("started_at", at))
            .transpose()?,
        ended_at: held
            .ended_at
            .as_deref()
            .map(|at| stored_count("ended_at", at))
            .transpose()?,
        reason: held.reason,
        attempts: held
            .attempts
            .as_deref()
            .map(|at| stored_number("attempts", at))
            .transpose()?
            .unwrap_or_default(),
        ceiling: held
            .ceiling
            .as_deref()
            .map(|at| stored_count("ceiling", at))
            .transpose()?,
        consumed: observed(held.consumed, held.unreadable)?,
        disposition: held
            .disposition
            .as_deref()
            .map(|it| Disposition::parse(it).ok_or_else(|| unreadable("disposition", it)))
            .transpose()?,
    })
}

/// Hands the backlog to a store as the text a user would have typed.
fn written(backlog: &Backlog) -> StoredBacklog {
    StoredBacklog {
        next_id: backlog.next_id().to_string(),
        tasks: backlog
            .tasks()
            .iter()
            .map(|task| {
                let (spent, unreadable) = kept(task.consumed());
                StoredTask {
                    id: task.id().to_string(),
                    title: task.title().to_owned(),
                    instruction: task.instruction().to_owned(),
                    original: task.original().map(str::to_owned),
                    branch: task.branch().map(str::to_owned),
                    after: task.after().map(|after| after.to_string()),
                    model: task.model().map(str::to_owned),
                    repository: task.repository().to_string(),
                    state: task.state().to_string(),
                    session: task.session().map(|id| id.to_string()),
                    worktree: task.worktree().map(str::to_owned),
                    conversation: task.conversation().map(str::to_owned),
                    started_at: task.started_at().map(|at| at.to_string()),
                    ended_at: task.ended_at().map(|at| at.to_string()),
                    reason: task.reason().map(str::to_owned),
                    attempts: match task.attempts() {
                        0 => None,
                        tried => Some(tried.to_string()),
                    },
                    ceiling: task.ceiling().map(|at| at.to_string()),
                    consumed: spent,
                    unreadable,
                    disposition: task.disposition().map(|it| it.to_string()),
                }
            })
            .collect(),
    }
}

/// Reads what a store held about one task's consumption.
///
/// A store holding both a count and a reason it could not be counted is holding two answers to one question.
/// This core cannot tell which to believe.
fn observed(
    consumed: Option<StoredConsumption>,
    unreadable: Option<String>,
) -> Result<Observation, Refusal> {
    match (consumed, unreadable) {
        (None, None) => Ok(Observation::NotYet),
        (None, Some(why)) => Ok(Observation::Unreadable { why }),
        (Some(counted), None) => Ok(Observation::Spent(Consumption {
            input: stored_count("input", &counted.input)?,
            output: stored_count("output", &counted.output)?,
            cache_written: stored_count("cache_written", &counted.cache_written)?,
            cache_read: stored_count("cache_read", &counted.cache_read)?,
            cost: stored_count("cost", &counted.cost)?,
        })),
        (Some(_), Some(_)) => Err(Refusal::Unavailable {
            reason: "the store says both what a task consumed and that it could not be read"
                .to_owned(),
        }),
    }
}

/// Hands one task's consumption to a store as the text a user would have typed.
/// The other way, for a store that hands its five figures back as the text it kept them as.
///
/// Beside `kept` so that the two directions of one conversion sit together. A figure that does
/// not read as a number leaves nothing, since a count that could not be read is not a count of
/// nothing; who is told that, and how, is the caller's.
pub(super) fn counted(spent: &StoredConsumption) -> Option<Consumption> {
    Some(Consumption {
        input: spent.input.parse().ok()?,
        output: spent.output.parse().ok()?,
        cache_written: spent.cache_written.parse().ok()?,
        cache_read: spent.cache_read.parse().ok()?,
        cost: spent.cost.parse().ok()?,
    })
}

/// Answers with both halves, since a store keeps the figures and the reason in two fields and
/// what ran leaves one of them and never both. Two calls would let a caller write one and
/// forget the other, which is what a task stored as having spent nothing looks like.
pub(super) fn kept(consumed: &Observation) -> (Option<StoredConsumption>, Option<String>) {
    match consumed {
        Observation::Spent(counted) => (
            Some(StoredConsumption {
                input: counted.input.to_string(),
                output: counted.output.to_string(),
                cache_written: counted.cache_written.to_string(),
                cache_read: counted.cache_read.to_string(),
                cost: counted.cost.to_string(),
            }),
            None,
        ),
        Observation::Unreadable { why } => (None, Some(why.clone())),
        Observation::NotYet => (None, None),
    }
}

fn stored_id(field: &str, value: &str) -> Result<TaskId, Refusal> {
    TaskId::parse(value).ok_or_else(|| unreadable(field, value))
}

fn stored_number(field: &str, value: &str) -> Result<u32, Refusal> {
    value.parse().map_err(|_| unreadable(field, value))
}

fn stored_count(field: &str, value: &str) -> Result<u64, Refusal> {
    value.parse().map_err(|_| unreadable(field, value))
}

/// A value the store holds that this core cannot read.
///
/// Unlike an argument, nobody is meant to write this file, so it fails as a store rather than as something typed wrong.
fn unreadable(field: &str, value: &str) -> Refusal {
    Refusal::Unavailable {
        reason: format!("the backlog holds {value} where {field} belongs"),
    }
}

fn unusable(e: &NotABacklog) -> String {
    match e {
        NotABacklog::RepeatedId { id } => format!("the backlog holds task:{id} twice"),
        NotABacklog::NoSuchPredecessor { task, after } => {
            format!("task:{task} waits for task:{after}, which the backlog does not hold")
        }
        NotABacklog::Cycle { task } => format!("the tasks after task:{task} lead back to it"),
    }
}

/// How much of what surrounds a task reaches the model.
///
/// A reader that is a model is paid for by the character, and a working tree can hold a rewrite.
/// Each is bounded twice, by how many lines and by how many characters over all of them, because
/// a count alone bounds nothing: two hundred lines of a lock file is a megabyte, and what adding
/// a task costs would be the repository's to decide rather than this file's.
///
/// Together they hold what surrounds a task under about ten thousand tokens, whatever repository
/// it is added from. First values, chosen to be enough to see what is being done.
const CHANGES_SHOWN: Room = Room {
    most: 200,
    chars: 20_000,
};
const COMMITS_SHOWN: Room = Room {
    most: 10,
    chars: 2_000,
};
const FILES_TRACKED: Room = Room {
    most: 300,
    chars: 12_000,
};

/// How long the success condition is given to say whether it fails.
///
/// A run of the gate is not a run of the work. A test suite that takes longer than this says
/// nothing here, and the author is asked instead of waited on.
const RUNS_WITHIN: Duration = Duration::from_secs(30);

/// What a run is given to work from, once the gate has read it.
enum Readied {
    /// Take it: it was forced, or it is a spec the author has already seen and it leaves nothing.
    Given(String),
    /// The spec as it stands, and what it still leaves for the agent to decide.
    ///
    /// Boxed because a spec is six parts wide and the other arm is one string, and an answer that
    /// is nearly always the small one should not carry the big one's weight.
    Confirm(Box<Spec>, Vec<Undecided>),
}

/// A spec built from what a model proposed, with what the author typed beside it.
///
/// Every part it proposed is an inference: the author has not seen it. What it left out is open.
fn spec_from(drafted: &Drafted, _wrote: &str) -> Spec {
    let mut spec = Spec::open();
    for (named, proposed) in [
        (Named::Goal, &drafted.goal),
        (Named::Place, &drafted.place),
        (Named::Success, &drafted.success),
        (Named::OnFailure, &drafted.on_failure),
        (Named::Why, &drafted.why),
        (Named::Scope, &drafted.scope),
    ] {
        let Some(proposed) = proposed else { continue };
        // A model that could not settle a part still says what to ask about it and what to
        // choose between, so the part stays open and carries the question with it.
        let mut part = match proposed.said.trim().is_empty() {
            true => Part::open(),
            false => Part::inferred(
                &proposed.said,
                proposed.drawn_from.as_deref().unwrap_or("the repository"),
            ),
        };
        part.others = proposed.others.clone();
        part.asks = proposed.asks.clone();
        *spec.part_mut(named) = part;
    }
    spec
}

/// The spec and what it leaves, in the terms the surface showing it is answered in.
fn unconfirmed(spec: &Spec, left: &[Undecided]) -> Unconfirmed {
    Unconfirmed {
        parts: spec
            .parts()
            .map(|(named, part)| Shown {
                part: named.label().to_owned(),
                said: part.said.clone(),
                settled: part.settled.to_string(),
                drawn_from: part.drawn_from.clone(),
                others: part.others.clone(),
                asks: part.asks.clone(),
            })
            .collect(),
        undecided: left
            .iter()
            .map(|one| Left {
                part: one.part().map(|named| named.label().to_owned()),
                decides: one.left_to_decide(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::core::port::outbound::{Proposed, StoredBacklog, Unavailable};

    use super::*;

    /// A spec an author has already seen, which is what most of these tests register with.
    static SEEN: &str = "goal: fix the parser\nplace: src/util.rs\nsuccess: cargo test util\non failure: stop after three attempts\nwhy: it panics\nscope: src/util.rs only";

    /// A backlog held in memory, so the steps can be checked without a file.
    struct Remembered {
        stored: Mutex<StoredBacklog>,
        /// Makes every read fail, standing in for a store that is there but cannot be understood.
        broken: bool,
        /// Counts reads, so a test can show that one never happened.
        reads: AtomicUsize,
    }

    impl Default for Remembered {
        fn default() -> Self {
            Remembered {
                stored: Mutex::new(StoredBacklog {
                    next_id: "1".to_owned(),
                    tasks: Vec::new(),
                }),
                broken: false,
                reads: AtomicUsize::new(0),
            }
        }
    }

    impl BacklogStore for Remembered {
        fn load(&self) -> Result<StoredBacklog, Unavailable> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            match self.broken {
                true => Err(Unavailable::new("not valid JSON")),
                false => Ok(self.stored.lock().unwrap().clone()),
            }
        }

        fn update(
            &self,
            change: &mut dyn FnMut(&mut StoredBacklog) -> bool,
        ) -> Result<(), Unavailable> {
            // The lock is held across the read and the write, which is what the port promises.
            self.reads.fetch_add(1, Ordering::Relaxed);
            if self.broken {
                return Err(Unavailable::new("not valid JSON"));
            }
            let mut held = self.stored.lock().unwrap();
            let mut backlog = held.clone();
            if change(&mut backlog) {
                *held = backlog;
            }
            Ok(())
        }
    }

    /// Answers with one root, or with none for anywhere outside a repository.
    struct Somewhere {
        root: Option<&'static str>,
    }

    impl RepositoryRoots for Somewhere {
        fn root_of(&self, _from: &str) -> Result<Option<String>, Unavailable> {
            Ok(self.root.map(str::to_owned))
        }
    }

    /// A repository holding no branch a task made, which is every task in these tests.
    struct NoBranch;

    impl Results for NoBranch {
        fn made(&self, _between: Between<'_>) -> Option<Vec<crate::core::port::outbound::Commit>> {
            None
        }

        fn counts(&self, _between: Between<'_>) -> Option<crate::core::port::outbound::Counts> {
            None
        }

        fn changes(&self, _between: Between<'_>) -> Option<crate::core::port::outbound::Changes> {
            None
        }

        fn apply(
            &self,
            _between: Between<'_>,
        ) -> Result<
            Vec<crate::core::port::outbound::Touched>,
            crate::core::port::outbound::NotApplied,
        > {
            Err(crate::core::port::outbound::NotApplied::NotThere)
        }

        fn reachable(&self, _repository: &str) -> Result<(), Unavailable> {
            Ok(())
        }
    }

    static NO_BRANCH: NoBranch = NoBranch;

    /// A repository with the given files changed, and the given files held by any word.
    struct Around {
        changed: Vec<String>,
        holds: Vec<String>,
    }

    impl Surroundings for Around {
        fn changes(&self, _repository: &str, _room: Room) -> String {
            self.changed
                .iter()
                .map(|path| format!("--- a/{path}"))
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn lately(&self, _repository: &str, _room: Room) -> String {
            String::new()
        }

        fn branch(&self, _repository: &str) -> Option<String> {
            None
        }

        fn tracks(&self, _repository: &str, _room: Room) -> Vec<String> {
            let mut held = self.changed.clone();
            held.extend(self.holds.iter().cloned());
            held
        }
    }

    static NOTHING_AROUND: Around = Around {
        changed: Vec::new(),
        holds: Vec::new(),
    };

    /// One part as a model would propose it.
    fn proposing(said: &str) -> Option<Proposed> {
        Some(Proposed {
            said: said.to_owned(),
            drawn_from: Some("the diff".to_owned()),
            others: Vec::new(),
            asks: None,
        })
    }

    /// A model that proposes what it was built with, and counts what it was asked.
    struct Proposing {
        first: Drafted,
        /// What it answers when it is asked again, where anything is.
        second: Option<Drafted>,
        asked: AtomicUsize,
    }

    impl Proposing {
        fn of(first: Drafted) -> Self {
            Proposing {
                first,
                second: None,
                asked: AtomicUsize::new(0),
            }
        }

        fn then(mut self, second: Drafted) -> Self {
            self.second = Some(second);
            self
        }
    }

    impl Drafter for Proposing {
        fn draft(&self, _ask: Draft<'_>) -> Option<Drafted> {
            self.asked.fetch_add(1, Ordering::Relaxed);
            Some(self.first.clone())
        }

        fn draft_again(
            &self,
            _ask: Draft<'_>,
            _held: &Drafted,
            _amiss: &[String],
        ) -> Option<Drafted> {
            self.asked.fetch_add(1, Ordering::Relaxed);
            self.second.clone()
        }
    }

    /// A model that cannot be reached, standing in for one that is down.
    struct NoModel;

    impl Drafter for NoModel {
        fn draft(&self, _ask: Draft<'_>) -> Option<Drafted> {
            None
        }

        fn draft_again(
            &self,
            _ask: Draft<'_>,
            _held: &Drafted,
            _amiss: &[String],
        ) -> Option<Drafted> {
            None
        }
    }

    static NO_MODEL: NoModel = NoModel;

    /// A repository that answers about the paths and commands it was built with.
    struct Grounds {
        /// How many files each place reaches. Anything else reaches nothing.
        reaches: &'static [(&'static str, usize)],
        /// The commands it has, and how each of them goes.
        runs: &'static [(&'static str, Ran)],
    }

    impl Grounding for Grounds {
        fn reaches(&self, _repository: &str, place: &str) -> Option<usize> {
            self.reaches
                .iter()
                .find(|(named, _)| *named == place)
                .map(|(_, files)| *files)
        }

        fn runnable(&self, _repository: &str, command: &str) -> bool {
            self.runs.iter().any(|(named, _)| *named == command)
        }

        fn run(&self, _repository: &str, command: &str, _within: Duration) -> Ran {
            self.runs
                .iter()
                .find(|(named, _)| *named == command)
                .map_or(Ran::Unknown, |(_, went)| *went)
        }
    }

    /// A repository holding one file and one test that fails, which is a task worth running.
    static GROUNDS: Grounds = Grounds {
        reaches: &[("src/search.rs", 1), ("src/util.rs", 1), ("src/core", 40)],
        runs: &[
            ("cargo test search", Ran::Failed),
            ("cargo test util", Ran::Failed),
            ("cargo test passing", Ran::Passed),
        ],
    };

    static IN_A_REPOSITORY: Somewhere = Somewhere {
        root: Some("/work/api"),
    };
    static NOWHERE: Somewhere = Somewhere { root: None };

    fn in_a_repository(tasks: &Remembered) -> BacklogService<'_> {
        BacklogService::new(
            tasks,
            &IN_A_REPOSITORY,
            &NO_BRANCH,
            &NOTHING_AROUND,
            &GROUNDS,
            &NO_MODEL,
        )
    }

    fn outside_one(tasks: &Remembered) -> BacklogService<'_> {
        BacklogService::new(
            tasks,
            &NOWHERE,
            &NO_BRANCH,
            &NOTHING_AROUND,
            &GROUNDS,
            &NO_MODEL,
        )
    }

    /// A whole spec, as one an author has already seen comes back.
    fn a_whole_spec() -> String {
        [
            "goal: stop the double count",
            "place: src/search.rs",
            "success: cargo test search",
            "on failure: stop after three attempts",
            "why: the counter is incremented twice",
            "scope: src/search.rs only",
        ]
        .join("\n")
    }

    fn registering(title: &str) -> Registration<'_> {
        Registration {
            cwd: "/work/api/src",
            title,
            instruction: SEEN,
            original: None,
            branch: None,
            after: None,
            model: None,
            force: false,
        }
    }

    fn register(tasks: &Remembered, title: &str) -> Added {
        added(in_a_repository(tasks).add(registering(title)))
    }

    /// The question a registration ended in, for a test that expects one.
    fn unconfirmed(outcome: Result<Registered, Refusal>) -> Unconfirmed {
        match outcome.expect("the registration was refused") {
            Registered::Unconfirmed(asked) => asked,
            Registered::Added(added) => panic!("a task, not a question: {added:?}"),
        }
    }

    /// The task a registration ended in, for a test that expects one.
    ///
    /// A registration can also end in a question, which is not a task and not a refusal, so a
    /// test that wanted one says which it wanted here rather than reading a field off an enum.
    fn added(outcome: Result<Registered, Refusal>) -> Added {
        match outcome.expect("the registration was refused") {
            Registered::Added(added) => added,
            Registered::Unconfirmed(asked) => panic!("a question, not a task: {asked:?}"),
        }
    }

    #[test]
    fn a_registered_task_is_waiting_in_the_backlog() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");

        assert_eq!(added.state, "Pending");
        let listing = in_a_repository(&tasks).list().unwrap();
        assert_eq!(listing.items.len(), 1);
        assert_eq!(listing.items[0].id, added.id);
    }

    #[test]
    fn showing_a_task_answers_with_what_it_was_registered_with() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");

        let detail = in_a_repository(&tasks).show(&added.id).unwrap();
        assert_eq!(detail.title, "refactor X");
        assert_eq!(detail.base_branch, "main");
        assert_eq!(detail.repository, "/work/api");
        assert_eq!(detail.state, "Pending");
        // Nothing runs a task yet, so what a session fills in is still empty.
        assert_eq!(detail.session, None);
        assert_eq!(detail.branch, None);
    }

    #[test]
    fn the_identifier_may_be_written_with_or_without_its_prefix() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");
        let bare = added.id.trim_start_matches("task:").to_owned();
        assert_eq!(in_a_repository(&tasks).show(&bare).unwrap().id, added.id);
    }

    #[test]
    fn a_title_of_nothing_is_refused() {
        let tasks = Remembered::default();
        let outcome = in_a_repository(&tasks).add(registering("   "));
        assert!(matches!(outcome, Err(Refusal::BadValue { .. })));
    }

    /// An instruction with no place to work and no way to check is turned back before the backlog
    /// is read, so nothing is written for a task that could not have run unattended.
    /// A spec the author has already seen and sent back registers, and nothing is asked again.
    #[test]
    fn a_spec_that_leaves_nothing_to_decide_registers() {
        let tasks = Remembered::default();

        let added = added(in_a_repository(&tasks).add(registering("first")));

        assert_eq!(added.state, "Pending");
        assert_eq!(tasks.stored.lock().unwrap().tasks[0].instruction, SEEN);
    }

    /// Nothing is registered while anything is left for the agent to settle by itself.
    #[test]
    fn nothing_is_registered_while_a_decision_is_left() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        // Everything but what to do when it fails, which no model may answer on an author's
        // behalf: it is the decision an agent settles by editing the test.
        let held = SEEN.replace("on failure: stop after three attempts\n", "");
        given.instruction = &held;

        let asked = unconfirmed(in_a_repository(&tasks).add(given));

        assert_eq!(
            asked.undecided,
            vec![Left {
                part: Some("on failure".to_owned()),
                decides: "what to do when it fails".to_owned(),
            }]
        );
        assert!(
            tasks.stored.lock().unwrap().tasks.is_empty(),
            "a task was written while a decision was left"
        );
    }

    /// A reviewer reads it afterwards; a run does not read it at all.
    #[test]
    fn nothing_said_about_why_holds_no_run_back() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        let held = SEEN.replace("why: it panics\n", "");
        given.instruction = &held;

        assert_eq!(added(in_a_repository(&tasks).add(given)).state, "Pending");
    }

    /// `2026/08/26` reads as a path by every rule of shape there is, and holds no file.
    #[test]
    fn a_place_the_repository_does_not_hold_is_a_decision_left() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        let held = SEEN.replace("place: src/util.rs", "place: 2026/08/26");
        given.instruction = &held;

        let asked = unconfirmed(in_a_repository(&tasks).add(given));

        // Nothing settles where the work is, so that is what is left.
        assert_eq!(
            asked.undecided,
            vec![Left {
                part: Some("place".to_owned()),
                decides: "where the work is, since nothing is there".to_owned(),
            }]
        );
    }

    /// A sentence about what done would look like leaves the agent judging its own work.
    #[test]
    fn a_success_condition_nothing_can_run_is_a_decision_left() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        let held = SEEN.replace(
            "success: cargo test util",
            "success: the count should match the documents",
        );
        given.instruction = &held;

        let asked = unconfirmed(in_a_repository(&tasks).add(given));

        assert_eq!(
            asked.undecided,
            vec![Left {
                part: Some("success".to_owned()),
                decides: "whether it is done".to_owned(),
            }]
        );
    }

    /// A command that passes already says either the work is done or that it does not tell.
    #[test]
    fn a_success_condition_that_passes_already_is_a_decision_left() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        let held = SEEN.replace("success: cargo test util", "success: cargo test passing");
        given.instruction = &held;

        let asked = unconfirmed(in_a_repository(&tasks).add(given));

        assert_eq!(
            asked.undecided,
            vec![Left {
                part: Some("success".to_owned()),
                decides: "whether there is anything to do".to_owned(),
            }]
        );
    }

    /// A directory of forty files is a search, and where to stop is a decision.
    #[test]
    fn a_place_that_reaches_too_far_is_a_decision_left() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        let held = SEEN.replace("place: src/util.rs", "place: src/core");
        given.instruction = &held;

        let asked = unconfirmed(in_a_repository(&tasks).add(given));

        assert_eq!(asked.undecided.len(), 1);
        assert!(asked.undecided[0].decides.contains("40 files"));
    }

    /// An ordinary line is handed to a model, and what it works out is shown rather than taken.
    #[test]
    fn a_line_that_is_not_a_spec_is_worked_out_and_shown() {
        let tasks = Remembered::default();
        let model = Proposing::of(Drafted {
            goal: proposing("stop the double count"),
            place: proposing("src/search.rs"),
            success: proposing("cargo test search"),
            on_failure: None,
            why: proposing("the counter is incremented twice"),
            scope: proposing("src/search.rs only"),
        });
        let service = BacklogService::new(
            &tasks,
            &IN_A_REPOSITORY,
            &NO_BRANCH,
            &NOTHING_AROUND,
            &GROUNDS,
            &model,
        );
        let mut given = registering("first");
        given.instruction = "make it stop double-counting";

        let asked = unconfirmed(service.add(given));

        // Every part the model wrote is shown as its own, with what it was drawn from beside it.
        let place = asked
            .parts
            .iter()
            .find(|shown| shown.part == "place")
            .expect("a place is shown");
        assert_eq!(place.said.as_deref(), Some("src/search.rs"));
        assert_eq!(place.settled, "inferred");
        assert_eq!(place.drawn_from.as_deref(), Some("the diff"));
        // The one no model may answer is the one left.
        assert_eq!(asked.undecided.len(), 1);
        assert_eq!(asked.undecided[0].part.as_deref(), Some("on failure"));
        assert!(tasks.stored.lock().unwrap().tasks.is_empty());
        // Asked once. Nothing it wrote failed against the repository.
        assert_eq!(model.asked.load(Ordering::Relaxed), 1);
    }

    /// A part that did not hold up is the model's to answer for, not the author's.
    #[test]
    fn a_part_the_repository_refuses_is_put_back_to_the_model_first() {
        let tasks = Remembered::default();
        let model = Proposing::of(Drafted {
            place: proposing("src/serch.rs"),
            success: proposing("cargo test search"),
            ..Drafted::default()
        })
        .then(Drafted {
            goal: proposing("stop the double count"),
            place: proposing("src/search.rs"),
            success: proposing("cargo test search"),
            why: proposing("the counter is incremented twice"),
            scope: proposing("src/search.rs only"),
            on_failure: None,
        });
        let service = BacklogService::new(
            &tasks,
            &IN_A_REPOSITORY,
            &NO_BRANCH,
            &NOTHING_AROUND,
            &GROUNDS,
            &model,
        );
        let mut given = registering("first");
        given.instruction = "make it stop double-counting";

        let asked = unconfirmed(service.add(given));

        assert_eq!(model.asked.load(Ordering::Relaxed), 2);
        let place = asked
            .parts
            .iter()
            .find(|shown| shown.part == "place")
            .unwrap();
        assert_eq!(place.said.as_deref(), Some("src/search.rs"));
        assert_eq!(asked.undecided.len(), 1);
    }

    /// A second answer that is worse is a second guess, so the first one stands.
    #[test]
    fn a_second_answer_is_taken_only_where_it_settles_more() {
        let tasks = Remembered::default();
        let model = Proposing::of(Drafted {
            goal: proposing("stop the double count"),
            place: proposing("src/serch.rs"),
            success: proposing("cargo test search"),
            ..Drafted::default()
        })
        .then(Drafted::default());
        let service = BacklogService::new(
            &tasks,
            &IN_A_REPOSITORY,
            &NO_BRANCH,
            &NOTHING_AROUND,
            &GROUNDS,
            &model,
        );
        let mut given = registering("first");
        given.instruction = "make it stop double-counting";

        let asked = unconfirmed(service.add(given));

        assert_eq!(model.asked.load(Ordering::Relaxed), 2);
        // The goal the first answer worked out is still there, rather than lost to an empty one.
        let goal = asked
            .parts
            .iter()
            .find(|shown| shown.part == "goal")
            .unwrap();
        assert_eq!(goal.said.as_deref(), Some("stop the double count"));
    }

    /// A model that cannot be reached leaves every part open, and the author is asked all of it.
    #[test]
    fn a_model_that_cannot_be_reached_leaves_everything_to_the_author() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        given.instruction = "make it stop double-counting";

        let asked = unconfirmed(in_a_repository(&tasks).add(given));

        assert_eq!(asked.undecided.len(), 5);
        assert!(asked.parts.iter().all(|shown| shown.settled == "open"));
    }

    /// Forcing takes the instruction as written and asks nobody anything.
    #[test]
    fn forcing_takes_the_instruction_as_written() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        given.instruction = "make it faster";
        given.force = true;

        assert_eq!(added(in_a_repository(&tasks).add(given)).state, "Pending");
        assert_eq!(
            tasks.stored.lock().unwrap().tasks[0].instruction,
            "make it faster"
        );
    }

    /// The run is given the spec; the author's own text is kept beside it.
    #[test]
    fn a_task_worked_out_from_a_line_keeps_what_the_author_wrote() {
        let tasks = Remembered::default();
        let wrote = "make it stop double-counting";
        let whole = a_whole_spec();
        let mut given = registering("first");
        given.instruction = &whole;
        given.original = Some(wrote);

        added(in_a_repository(&tasks).add(given));

        let held = tasks.stored.lock().unwrap();
        assert_eq!(held.tasks[0].original.as_deref(), Some(wrote));
    }

    /// A task the author wrote whole carries no separate original to compare it against.
    #[test]
    fn a_task_written_whole_keeps_no_original() {
        let tasks = Remembered::default();

        register(&tasks, "first");

        assert_eq!(tasks.stored.lock().unwrap().tasks[0].original, None);
    }

    /// An author who answered with what they had already written filled nothing in.
    #[test]
    fn an_original_that_is_the_instruction_is_not_kept_beside_it() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        given.original = Some(SEEN);

        added(in_a_repository(&tasks).add(given));

        assert_eq!(tasks.stored.lock().unwrap().tasks[0].original, None);
    }

    /// Both go out, since without the instruction the original has nothing to sit next to.
    #[test]
    fn showing_a_task_worked_out_from_a_line_surfaces_both() {
        let tasks = Remembered::default();
        let wrote = "make it stop double-counting";
        let whole = a_whole_spec();
        let mut given = registering("first");
        given.instruction = &whole;
        given.original = Some(wrote);
        let registered = added(in_a_repository(&tasks).add(given));

        let shown = in_a_repository(&tasks).show(&registered.id).unwrap();

        assert_eq!(shown.instruction, a_whole_spec());
        assert_eq!(shown.original.as_deref(), Some(wrote));
    }

    #[test]
    fn waiting_for_a_task_nobody_registered_is_refused_as_missing() {
        let tasks = Remembered::default();
        let mut given = registering("second");
        given.after = Some("7");
        assert_eq!(
            in_a_repository(&tasks).add(given),
            Err(Refusal::NoSuchTask {
                id: "task:7".to_owned()
            })
        );
    }

    #[test]
    fn waiting_for_a_task_that_is_there_is_recorded() {
        let tasks = Remembered::default();
        let first = register(&tasks, "first");

        let mut given = registering("second");
        given.after = Some(&first.id);
        let second = added(in_a_repository(&tasks).add(given));

        assert_eq!(second.after, Some(first.id.clone()));
        assert_eq!(second.base_branch, "cistern/1");
    }

    /// A command run outside a repository is refused whatever the backlog holds, so the backlog is never read.
    #[test]
    fn registering_outside_a_repository_is_refused_without_reading_the_backlog() {
        let tasks = Remembered::default();
        let outcome = outside_one(&tasks).add(registering("refactor X"));

        assert!(matches!(outcome, Err(Refusal::NotARepository { .. })));
        assert_eq!(tasks.reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn removing_a_task_takes_it_out_of_the_backlog() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");

        let removed = in_a_repository(&tasks).remove(&added.id).unwrap();
        assert_eq!(removed.title, "refactor X");
        assert!(in_a_repository(&tasks).list().unwrap().items.is_empty());
    }

    /// Removing a predecessor used to leave a file the core could not read.
    /// The next command refused with code 5, and nothing could be listed.
    #[test]
    fn the_backlog_is_still_readable_after_a_predecessor_is_removed() {
        let tasks = Remembered::default();
        let first = register(&tasks, "first");

        let mut given = registering("second");
        given.after = Some(&first.id);
        added(in_a_repository(&tasks).add(given));

        in_a_repository(&tasks).remove(&first.id).unwrap();

        let listing = in_a_repository(&tasks).list().unwrap();
        assert_eq!(listing.items.len(), 1);
        assert_eq!(listing.items[0].base_branch, "main");
    }

    #[test]
    fn removing_a_task_nobody_registered_is_refused_as_missing() {
        let tasks = Remembered::default();
        assert_eq!(
            in_a_repository(&tasks).remove("7"),
            Err(Refusal::NoSuchTask {
                id: "task:7".to_owned()
            })
        );
    }

    #[test]
    fn showing_a_task_nobody_registered_is_refused_as_missing() {
        let tasks = Remembered::default();
        assert!(matches!(
            in_a_repository(&tasks).show("7"),
            Err(Refusal::NoSuchTask { .. })
        ));
    }

    #[test]
    fn an_identifier_that_is_not_a_number_is_an_argument_error() {
        let tasks = Remembered::default();
        assert!(matches!(
            in_a_repository(&tasks).show("seven"),
            Err(Refusal::BadValue { .. })
        ));
    }

    #[test]
    fn the_number_of_a_removed_task_is_not_handed_out_again() {
        let tasks = Remembered::default();
        let first = register(&tasks, "first");
        in_a_repository(&tasks).remove(&first.id).unwrap();

        let second = register(&tasks, "second");
        assert_ne!(second.id, first.id);
    }

    #[test]
    fn listing_an_empty_backlog_is_not_a_refusal() {
        let tasks = Remembered::default();
        assert_eq!(in_a_repository(&tasks).list().unwrap().items, Vec::new());
    }

    #[test]
    fn what_goes_to_a_store_comes_back_the_same() {
        let tasks = Remembered::default();
        let first = register(&tasks, "first");

        let mut given = registering("second");
        given.branch = Some("develop");
        given.after = Some(&first.id);
        given.model = Some("opus");
        added(in_a_repository(&tasks).add(given));

        // A second reader over the same store is what a restarted core is.
        let restarted = Remembered {
            stored: Mutex::new(tasks.stored.lock().unwrap().clone()),
            ..Default::default()
        };
        assert_eq!(
            in_a_repository(&restarted).list().unwrap(),
            in_a_repository(&tasks).list().unwrap()
        );
        assert_eq!(
            in_a_repository(&restarted).show("2").unwrap(),
            in_a_repository(&tasks).show("2").unwrap()
        );
    }

    /// A value the store holds that cannot be read is a store this core cannot use, not something the user typed wrong.
    #[test]
    fn a_state_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        register(&tasks, "first");
        tasks.stored.lock().unwrap().tasks[0].state = "Sleeping".to_owned();

        assert!(matches!(
            in_a_repository(&tasks).list(),
            Err(Refusal::Unavailable { .. })
        ));
    }

    #[test]
    fn an_identifier_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        register(&tasks, "first");
        tasks.stored.lock().unwrap().tasks[0].id = "first".to_owned();

        assert!(matches!(
            in_a_repository(&tasks).list(),
            Err(Refusal::Unavailable { .. })
        ));
    }

    #[test]
    fn a_next_number_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        tasks.stored.lock().unwrap().next_id = "soon".to_owned();

        assert!(matches!(
            in_a_repository(&tasks).list(),
            Err(Refusal::Unavailable { .. })
        ));
    }

    #[test]
    fn a_store_that_cannot_be_read_stops_a_write() {
        let tasks = Remembered {
            broken: true,
            ..Default::default()
        };
        assert!(matches!(
            in_a_repository(&tasks).add(registering("refactor X")),
            Err(Refusal::Unavailable { .. })
        ));
    }

    /// A backlog file can be edited by hand, and one that does not add up is a store this core cannot use.
    #[test]
    fn a_cycle_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        let mut held = tasks.stored.lock().unwrap().clone();
        held.tasks = vec![
            crate::core::port::outbound::StoredTask {
                session: None,
                worktree: None,
                conversation: None,
                started_at: None,
                ended_at: None,
                reason: None,
                attempts: None,
                ceiling: None,
                consumed: None,
                unreadable: None,
                disposition: None,
                id: "1".to_owned(),
                title: "first".to_owned(),
                instruction: "do it".to_owned(),
                original: None,
                branch: None,
                after: Some("2".to_owned()),
                model: None,
                repository: "/work/api".to_owned(),
                state: "Pending".to_owned(),
            },
            crate::core::port::outbound::StoredTask {
                session: None,
                worktree: None,
                conversation: None,
                started_at: None,
                ended_at: None,
                reason: None,
                attempts: None,
                ceiling: None,
                consumed: None,
                unreadable: None,
                disposition: None,
                id: "2".to_owned(),
                title: "second".to_owned(),
                instruction: "do it".to_owned(),
                original: None,
                branch: None,
                after: Some("1".to_owned()),
                model: None,
                repository: "/work/api".to_owned(),
                state: "Pending".to_owned(),
            },
        ];
        *tasks.stored.lock().unwrap() = held;

        assert!(matches!(
            in_a_repository(&tasks).list(),
            Err(Refusal::Unavailable { .. })
        ));
    }

    /// A count and a reason it could not be counted are two answers to one question.
    /// Nothing here can tell which one to believe.
    #[test]
    fn a_task_stored_with_both_a_count_and_a_reason_is_refused() {
        let tasks = Remembered::default();
        let mut held = tasks.stored.lock().unwrap().clone();
        held.tasks = vec![crate::core::port::outbound::StoredTask {
            session: None,
            worktree: None,
            conversation: None,
            started_at: None,
            ended_at: None,
            reason: None,
            attempts: None,
            ceiling: None,
            consumed: Some(StoredConsumption {
                input: "77".to_owned(),
                output: "3377".to_owned(),
                cache_written: "28879".to_owned(),
                cache_read: "263483".to_owned(),
                cost: "92170".to_owned(),
            }),
            unreadable: Some("the answer said nothing about it".to_owned()),
            disposition: None,
            id: "1".to_owned(),
            title: "first".to_owned(),
            instruction: "do it".to_owned(),
            original: None,
            branch: None,
            after: None,
            model: None,
            repository: "/work/api".to_owned(),
            state: "Completed".to_owned(),
        }];
        *tasks.stored.lock().unwrap() = held;

        assert!(matches!(
            in_a_repository(&tasks).list(),
            Err(Refusal::Unavailable { .. })
        ));
    }

    #[test]
    fn a_figure_the_store_holds_that_is_not_a_number_fails_as_a_store() {
        let tasks = Remembered::default();
        let mut held = tasks.stored.lock().unwrap().clone();
        held.tasks = vec![crate::core::port::outbound::StoredTask {
            session: None,
            worktree: None,
            conversation: None,
            started_at: None,
            ended_at: None,
            reason: None,
            attempts: None,
            ceiling: None,
            consumed: Some(StoredConsumption {
                input: "a lot".to_owned(),
                output: "3377".to_owned(),
                cache_written: "28879".to_owned(),
                cache_read: "263483".to_owned(),
                cost: "92170".to_owned(),
            }),
            unreadable: None,
            disposition: None,
            id: "1".to_owned(),
            title: "first".to_owned(),
            instruction: "do it".to_owned(),
            original: None,
            branch: None,
            after: None,
            model: None,
            repository: "/work/api".to_owned(),
            state: "Completed".to_owned(),
        }];
        *tasks.stored.lock().unwrap() = held;

        assert!(matches!(
            in_a_repository(&tasks).list(),
            Err(Refusal::Unavailable { reason }) if reason.contains("a lot")
        ));
    }
}
