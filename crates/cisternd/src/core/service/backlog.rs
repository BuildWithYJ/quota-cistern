//! What `task add`, `task rm`, `task show`, and `backlog` do.

use crate::core::{
    domain::{
        Backlog, Consumption, Disposition, NotABacklog, Observation, Readiness, RemovalRefused,
        Repository, Restored, SessionId, Task, TaskId, TaskState,
    },
    port::{
        inbound::{
            Added, BacklogUseCase, Detail, Listing, Made, Refusal, Registration, Removed, Waiting,
        },
        outbound::{
            BacklogStore, Between, RepositoryRoots, Results, StoredBacklog, StoredConsumption,
            StoredTask,
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
}

impl<'a> BacklogService<'a> {
    pub fn new(
        store: &'a dyn BacklogStore,
        roots: &'a dyn RepositoryRoots,
        results: &'a dyn Results,
    ) -> Self {
        BacklogService {
            store,
            roots,
            results,
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
    fn add(&self, given: Registration<'_>) -> Result<Added, Refusal> {
        if given.title.trim().is_empty() {
            return Err(Refusal::BadValue {
                key: "title".to_owned(),
                value: given.title.to_owned(),
            });
        }

        // Read before the backlog is: an instruction that carries too little to run unattended is
        // turned back whatever the backlog holds, so nothing is written for a run that could not
        // have gone anywhere.
        let readiness = Readiness::read(given.instruction);
        if !given.force && !readiness.ready() {
            return Err(Refusal::NotReady {
                missing: readiness.missing(),
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
                given.instruction.to_owned(),
                given.branch.map(str::to_owned),
                after,
                given.model.map(str::to_owned),
                Repository::new(root),
            );

            Ok(Added {
                id: registered.id().labelled(),
                title: registered.title().to_owned(),
                base_branch: registered.base_branch(),
                after: registered.after().map(|after| after.labelled()),
                model: registered.model().map(str::to_owned),
                repository: registered.repository().to_string(),
                state: registered.state().to_string(),
            })
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
            base_branch: task.base_branch(),
            after: task.after().map(|after| after.labelled()),
            model: task.model().map(str::to_owned),
            repository: task.repository().to_string(),
            branch: task.result_branch(),
            reason: task.reason().map(str::to_owned),
            worktree: task.worktree().map(str::to_owned),
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
            .map(|task| StoredTask {
                id: task.id().to_string(),
                title: task.title().to_owned(),
                instruction: task.instruction().to_owned(),
                branch: task.branch().map(str::to_owned),
                after: task.after().map(|after| after.to_string()),
                model: task.model().map(str::to_owned),
                repository: task.repository().to_string(),
                state: task.state().to_string(),
                session: task.session().map(|id| id.to_string()),
                worktree: task.worktree().map(str::to_owned),
                started_at: task.started_at().map(|at| at.to_string()),
                ended_at: task.ended_at().map(|at| at.to_string()),
                reason: task.reason().map(str::to_owned),
                consumed: kept(task.consumed()),
                unreadable: match task.consumed() {
                    Observation::Unreadable { why } => Some(why.clone()),
                    _ => None,
                },
                disposition: task.disposition().map(|it| it.to_string()),
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
fn kept(consumed: &Observation) -> Option<StoredConsumption> {
    match consumed {
        Observation::Spent(counted) => Some(StoredConsumption {
            input: counted.input.to_string(),
            output: counted.output.to_string(),
            cache_written: counted.cache_written.to_string(),
            cache_read: counted.cache_read.to_string(),
            cost: counted.cost.to_string(),
        }),
        _ => None,
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use crate::core::port::outbound::{StoredBacklog, Unavailable};

    use super::*;

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

    static IN_A_REPOSITORY: Somewhere = Somewhere {
        root: Some("/work/api"),
    };
    static NOWHERE: Somewhere = Somewhere { root: None };

    fn in_a_repository(tasks: &Remembered) -> BacklogService<'_> {
        BacklogService::new(tasks, &IN_A_REPOSITORY, &NO_BRANCH)
    }

    fn outside_one(tasks: &Remembered) -> BacklogService<'_> {
        BacklogService::new(tasks, &NOWHERE, &NO_BRANCH)
    }

    fn registering(title: &str) -> Registration<'_> {
        Registration {
            cwd: "/work/api/src",
            title,
            instruction: "fix parse() in src/util.rs; cargo test util",
            branch: None,
            after: None,
            model: None,
            force: false,
        }
    }

    fn register(tasks: &Remembered, title: &str) -> Added {
        in_a_repository(tasks).add(registering(title)).unwrap()
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
    #[test]
    fn an_instruction_that_carries_too_little_is_refused() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        given.instruction = "make search a bit better";
        let outcome = in_a_repository(&tasks).add(given);

        assert!(matches!(outcome, Err(Refusal::NotReady { .. })));
        assert_eq!(tasks.reads.load(Ordering::Relaxed), 0);
    }

    /// Force registers a task as written, even one the gate would otherwise turn back.
    #[test]
    fn force_registers_what_the_gate_would_turn_back() {
        let tasks = Remembered::default();
        let mut given = registering("first");
        given.instruction = "make search a bit better";
        given.force = true;

        let added = in_a_repository(&tasks).add(given).unwrap();
        assert_eq!(added.state, "Pending");
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
        let second = in_a_repository(&tasks).add(given).unwrap();

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
        in_a_repository(&tasks).add(given).unwrap();

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
        in_a_repository(&tasks).add(given).unwrap();

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
                started_at: None,
                ended_at: None,
                reason: None,
                consumed: None,
                unreadable: None,
                disposition: None,
                id: "1".to_owned(),
                title: "first".to_owned(),
                instruction: "do it".to_owned(),
                branch: None,
                after: Some("2".to_owned()),
                model: None,
                repository: "/work/api".to_owned(),
                state: "Pending".to_owned(),
            },
            crate::core::port::outbound::StoredTask {
                session: None,
                worktree: None,
                started_at: None,
                ended_at: None,
                reason: None,
                consumed: None,
                unreadable: None,
                disposition: None,
                id: "2".to_owned(),
                title: "second".to_owned(),
                instruction: "do it".to_owned(),
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
            started_at: None,
            ended_at: None,
            reason: None,
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
            started_at: None,
            ended_at: None,
            reason: None,
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
