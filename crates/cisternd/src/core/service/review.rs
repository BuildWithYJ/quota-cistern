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
        self.waits_again(id, Backlog::try_again)
    }

    fn resume(&self, id: &str) -> Result<Requeued, Refusal> {
        self.waits_again(id, Backlog::carries_on)
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

impl ReviewService<'_> {
    /// Puts a task that ended back in the backlog, by the rule the caller names.
    ///
    /// The two callers differ by that rule and by nothing else: what is checked, what is
    /// reported, and what is left alone are the same whether the work is done over or carried
    /// on.
    fn waits_again(
        &self,
        id: &str,
        putting: fn(&mut Backlog, TaskId) -> Result<(), DisposalRefused>,
    ) -> Result<Requeued, Refusal> {
        let wanted = identifier(id)?;

        change(self.tasks, |backlog| {
            let task = held(backlog, wanted)?;
            let branch = ended(task, wanted)?;
            let attempts = task.attempts();

            putting(backlog, wanted).map_err(|refused| match refused {
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
                carries_on: held(backlog, wanted)?.conversation().is_some(),
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
mod tests;
