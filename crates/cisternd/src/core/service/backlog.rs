//! What `task add`, `task rm`, `task show`, and `backlog` do.

use crate::core::{
    Added, Detail, Listing, Refusal, Removed, Waiting,
    domain::{Backlog, NotABacklog, RemovalRefused, Repository, Restored, TaskId, TaskState},
    port::{Repository as RepositoryPort, StoredBacklog, StoredTask, Tasks},
};

/// What `task add` was given.
///
/// The arguments arrive together because they are read together, and a
/// parameter list of this length is harder to call correctly than a value with
/// named fields.
pub struct Registration<'a> {
    /// Where the surface was run. The core runs as a daemon, so it cannot read
    /// this from its own process.
    pub cwd: &'a str,
    pub title: &'a str,
    pub instruction: &'a str,
    pub branch: Option<&'a str>,
    pub after: Option<&'a str>,
    pub model: Option<&'a str>,
}

/// Registers a task.
pub fn add(
    tasks: &impl Tasks,
    repository: &impl RepositoryPort,
    given: Registration<'_>,
) -> Result<Added, Refusal> {
    if given.title.trim().is_empty() {
        return Err(Refusal::BadValue {
            key: "title".to_owned(),
            value: given.title.to_owned(),
        });
    }
    let after = given.after.map(identifier).transpose()?;

    // Asked before the backlog is read, since a command run outside a
    // repository is refused whatever the backlog holds.
    let Some(root) = repository.root_of(given.cwd)? else {
        return Err(Refusal::NotARepository {
            at: given.cwd.to_owned(),
        });
    };

    let mut backlog = read(tasks)?;
    if let Some(after) = after
        && backlog.find(after).is_none()
    {
        return Err(Refusal::NoSuchTask {
            id: after.labelled(),
        });
    }

    let id = backlog.add(
        given.title.to_owned(),
        given.instruction.to_owned(),
        given.branch.map(str::to_owned),
        after,
        given.model.map(str::to_owned),
        Repository::new(root),
    );
    tasks.store(&written(&backlog))?;

    let Some(registered) = backlog.find(id) else {
        return Err(Refusal::NoSuchTask { id: id.labelled() });
    };
    Ok(Added {
        id: registered.id().labelled(),
        title: registered.title().to_owned(),
        base_branch: registered.base_branch(),
        after: registered.after().map(|after| after.labelled()),
        model: registered.model().map(str::to_owned),
        repository: registered.repository().to_string(),
        state: registered.state().to_string(),
    })
}

/// Takes a task out of the backlog.
pub fn remove(tasks: &impl Tasks, id: &str) -> Result<Removed, Refusal> {
    let parsed = identifier(id)?;
    let mut backlog = read(tasks)?;

    let removed = backlog.remove(parsed).map_err(|why| match why {
        RemovalRefused::NoSuchTask => Refusal::NoSuchTask {
            id: parsed.labelled(),
        },
        RemovalRefused::NotPending => Refusal::NotPending {
            id: parsed.labelled(),
        },
    })?;
    tasks.store(&written(&backlog))?;

    Ok(Removed {
        id: removed.id().labelled(),
        title: removed.title().to_owned(),
    })
}

/// Reads one task in full.
pub fn show(tasks: &impl Tasks, id: &str) -> Result<Detail, Refusal> {
    let parsed = identifier(id)?;
    let backlog = read(tasks)?;
    let Some(task) = backlog.find(parsed) else {
        return Err(Refusal::NoSuchTask {
            id: parsed.labelled(),
        });
    };

    Ok(Detail {
        id: task.id().labelled(),
        // Filled in once something assigns and runs a task.
        session: None,
        state: task.state().to_string(),
        title: task.title().to_owned(),
        base_branch: task.base_branch(),
        after: task.after().map(|after| after.labelled()),
        model: task.model().map(str::to_owned),
        repository: task.repository().to_string(),
        branch: None,
        reason: None,
        worktree: None,
        disposition: None,
    })
}

/// Lists the tasks waiting to be assigned.
pub fn list(tasks: &impl Tasks) -> Result<Listing, Refusal> {
    let backlog = read(tasks)?;
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

fn identifier(id: &str) -> Result<TaskId, Refusal> {
    TaskId::parse(id).ok_or_else(|| Refusal::BadValue {
        key: "task".to_owned(),
        value: id.to_owned(),
    })
}

/// Reads the store and holds it to the same standard as an argument.
///
/// A backlog file can be edited by hand, so what a store hands back is a claim
/// rather than a fact. Unlike the configuration, nobody is meant to write this
/// file, so a backlog that does not add up is a store this core cannot use
/// rather than something the user typed wrong.
fn read(tasks: &impl Tasks) -> Result<Backlog, Refusal> {
    let stored = tasks.load()?;
    let next_id = stored_number("next_id", &stored.next_id)?;

    let mut restored = Vec::with_capacity(stored.tasks.len());
    for held in stored.tasks {
        restored.push(restored_from(held)?);
    }

    Backlog::restore(next_id, restored).map_err(|e| Refusal::Unavailable {
        reason: unusable(&e),
    })
}

/// Reads one task as a store handed it over.
///
/// The domain is given values it can take, never the text they were kept as,
/// so reading them is this layer's work.
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
            })
            .collect(),
    }
}

fn stored_id(field: &str, value: &str) -> Result<TaskId, Refusal> {
    TaskId::parse(value).ok_or_else(|| unreadable(field, value))
}

fn stored_number(field: &str, value: &str) -> Result<u32, Refusal> {
    value.parse().map_err(|_| unreadable(field, value))
}

/// A value the store holds that this core cannot read.
///
/// Unlike an argument, nobody is meant to write this file, so it fails as a
/// store rather than as something typed wrong.
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
    use std::cell::{Cell, RefCell};

    use crate::core::port::{StoredBacklog, Unavailable};

    use super::*;

    /// A backlog held in memory, so the steps can be checked without a file.
    struct Remembered {
        stored: RefCell<StoredBacklog>,
        /// Makes every read fail, standing in for a store that is there but
        /// cannot be understood.
        broken: bool,
        /// Counts reads, so a test can show that one never happened.
        reads: Cell<usize>,
    }

    impl Default for Remembered {
        fn default() -> Self {
            Remembered {
                stored: RefCell::new(StoredBacklog {
                    next_id: "1".to_owned(),
                    tasks: Vec::new(),
                }),
                broken: false,
                reads: Cell::new(0),
            }
        }
    }

    impl Tasks for Remembered {
        fn load(&self) -> Result<StoredBacklog, Unavailable> {
            self.reads.set(self.reads.get() + 1);
            match self.broken {
                true => Err(Unavailable::new("not valid JSON")),
                false => Ok(self.stored.borrow().clone()),
            }
        }

        fn store(&self, backlog: &StoredBacklog) -> Result<(), Unavailable> {
            *self.stored.borrow_mut() = backlog.clone();
            Ok(())
        }
    }

    /// Answers with one root, or with none for anywhere outside a repository.
    struct Somewhere {
        root: Option<String>,
    }

    impl Default for Somewhere {
        fn default() -> Self {
            Somewhere {
                root: Some("/work/api".to_owned()),
            }
        }
    }

    impl RepositoryPort for Somewhere {
        fn root_of(&self, _from: &str) -> Result<Option<String>, Unavailable> {
            Ok(self.root.clone())
        }
    }

    fn nowhere() -> Somewhere {
        Somewhere { root: None }
    }

    fn registering(title: &str) -> Registration<'_> {
        Registration {
            cwd: "/work/api/src",
            title,
            instruction: "do it",
            branch: None,
            after: None,
            model: None,
        }
    }

    fn register(tasks: &Remembered, title: &str) -> Added {
        add(tasks, &Somewhere::default(), registering(title)).unwrap()
    }

    #[test]
    fn a_registered_task_is_waiting_in_the_backlog() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");

        assert_eq!(added.state, "Pending");
        let listing = list(&tasks).unwrap();
        assert_eq!(listing.items.len(), 1);
        assert_eq!(listing.items[0].id, added.id);
    }

    #[test]
    fn showing_a_task_answers_with_what_it_was_registered_with() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");

        let detail = show(&tasks, &added.id).unwrap();
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
        assert_eq!(show(&tasks, &bare).unwrap().id, added.id);
    }

    #[test]
    fn a_title_of_nothing_is_refused() {
        let tasks = Remembered::default();
        let outcome = add(&tasks, &Somewhere::default(), registering("   "));
        assert!(matches!(outcome, Err(Refusal::BadValue { .. })));
    }

    #[test]
    fn waiting_for_a_task_nobody_registered_is_refused_as_missing() {
        let tasks = Remembered::default();
        let mut given = registering("second");
        given.after = Some("7");
        assert_eq!(
            add(&tasks, &Somewhere::default(), given),
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
        let second = add(&tasks, &Somewhere::default(), given).unwrap();

        assert_eq!(second.after, Some(first.id.clone()));
        assert_eq!(second.base_branch, "cistern/1");
    }

    /// A command run outside a repository is refused whatever the backlog
    /// holds, so the backlog is never read.
    #[test]
    fn registering_outside_a_repository_is_refused_without_reading_the_backlog() {
        let tasks = Remembered::default();
        let outcome = add(&tasks, &nowhere(), registering("refactor X"));

        assert!(matches!(outcome, Err(Refusal::NotARepository { .. })));
        assert_eq!(tasks.reads.get(), 0);
    }

    #[test]
    fn removing_a_task_takes_it_out_of_the_backlog() {
        let tasks = Remembered::default();
        let added = register(&tasks, "refactor X");

        let removed = remove(&tasks, &added.id).unwrap();
        assert_eq!(removed.title, "refactor X");
        assert!(list(&tasks).unwrap().items.is_empty());
    }

    #[test]
    fn removing_a_task_nobody_registered_is_refused_as_missing() {
        let tasks = Remembered::default();
        assert_eq!(
            remove(&tasks, "7"),
            Err(Refusal::NoSuchTask {
                id: "task:7".to_owned()
            })
        );
    }

    #[test]
    fn showing_a_task_nobody_registered_is_refused_as_missing() {
        let tasks = Remembered::default();
        assert!(matches!(show(&tasks, "7"), Err(Refusal::NoSuchTask { .. })));
    }

    #[test]
    fn an_identifier_that_is_not_a_number_is_an_argument_error() {
        let tasks = Remembered::default();
        assert!(matches!(
            show(&tasks, "seven"),
            Err(Refusal::BadValue { .. })
        ));
    }

    #[test]
    fn the_number_of_a_removed_task_is_not_handed_out_again() {
        let tasks = Remembered::default();
        let first = register(&tasks, "first");
        remove(&tasks, &first.id).unwrap();

        let second = register(&tasks, "second");
        assert_ne!(second.id, first.id);
    }

    #[test]
    fn listing_an_empty_backlog_is_not_a_refusal() {
        let tasks = Remembered::default();
        assert_eq!(list(&tasks).unwrap().items, Vec::new());
    }

    #[test]
    fn what_goes_to_a_store_comes_back_the_same() {
        let tasks = Remembered::default();
        let first = register(&tasks, "first");

        let mut given = registering("second");
        given.branch = Some("develop");
        given.after = Some(&first.id);
        given.model = Some("opus");
        add(&tasks, &Somewhere::default(), given).unwrap();

        // A second reader over the same store is what a restarted core is.
        let restarted = Remembered {
            stored: RefCell::new(tasks.stored.borrow().clone()),
            ..Default::default()
        };
        assert_eq!(list(&restarted).unwrap(), list(&tasks).unwrap());
        assert_eq!(show(&restarted, "2").unwrap(), show(&tasks, "2").unwrap());
    }

    /// A value the store holds that cannot be read is a store this core cannot
    /// use, not something the user typed wrong.
    #[test]
    fn a_state_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        register(&tasks, "first");
        tasks.stored.borrow_mut().tasks[0].state = "Sleeping".to_owned();

        assert!(matches!(list(&tasks), Err(Refusal::Unavailable { .. })));
    }

    #[test]
    fn an_identifier_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        register(&tasks, "first");
        tasks.stored.borrow_mut().tasks[0].id = "first".to_owned();

        assert!(matches!(list(&tasks), Err(Refusal::Unavailable { .. })));
    }

    #[test]
    fn a_next_number_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        tasks.stored.borrow_mut().next_id = "soon".to_owned();

        assert!(matches!(list(&tasks), Err(Refusal::Unavailable { .. })));
    }

    #[test]
    fn a_store_that_cannot_be_read_stops_a_write() {
        let tasks = Remembered {
            broken: true,
            ..Default::default()
        };
        assert!(matches!(
            add(&tasks, &Somewhere::default(), registering("refactor X")),
            Err(Refusal::Unavailable { .. })
        ));
    }

    /// A backlog file can be edited by hand, and one that does not add up is a
    /// store this core cannot use.
    #[test]
    fn a_cycle_edited_into_the_store_is_refused_on_reading_it() {
        let tasks = Remembered::default();
        let mut held = tasks.stored.borrow().clone();
        held.tasks = vec![
            crate::core::port::StoredTask {
                id: "1".to_owned(),
                title: "first".to_owned(),
                instruction: "do it".to_owned(),
                branch: None,
                after: Some("2".to_owned()),
                model: None,
                repository: "/work/api".to_owned(),
                state: "Pending".to_owned(),
            },
            crate::core::port::StoredTask {
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
        *tasks.stored.borrow_mut() = held;

        assert!(matches!(list(&tasks), Err(Refusal::Unavailable { .. })));
    }
}
