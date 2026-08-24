//! What `task add`, `task rm`, `task show`, and `backlog` do.

use crate::core::{
    domain::{
        Backlog, Consumption, Disposition, NotABacklog, Observation, RemovalRefused, Repository,
        Restored, SessionId, Task, TaskId, TaskState,
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

#[cfg(test)]
mod tests;
