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
    ) -> Result<Vec<crate::core::port::outbound::Touched>, crate::core::port::outbound::NotApplied>
    {
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
        instruction: "do it",
        branch: None,
        after: None,
        model: None,
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
