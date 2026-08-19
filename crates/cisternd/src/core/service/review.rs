//! What `diff`, `review ls`, `apply`, and `discard` do.

use crate::core::{
    domain::{Backlog, DisposalRefused, Disposition, Task, TaskId},
    port::{
        inbound::{
            Awaiting, Changed, Difference, Dropped, Queue, Refusal, Requeued, ReviewUseCase, Taken,
            Tidied, Tidying,
        },
        outbound::{BacklogStore, Between, Changes, NotApplied, Results, Touched, Worktrees},
    },
};

use super::backlog::{change, read};

/// The commands over a result, and what they need from outside.
pub struct ReviewService<'a> {
    tasks: &'a dyn BacklogStore,
    results: &'a dyn Results,
    worktrees: &'a dyn Worktrees,
}

/// The three names a question about a result takes.
///
/// The base is worked out from what the task was registered with and the branch from its number.
/// Neither is a value anything holds.
/// They are gathered here so that what crosses the port can borrow them.
struct Lies {
    repository: String,
    base: String,
    branch: String,
}

impl Lies {
    fn of(task: &Task, branch: String) -> Self {
        Lies {
            repository: task.repository().to_string(),
            base: task.base_branch(),
            branch,
        }
    }

    fn asked(&self) -> Between<'_> {
        Between {
            repository: &self.repository,
            base: &self.base,
            branch: &self.branch,
        }
    }
}

impl<'a> ReviewService<'a> {
    pub fn new(
        tasks: &'a dyn BacklogStore,
        results: &'a dyn Results,
        worktrees: &'a dyn Worktrees,
    ) -> Self {
        ReviewService {
            tasks,
            results,
            worktrees,
        }
    }

    /// What a task changed, or why it could not be read.
    fn changes(&self, lies: &Lies) -> Result<Changes, Refusal> {
        self.results
            .changes(lies.asked())
            .ok_or_else(|| self.unreadable(lies))
    }

    /// Why a result could not be read.
    ///
    /// A branch that is not there and a repository that is not there are told apart here.
    /// One is a branch the user deleted and the other is a repository they moved.
    /// The two are put right differently.
    fn unreadable(&self, lies: &Lies) -> Refusal {
        match self.results.reachable(&lies.repository) {
            Err(_) => Refusal::NoRepository {
                at: lies.repository.clone(),
            },
            Ok(()) => Refusal::NoResult {
                branch: lies.branch.clone(),
                at: lies.repository.clone(),
            },
        }
    }
}

impl ReviewUseCase for ReviewService<'_> {
    fn diff(&self, id: &str) -> Result<Difference, Refusal> {
        let wanted = identifier(id)?;
        let backlog = read(self.tasks)?;
        let task = held(&backlog, wanted)?;

        // A task that was never assigned has no branch.
        // Section 2.3 gives that the same answer as a branch holding nothing: no changes.
        let Some(branch) = task.result_branch() else {
            return Ok(Difference {
                base: task.base_branch(),
                branch: None,
                files: Vec::new(),
                patch: String::new(),
            });
        };

        let lies = Lies::of(task, branch);
        let changes = self.changes(&lies)?;
        Ok(Difference {
            base: lies.base,
            branch: Some(lies.branch),
            files: counted(changes.files),
            patch: changes.patch,
        })
    }

    fn queue(&self) -> Result<Queue, Refusal> {
        let backlog = read(self.tasks)?;

        let mut items = Vec::new();
        for task in backlog.awaiting_review() {
            // Read rather than insisted on: a deleted branch leaves this task without counts.
            // It still has to be listed.
            let lies = task.result_branch().map(|branch| Lies::of(task, branch));
            let counts = lies
                .as_ref()
                .and_then(|lies| self.results.counts(lies.asked()));

            items.push(Awaiting {
                id: task.id().labelled(),
                title: task.title().to_owned(),
                session: task.session().map(|id| id.labelled()),
                branch: lies.map(|lies| lies.branch),
                state: task.state().to_string(),
                commit_count: counts.as_ref().and_then(|it| count(&it.commits)),
                base_ahead: counts.as_ref().and_then(|it| count(&it.base_ahead)),
            });
        }
        Ok(Queue { items })
    }

    fn apply(&self, id: &str) -> Result<Taken, Refusal> {
        let wanted = identifier(id)?;

        // The whole of it is one change to the store, so a refusal leaves the backlog holding what it held.
        change(self.tasks, |backlog| {
            let task = held(backlog, wanted)?;
            let lies = Lies::of(task, ended(task, wanted)?);

            let applied = self.results.apply(lies.asked()).map_err(|why| match why {
                NotApplied::NotThere => self.unreadable(&lies),
                NotApplied::NotCommitted => Refusal::Uncommitted {
                    at: lies.repository.clone(),
                },
                NotApplied::Nothing => Refusal::NoChange {
                    id: wanted.labelled(),
                },
                NotApplied::Already => Refusal::AlreadyApplied {
                    id: wanted.labelled(),
                },
                NotApplied::Conflicts { why } => Refusal::Conflicts { why },
            })?;

            decided(backlog, wanted, Disposition::Applied)?;
            Ok(Taken {
                task: wanted.labelled(),
                branch: lies.branch,
                files: counted(applied),
            })
        })
    }

    fn discard(&self, id: &str) -> Result<Dropped, Refusal> {
        let wanted = identifier(id)?;

        change(self.tasks, |backlog| {
            let task = held(backlog, wanted)?;
            let branch = ended(task, wanted)?;

            decided(backlog, wanted, Disposition::Discarded)?;
            Ok(Dropped {
                task: wanted.labelled(),
                branch,
            })
        })
    }

    fn retry(&self, id: &str) -> Result<Requeued, Refusal> {
        let wanted = identifier(id)?;

        change(self.tasks, |backlog| {
            let task = held(backlog, wanted)?;
            let branch = ended(task, wanted)?;
            let attempts = task.attempts();

            backlog.try_again(wanted).map_err(|refused| match refused {
                DisposalRefused::NoSuchTask => Refusal::NoSuchTask {
                    id: wanted.labelled(),
                },
                DisposalRefused::NotEnded => Refusal::NotEnded {
                    id: wanted.labelled(),
                },
            })?;
            Ok(Requeued {
                task: wanted.labelled(),
                branch,
                attempts: attempts.to_string(),
            })
        })
    }

    /// Takes away the work areas of tasks that have been disposed of.
    ///
    /// Which ones may go is the backlog's to say. Taking one away runs git, which is slow and
    /// may be refused, so it happens between two holds rather than under one: the list is read,
    /// the work areas are taken away, and only what actually went is written down.
    ///
    /// A work area git would not remove is left where it is and says why. Section 2.4 keeps the
    /// branch either way, so nothing a run committed goes with a work area that does go.
    fn tidy(&self) -> Result<Tidying, Refusal> {
        let backlog = read(self.tasks)?;
        let asked: Vec<(TaskId, String, String)> = backlog
            .tidyable()
            .into_iter()
            .filter_map(|(id, at)| Some((id, backlog.find(id)?.repository().to_string(), at)))
            .collect();

        let tidied: Vec<(TaskId, Tidied)> = asked
            .into_iter()
            .map(|(id, repository, at)| {
                let kept = self
                    .worktrees
                    .remove(&repository, &at)
                    .err()
                    .map(|why| why.reason);
                (
                    id,
                    Tidied {
                        task: id.labelled(),
                        worktree: at,
                        kept,
                    },
                )
            })
            .collect();

        change(self.tasks, |backlog| {
            for (id, _) in tidied.iter().filter(|(_, one)| one.kept.is_none()) {
                backlog.work_area_gone(*id);
            }
            Ok(Tidying {
                items: tidied.iter().map(|(_, one)| one.clone()).collect(),
            })
        })
    }
}

/// The branch a task's result is on, for a task whose run is over.
///
/// Section 2.4 refuses to dispose of a run that has not ended, and every task that has ended carries a branch.
fn ended(task: &Task, wanted: TaskId) -> Result<String, Refusal> {
    task.state()
        .ended()
        .then(|| task.result_branch())
        .flatten()
        .ok_or(Refusal::NotEnded {
            id: wanted.labelled(),
        })
}

fn decided(backlog: &mut Backlog, wanted: TaskId, disposition: Disposition) -> Result<(), Refusal> {
    backlog
        .dispose(wanted, disposition)
        .map_err(|why| match why {
            DisposalRefused::NoSuchTask => Refusal::NoSuchTask {
                id: wanted.labelled(),
            },
            DisposalRefused::NotEnded => Refusal::NotEnded {
                id: wanted.labelled(),
            },
        })
}

fn held(backlog: &Backlog, wanted: TaskId) -> Result<&Task, Refusal> {
    backlog.find(wanted).ok_or(Refusal::NoSuchTask {
        id: wanted.labelled(),
    })
}

fn identifier(id: &str) -> Result<TaskId, Refusal> {
    TaskId::parse(id).ok_or_else(|| Refusal::BadValue {
        key: "task".to_owned(),
        value: id.to_owned(),
    })
}

/// Per-file counts, read from the text they crossed as.
///
/// git counts no lines in a binary file and writes a dash where a number would be.
/// A count that is not a number is absent rather than wrong.
fn counted(files: Vec<Touched>) -> Vec<Changed> {
    files
        .into_iter()
        .map(|file| Changed {
            path: file.path,
            added: count(&file.added),
            removed: count(&file.removed),
        })
        .collect()
}

fn count(written: &str) -> Option<u64> {
    written.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::core::port::outbound::{
        Commit, Counts, Cut, StoredBacklog, StoredTask, Unavailable,
    };

    use super::*;

    /// A backlog held in memory, so the steps can be checked without a file.
    #[derive(Default)]
    struct Remembered {
        stored: Mutex<StoredBacklog>,
    }

    impl BacklogStore for Remembered {
        fn load(&self) -> Result<StoredBacklog, Unavailable> {
            Ok(self.stored.lock().unwrap().clone())
        }

        fn update(
            &self,
            change: &mut dyn FnMut(&mut StoredBacklog) -> bool,
        ) -> Result<(), Unavailable> {
            let mut held = self.stored.lock().unwrap();
            let mut backlog = held.clone();
            if change(&mut backlog) {
                *held = backlog;
            }
            Ok(())
        }
    }

    /// Work areas nothing takes away, for the tests that are not about tidying up.
    struct Untouched;

    impl Worktrees for Untouched {
        fn prepare(&self, _cut: Cut<'_>) -> Result<String, Unavailable> {
            Err(Unavailable::new("nothing prepares a work area here"))
        }

        fn remove(&self, _repository: &str, _at: &str) -> Result<(), Unavailable> {
            Err(Unavailable::new("nothing takes a work area away here"))
        }
    }

    static UNTOUCHED: Untouched = Untouched;

    /// Work areas that remember what they were asked to take away, and refuse the ones named.
    #[derive(Default)]
    struct Taking {
        taken: Mutex<Vec<String>>,
        refusing: Vec<String>,
    }

    impl Worktrees for Taking {
        fn prepare(&self, _cut: Cut<'_>) -> Result<String, Unavailable> {
            Err(Unavailable::new("nothing prepares a work area here"))
        }

        fn remove(&self, _repository: &str, at: &str) -> Result<(), Unavailable> {
            if self.refusing.iter().any(|one| one == at) {
                return Err(Unavailable::new(format!("{at} contains modified files")));
            }
            self.taken.lock().unwrap().push(at.to_owned());
            Ok(())
        }
    }

    fn holding(tasks: Vec<StoredTask>) -> Remembered {
        Remembered {
            stored: Mutex::new(StoredBacklog {
                next_id: "9".to_owned(),
                tasks,
            }),
        }
    }

    fn a_task(id: &str, state: &str) -> StoredTask {
        StoredTask {
            id: id.to_owned(),
            title: "verify webhook signature".to_owned(),
            instruction: "do it".to_owned(),
            branch: None,
            after: None,
            model: None,
            repository: "/work/api".to_owned(),
            state: state.to_owned(),
            session: Some("1".to_owned()),
            worktree: None,
            started_at: None,
            ended_at: None,
            reason: None,
            attempts: None,
            ceiling: None,
            consumed: None,
            unreadable: None,
            disposition: None,
        }
    }

    /// A task that ended, was disposed of, and has a work area to take away.
    fn a_tidied_task(id: &str) -> StoredTask {
        StoredTask {
            state: "Completed".to_owned(),
            worktree: Some(format!("/areas/{id}")),
            disposition: Some("applied".to_owned()),
            ..a_task(id, "Completed")
        }
    }

    /// A repository that answers, standing in for git.
    ///
    /// What it was asked is kept, so a test can show that the base and the branch reached it as the task holds them.
    struct Repository {
        changes: Option<Changes>,
        counts: Option<Counts>,
        applies: Option<NotApplied>,
        reachable: bool,
        asked: Mutex<Vec<String>>,
    }

    impl Default for Repository {
        fn default() -> Self {
            Repository {
                changes: Some(Changes {
                    files: vec![Touched {
                        path: "src/webhook/verify.ts".to_owned(),
                        added: "64".to_owned(),
                        removed: "3".to_owned(),
                    }],
                    patch: "diff --git a b".to_owned(),
                }),
                counts: Some(Counts {
                    commits: "3".to_owned(),
                    base_ahead: "2".to_owned(),
                }),
                applies: None,
                reachable: true,
                asked: Mutex::new(Vec::new()),
            }
        }
    }

    impl Repository {
        fn holding_nothing() -> Self {
            Repository {
                changes: None,
                counts: None,
                ..Default::default()
            }
        }

        fn refusing(why: NotApplied) -> Self {
            Repository {
                applies: Some(why),
                ..Default::default()
            }
        }
    }

    impl Results for Repository {
        fn made(&self, _between: Between<'_>) -> Option<Vec<Commit>> {
            Some(Vec::new())
        }

        fn counts(&self, between: Between<'_>) -> Option<Counts> {
            self.asked.lock().unwrap().push(format!(
                "counts {} {}..{}",
                between.repository, between.base, between.branch
            ));
            self.counts.clone()
        }

        fn changes(&self, between: Between<'_>) -> Option<Changes> {
            self.asked.lock().unwrap().push(format!(
                "changes {} {}..{}",
                between.repository, between.base, between.branch
            ));
            self.changes.clone()
        }

        fn apply(&self, between: Between<'_>) -> Result<Vec<Touched>, NotApplied> {
            self.asked.lock().unwrap().push(format!(
                "apply {} {}..{}",
                between.repository, between.base, between.branch
            ));
            match self.applies.clone() {
                Some(why) => Err(why),
                None => Ok(self.changes.clone().unwrap_or_default().files),
            }
        }

        fn reachable(&self, _repository: &str) -> Result<(), Unavailable> {
            match self.reachable {
                true => Ok(()),
                false => Err(Unavailable::new("no such repository")),
            }
        }
    }

    fn disposition_of(tasks: &Remembered, id: &str) -> Option<String> {
        tasks
            .stored
            .lock()
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.id == id)
            .and_then(|task| task.disposition.clone())
    }

    #[test]
    fn the_queue_holds_what_ended_and_nothing_else() {
        let tasks = holding(vec![
            a_task("1", "Pending"),
            a_task("2", "Running"),
            a_task("3", "Completed"),
            a_task("4", "Interrupted"),
            a_task("5", "Error"),
        ]);
        let git = Repository::default();

        let queue = ReviewService::new(&tasks, &git, &UNTOUCHED)
            .queue()
            .unwrap();
        let ids: Vec<&str> = queue.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["task:3", "task:4", "task:5"]);
        assert_eq!(queue.items[0].commit_count, Some(3));
        assert_eq!(queue.items[0].base_ahead, Some(2));
        assert_eq!(queue.items[0].branch.as_deref(), Some("cistern/3"));
    }

    /// A list says how many commits a branch holds and never shows what is in them.
    /// It must not build the patch to find out.
    #[test]
    fn listing_the_queue_asks_only_for_the_counts() {
        let tasks = holding(vec![a_task("1", "Completed"), a_task("2", "Error")]);
        let git = Repository::default();

        ReviewService::new(&tasks, &git, &UNTOUCHED)
            .queue()
            .unwrap();
        assert_eq!(
            git.asked.lock().unwrap().as_slice(),
            [
                "counts /work/api main..cistern/1",
                "counts /work/api main..cistern/2"
            ]
        );
    }

    /// The repository belongs to whoever is using this, and a task they left no branch for still has to appear.
    #[test]
    fn a_task_whose_branch_cannot_be_read_is_listed_without_counts() {
        let tasks = holding(vec![a_task("1", "Completed")]);
        let git = Repository::holding_nothing();

        let queue = ReviewService::new(&tasks, &git, &UNTOUCHED)
            .queue()
            .unwrap();
        assert_eq!(queue.items.len(), 1);
        assert_eq!(queue.items[0].commit_count, None);
        assert_eq!(queue.items[0].base_ahead, None);
    }

    #[test]
    fn a_task_that_was_never_assigned_changed_nothing() {
        let tasks = holding(vec![a_task("1", "Pending")]);
        let git = Repository::default();

        let difference = ReviewService::new(&tasks, &git, &UNTOUCHED)
            .diff("1")
            .unwrap();
        assert_eq!(difference.branch, None);
        assert_eq!(difference.files, Vec::new());
        assert_eq!(difference.patch, "");
    }

    #[test]
    fn what_a_task_changed_is_asked_for_between_its_base_and_its_branch() {
        let tasks = holding(vec![a_task("1", "Completed")]);
        let git = Repository::default();

        let difference = ReviewService::new(&tasks, &git, &UNTOUCHED)
            .diff("1")
            .unwrap();
        assert_eq!(difference.base, "main");
        assert_eq!(difference.branch.as_deref(), Some("cistern/1"));
        assert_eq!(difference.files[0].added, Some(64));
        assert_eq!(
            git.asked.lock().unwrap().as_slice(),
            ["changes /work/api main..cistern/1"]
        );
    }

    /// A branch the user deleted and a repository they moved are put right differently.
    /// They are not answered the same way.
    #[test]
    fn a_branch_that_is_gone_is_told_apart_from_a_repository_that_is_gone() {
        let tasks = holding(vec![a_task("1", "Completed")]);

        let gone = Repository::holding_nothing();
        assert!(matches!(
            ReviewService::new(&tasks, &gone, &UNTOUCHED).diff("1"),
            Err(Refusal::NoResult { .. })
        ));

        let moved = Repository {
            reachable: false,
            ..Repository::holding_nothing()
        };
        assert!(matches!(
            ReviewService::new(&tasks, &moved, &UNTOUCHED).diff("1"),
            Err(Refusal::NoRepository { .. })
        ));
    }

    #[test]
    fn applying_a_result_records_it_and_takes_the_task_out_of_the_queue() {
        let tasks = holding(vec![a_task("1", "Completed")]);
        let git = Repository::default();
        let review = ReviewService::new(&tasks, &git, &UNTOUCHED);

        let taken = review.apply("1").unwrap();
        assert_eq!(taken.task, "task:1");
        assert_eq!(taken.branch, "cistern/1");
        assert_eq!(taken.files[0].path, "src/webhook/verify.ts");

        assert!(review.queue().unwrap().items.is_empty());
        assert_eq!(disposition_of(&tasks, "1").as_deref(), Some("applied"));
    }

    #[test]
    fn discarding_a_result_leaves_the_branch_and_the_state_where_they_are() {
        let tasks = holding(vec![a_task("1", "Interrupted")]);
        let git = Repository::default();
        let review = ReviewService::new(&tasks, &git, &UNTOUCHED);

        let dropped = review.discard("1").unwrap();
        assert_eq!(dropped.branch, "cistern/1");
        assert!(review.queue().unwrap().items.is_empty());
        assert_eq!(disposition_of(&tasks, "1").as_deref(), Some("discarded"));
        assert_eq!(
            tasks.stored.lock().unwrap().tasks[0].state,
            "Interrupted".to_owned()
        );
        // Nothing was asked of git, since nothing was read or written.
        assert!(git.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn a_discarded_result_can_be_applied_afterwards() {
        let tasks = holding(vec![a_task("1", "Completed")]);
        let git = Repository::default();
        let review = ReviewService::new(&tasks, &git, &UNTOUCHED);

        review.discard("1").unwrap();
        review.apply("1").unwrap();
        assert_eq!(disposition_of(&tasks, "1").as_deref(), Some("applied"));
    }

    #[test]
    fn a_run_that_has_not_ended_cannot_be_disposed_of() {
        let tasks = holding(vec![a_task("1", "Running")]);
        let git = Repository::default();
        let review = ReviewService::new(&tasks, &git, &UNTOUCHED);

        for outcome in [
            format!("{:?}", review.apply("1")),
            format!("{:?}", review.discard("1")),
        ] {
            assert!(outcome.contains("NotEnded"), "{outcome}");
        }
        // Nothing was asked of git for a task that may not be disposed of.
        assert!(git.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn each_way_git_refuses_is_answered_in_its_own_words() {
        let refusals = [
            (NotApplied::NotThere, "NoResult"),
            (NotApplied::NotCommitted, "Uncommitted"),
            (NotApplied::Nothing, "NoChange"),
            (NotApplied::Already, "AlreadyApplied"),
            (
                NotApplied::Conflicts {
                    why: "patch failed".to_owned(),
                },
                "Conflicts",
            ),
        ];

        for (why, named) in refusals {
            let tasks = holding(vec![a_task("1", "Completed")]);
            let git = Repository::refusing(why);
            let refused = ReviewService::new(&tasks, &git, &UNTOUCHED).apply("1");
            assert!(
                format!("{refused:?}").contains(named),
                "{named}: {refused:?}"
            );
            // A refusal leaves the backlog holding what it held.
            assert_eq!(disposition_of(&tasks, "1"), None);
        }
    }

    #[test]
    fn disposing_of_a_task_nobody_registered_says_so() {
        let tasks = holding(Vec::new());
        let git = Repository::default();
        let review = ReviewService::new(&tasks, &git, &UNTOUCHED);

        for outcome in [
            format!("{:?}", review.apply("7")),
            format!("{:?}", review.discard("7")),
            format!("{:?}", review.diff("7")),
        ] {
            assert!(outcome.contains("NoSuchTask"), "{outcome}");
        }
    }

    #[test]
    fn an_identifier_that_is_not_a_number_is_an_argument_error() {
        let tasks = holding(Vec::new());
        let git = Repository::default();
        assert!(matches!(
            ReviewService::new(&tasks, &git, &UNTOUCHED).diff("seven"),
            Err(Refusal::BadValue { .. })
        ));
    }

    /// git counts no lines in a binary file and writes a dash where a number would be.
    #[test]
    fn a_file_git_counted_no_lines_in_carries_no_counts() {
        let tasks = holding(vec![a_task("1", "Completed")]);
        let git = Repository {
            changes: Some(Changes {
                files: vec![Touched {
                    path: "logo.png".to_owned(),
                    added: "-".to_owned(),
                    removed: "-".to_owned(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };

        let difference = ReviewService::new(&tasks, &git, &UNTOUCHED)
            .diff("1")
            .unwrap();
        assert_eq!(difference.files[0].added, None);
        assert_eq!(difference.files[0].removed, None);
    }

    // taking work areas away

    /// Section 2.1 reports the place as null once it is gone.
    #[test]
    fn tidying_up_takes_away_the_work_areas_of_tasks_that_were_disposed_of() {
        let tasks = holding(vec![a_tidied_task("1"), a_tidied_task("2")]);
        let git = Repository::default();
        let areas = Taking::default();

        let tidied = ReviewService::new(&tasks, &git, &areas).tidy().unwrap();

        assert_eq!(*areas.taken.lock().unwrap(), ["/areas/1", "/areas/2"]);
        assert!(tidied.items.iter().all(|one| one.kept.is_none()));
        assert!(
            tasks
                .load()
                .unwrap()
                .tasks
                .iter()
                .all(|task| task.worktree.is_none())
        );
    }

    /// Until a task has been disposed of, its work area is what a person opens to see what was
    /// done, and a task that ended may be put back in the backlog and carry on in it.
    #[test]
    fn a_task_nobody_has_disposed_of_keeps_its_work_area() {
        let tasks = holding(vec![StoredTask {
            worktree: Some("/areas/1".to_owned()),
            ..a_task("1", "Completed")
        }]);
        let git = Repository::default();
        let areas = Taking::default();

        let tidied = ReviewService::new(&tasks, &git, &areas).tidy().unwrap();

        assert!(tidied.items.is_empty());
        assert!(areas.taken.lock().unwrap().is_empty());
    }

    /// A run going is in there.
    #[test]
    fn a_task_still_running_keeps_its_work_area() {
        let tasks = holding(vec![StoredTask {
            worktree: Some("/areas/1".to_owned()),
            disposition: Some("applied".to_owned()),
            ..a_task("1", "Running")
        }]);
        let git = Repository::default();
        let areas = Taking::default();

        assert!(
            ReviewService::new(&tasks, &git, &areas)
                .tidy()
                .unwrap()
                .items
                .is_empty()
        );
    }

    /// What a run left uncommitted is in no branch, so whether a work area is safe to take away
    /// is git's to answer. A refusal is passed on as it was given and the place is still there.
    #[test]
    fn a_work_area_git_will_not_take_away_is_left_and_says_why() {
        let tasks = holding(vec![a_tidied_task("1"), a_tidied_task("2")]);
        let git = Repository::default();
        let areas = Taking {
            refusing: vec!["/areas/1".to_owned()],
            ..Taking::default()
        };

        let tidied = ReviewService::new(&tasks, &git, &areas).tidy().unwrap();

        let left = tidied
            .items
            .iter()
            .find(|one| one.task == "task:1")
            .unwrap();
        assert!(left.kept.as_deref().unwrap().contains("modified files"));
        assert_eq!(*areas.taken.lock().unwrap(), ["/areas/2"]);

        let held = tasks.load().unwrap();
        assert_eq!(held.tasks[0].worktree.as_deref(), Some("/areas/1"));
        assert_eq!(held.tasks[1].worktree, None);
    }
}
