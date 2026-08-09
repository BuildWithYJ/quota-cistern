//! What `run` does.

use crate::core::{
    domain::{
        Budget, Held, Key, NotASessionSet, NotOpened, SessionState, Sessions, Setting, Span,
        TaskId, TaskState, Usage,
    },
    port::{
        inbound::{Declaration, Declared, ExecutionUseCase, Refusal, Started},
        outbound::{
            Agent, BacklogStore, ConfigurationStore, Cut, SessionStore, StoredSession,
            StoredSessions, Work, Worktrees,
        },
    },
};

use super::backlog;

/// The commands over sessions, and what they need from outside.
///
/// The configuration is here because a share of a plan cannot be declared
/// without one, which is the only thing this reads it for.
pub struct ExecutionService<'a> {
    sessions: &'a dyn SessionStore,
    tasks: &'a dyn BacklogStore,
    configuration: &'a dyn ConfigurationStore,
    worktrees: &'a dyn Worktrees,
    agent: &'a dyn Agent,
}

impl<'a> ExecutionService<'a> {
    pub fn new(
        sessions: &'a dyn SessionStore,
        tasks: &'a dyn BacklogStore,
        configuration: &'a dyn ConfigurationStore,
        worktrees: &'a dyn Worktrees,
        agent: &'a dyn Agent,
    ) -> Self {
        ExecutionService {
            sessions,
            tasks,
            configuration,
            worktrees,
            agent,
        }
    }

    /// Whether a plan is configured, which is what a share is measured against.
    ///
    /// A plan that is there and cannot be read is a store this core cannot use,
    /// the same as any other value a file holds wrongly.
    fn plan_is_configured(&self) -> Result<bool, Refusal> {
        let Some(plan) = self.configuration.load()?.plan else {
            return Ok(false);
        };
        match Setting::parse(Key::Plan, &plan) {
            Some(_) => Ok(true),
            None => Err(Refusal::Unavailable {
                reason: format!("the configuration holds {plan} where plan belongs"),
            }),
        }
    }
}

impl ExecutionUseCase for ExecutionService<'_> {
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal> {
        let usage = Usage::parse(declared.usage).ok_or_else(|| Refusal::BadValue {
            key: "usage".to_owned(),
            value: declared.usage.to_owned(),
        })?;
        let time = Span::parse(declared.time).ok_or_else(|| Refusal::BadValue {
            key: "time".to_owned(),
            value: declared.time.to_owned(),
        })?;

        // Asked before the sessions are read, since a share with no plan is
        // refused whatever else is running.
        if matches!(usage, Usage::Share(_)) && !self.plan_is_configured()? {
            return Err(Refusal::NoPlanConfigured);
        }

        // Asked before a session is opened, so that a run with nothing to do
        // does not leave one behind that has to be stopped again.
        if backlog::read(self.tasks)?.next_to_assign().is_none() {
            return Err(Refusal::NothingToAssign);
        }

        let budget = Budget { usage, time };
        let model = declared.model.map(str::to_owned);

        let opened = change(self.sessions, |sessions| {
            sessions
                .open(budget, model)
                .map_err(|NotOpened::AlreadyRunning { id }| Refusal::AlreadyRunning {
                    id: id.labelled(),
                })
        })?;

        // There is no supervisor yet to decide what and how many, so one task
        // is assigned and the session runs until something stops it.
        let assigned = backlog::change(self.tasks, |tasks| Ok(tasks.assign(opened)))?;

        Ok(Started {
            session: opened.labelled(),
            state: SessionState::Running.to_string(),
            assigned: assigned.iter().map(TaskId::labelled).collect(),
            budget: Declared {
                usage: usage.to_string(),
                time: time.to_string(),
            },
        })
    }

    fn carry_on(&self, task: &str) -> Result<(), Refusal> {
        let id = TaskId::parse(task).ok_or_else(|| Refusal::BadValue {
            key: "task".to_owned(),
            value: task.to_owned(),
        })?;

        let (repository, base, branch, instruction, model) = {
            let tasks = backlog::read(self.tasks)?;
            let held = tasks
                .find(id)
                .ok_or_else(|| Refusal::NoSuchTask { id: id.labelled() })?;
            (
                held.repository().to_string(),
                held.base_branch(),
                held.result_branch().unwrap_or_default(),
                held.instruction().to_owned(),
                held.model().map(str::to_owned),
            )
        };

        let at = match self.worktrees.prepare(Cut {
            repository: &repository,
            base: &base,
            branch: &branch,
            task: &id.to_string(),
        }) {
            Ok(at) => at,
            // A task that could not be given a place to work in has ended, and
            // saying so is what keeps it from being read as still running.
            Err(e) => return self.ended(id, TaskState::Error, Some(e.reason)),
        };
        backlog::change(self.tasks, |tasks| {
            tasks.work_area(id, at.clone());
            Ok(())
        })?;

        let ended = self.agent.work(Work {
            at: &at,
            instruction: &instruction,
            model: model.as_deref(),
        });
        match ended {
            Ok(ended) if ended.done => self.ended(id, TaskState::Completed, None),
            Ok(ended) => self.ended(id, TaskState::Error, ended.reason),
            Err(e) => self.ended(id, TaskState::Error, Some(e.reason)),
        }
    }
}

impl ExecutionService<'_> {
    /// Moves a task to the state it ended in.
    fn ended(&self, id: TaskId, state: TaskState, reason: Option<String>) -> Result<(), Refusal> {
        backlog::change(self.tasks, |tasks| {
            tasks.finish(id, state, reason.clone());
            Ok(())
        })
    }
}

/// Reads the sessions and holds them to the same standard as an argument.
///
/// Nobody is meant to write this file, so a set that does not add up is a store
/// this core cannot use rather than something the user typed wrong. This is
/// what `service::backlog` does for the backlog, and the two stay apart because
/// neither store knows what the other holds.
fn read_from(stored: StoredSessions) -> Result<Sessions, Refusal> {
    let next_id = stored
        .next_id
        .parse()
        .map_err(|_| unreadable("next_id", &stored.next_id))?;

    let mut held = Vec::with_capacity(stored.sessions.len());
    for one in stored.sessions {
        held.push(held_from(one)?);
    }

    Sessions::restore(next_id, held).map_err(|e| Refusal::Unavailable {
        reason: unusable(&e),
    })
}

/// Reads one session as a store handed it over.
fn held_from(one: StoredSession) -> Result<Held, Refusal> {
    use crate::core::domain::{SessionId, SessionState, StoppedReason};

    Ok(Held {
        id: SessionId::parse(&one.id).ok_or_else(|| unreadable("id", &one.id))?,
        state: SessionState::parse(&one.state).ok_or_else(|| unreadable("state", &one.state))?,
        stopped_reason: one
            .stopped_reason
            .as_deref()
            .map(|reason| {
                StoppedReason::parse(reason).ok_or_else(|| unreadable("stopped_reason", reason))
            })
            .transpose()?,
        budget: Budget {
            usage: Usage::parse(&one.usage).ok_or_else(|| unreadable("usage", &one.usage))?,
            time: Span::parse(&one.time).ok_or_else(|| unreadable("time", &one.time))?,
        },
        model: one.model,
    })
}

/// Hands the sessions to a store as the text a user would have typed.
fn written(sessions: &Sessions) -> StoredSessions {
    StoredSessions {
        next_id: sessions.next_id().to_string(),
        sessions: sessions
            .sessions()
            .iter()
            .map(|session| StoredSession {
                id: session.id().to_string(),
                state: session.state().to_string(),
                stopped_reason: session.stopped_reason().map(|why| why.to_string()),
                usage: session.budget().usage.to_string(),
                time: session.budget().time.to_string(),
                model: session.model().map(str::to_owned),
            })
            .collect(),
    }
}

/// Reads the sessions, changes them, and writes them back as one step, for the
/// reason `service::backlog` gives.
fn change<T>(
    store: &dyn SessionStore,
    with: impl FnOnce(&mut Sessions) -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    let mut with = Some(with);
    let mut answer = None;

    store.update(&mut |stored| {
        let Some(with) = with.take() else {
            return false;
        };

        let done = read_from(stored.clone()).and_then(|mut sessions| {
            let got = with(&mut sessions)?;
            Ok((got, sessions))
        });
        match done {
            Ok((got, sessions)) => {
                *stored = written(&sessions);
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

fn unreadable(field: &str, value: &str) -> Refusal {
    Refusal::Unavailable {
        reason: format!("the sessions hold {value} where {field} belongs"),
    }
}

fn unusable(e: &NotASessionSet) -> String {
    match e {
        NotASessionSet::RepeatedId { id } => format!("the sessions hold session:{id} twice"),
        NotASessionSet::TwoRunning { first, second } => {
            format!("session:{first} and session:{second} are both running")
        }
        NotASessionSet::ReasonDoesNotMatchState { id } => {
            format!("session:{id} does not say why it stopped, or says so while running")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::core::{
        domain::SessionId,
        port::outbound::{Ended, StoredBacklog, StoredConfiguration, StoredTask, Unavailable},
    };

    use super::*;

    /// Sessions held in memory, so the steps can be checked without a file.
    struct Remembered {
        stored: Mutex<StoredSessions>,
    }

    impl Remembered {
        fn empty() -> Self {
            Remembered::holding(StoredSessions {
                next_id: "1".to_owned(),
                sessions: Vec::new(),
            })
        }

        fn holding(stored: StoredSessions) -> Self {
            Remembered {
                stored: Mutex::new(stored),
            }
        }

        fn load(&self) -> StoredSessions {
            self.stored.lock().unwrap().clone()
        }
    }

    impl SessionStore for Remembered {
        fn update(
            &self,
            change: &mut dyn FnMut(&mut StoredSessions) -> bool,
        ) -> Result<(), Unavailable> {
            // The lock is held across the read and the write, which is what
            // the port promises. A fake that let go between them would allow
            // the very thing the real store is written to prevent.
            let mut held = self.stored.lock().unwrap();
            let mut sessions = held.clone();
            if change(&mut sessions) {
                *held = sessions;
            }
            Ok(())
        }
    }

    /// A backlog held in memory.
    struct Tasks {
        stored: Mutex<StoredBacklog>,
    }

    impl Tasks {
        fn holding(tasks: Vec<StoredTask>) -> Self {
            Tasks {
                stored: Mutex::new(StoredBacklog {
                    next_id: (tasks.len() + 1).to_string(),
                    tasks,
                }),
            }
        }

        fn first(&self) -> StoredTask {
            self.stored.lock().unwrap().tasks[0].clone()
        }
    }

    impl BacklogStore for Tasks {
        fn load(&self) -> Result<StoredBacklog, Unavailable> {
            Ok(self.stored.lock().unwrap().clone())
        }

        fn update(
            &self,
            change: &mut dyn FnMut(&mut StoredBacklog) -> bool,
        ) -> Result<(), Unavailable> {
            let mut held = self.stored.lock().unwrap();
            let mut tasks = held.clone();
            if change(&mut tasks) {
                *held = tasks;
            }
            Ok(())
        }
    }

    fn a_pending_task() -> StoredTask {
        StoredTask {
            id: "1".to_owned(),
            title: "tidy up".to_owned(),
            instruction: "tidy up src/utils".to_owned(),
            branch: None,
            after: None,
            model: None,
            repository: "/work/api".to_owned(),
            state: "Pending".to_owned(),
            session: None,
            worktree: None,
            reason: None,
        }
    }

    /// A configuration held in memory.
    struct Configured {
        plan: Option<&'static str>,
    }

    impl ConfigurationStore for Configured {
        fn load(&self) -> Result<StoredConfiguration, Unavailable> {
            Ok(StoredConfiguration {
                vendor: None,
                plan: self.plan.map(str::to_owned),
                usage_limit: None,
            })
        }

        fn store(&self, _stored: &StoredConfiguration) -> Result<(), Unavailable> {
            Ok(())
        }
    }

    static ON_A_PLAN: Configured = Configured {
        plan: Some("max-20x"),
    };
    static ON_NO_PLAN: Configured = Configured { plan: None };

    /// Work areas that are only remembered, so no repository is needed.
    #[derive(Default)]
    struct Areas {
        cut: Mutex<Vec<(String, String, String)>>,
        refuse: bool,
    }

    impl Worktrees for Areas {
        fn prepare(&self, cut: Cut<'_>) -> Result<String, Unavailable> {
            if self.refuse {
                return Err(Unavailable::new("no such base branch"));
            }
            self.cut.lock().unwrap().push((
                cut.repository.to_owned(),
                cut.base.to_owned(),
                cut.branch.to_owned(),
            ));
            Ok(format!("/areas/{}", cut.task))
        }
    }

    /// An agent that answers as it was told to, and remembers what it was asked.
    struct Standing {
        ended: Ended,
        asked: Mutex<Vec<(String, String, Option<String>)>>,
    }

    impl Standing {
        fn ending(ended: Ended) -> Self {
            Standing {
                ended,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn finishing() -> Self {
            Standing::ending(Ended {
                done: true,
                reason: None,
            })
        }
    }

    impl Agent for Standing {
        fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable> {
            self.asked.lock().unwrap().push((
                work.at.to_owned(),
                work.instruction.to_owned(),
                work.model.map(str::to_owned),
            ));
            Ok(self.ended.clone())
        }
    }

    fn declaring<'a>(usage: &'a str, time: &'a str) -> Declaration<'a> {
        Declaration {
            usage,
            time,
            model: None,
        }
    }

    #[test]
    fn a_session_opens_and_answers_what_it_was_declared_with() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        let started = execution.run(declaring("50%", "8h")).unwrap();
        assert_eq!(started.session, "session:1");
        assert_eq!(started.state, "running");
        assert_eq!(started.assigned, vec!["task:1".to_owned()]);
        assert_eq!(started.budget.usage, "50%");
        assert_eq!(started.budget.time, "8h");
    }

    #[test]
    fn what_was_opened_is_there_for_the_next_command_to_read() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);
        execution.run(declaring("2M", "30m")).unwrap();

        let held = sessions.load();
        assert_eq!(held.next_id, "2");
        assert_eq!(held.sessions.len(), 1);
        assert_eq!(held.sessions[0].state, "running");
        assert_eq!(held.sessions[0].usage, "2000000");
    }

    /// A task that was assigned is running and belongs to the session that
    /// took it, so nothing else may take it as well.
    #[test]
    fn the_task_that_was_assigned_says_which_session_took_it() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);
        execution.run(declaring("50%", "8h")).unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Running");
        assert_eq!(held.session.as_deref(), Some("1"));
    }

    #[test]
    fn a_second_session_is_refused_while_one_is_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);
        execution.run(declaring("50%", "8h")).unwrap();

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::AlreadyRunning {
                id: "session:1".to_owned()
            })
        );
        assert_eq!(sessions.load().sessions.len(), 1);
    }

    fn a_second_pending_task() -> StoredTask {
        StoredTask {
            id: "2".to_owned(),
            ..a_pending_task()
        }
    }

    #[test]
    fn a_run_with_nothing_to_start_is_refused_and_opens_no_session() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(Vec::new());
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::NothingToAssign)
        );
        assert!(sessions.load().sessions.is_empty());
    }

    #[test]
    fn a_share_of_a_plan_nobody_configured_is_refused() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_NO_PLAN, &areas, &agent);

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::NoPlanConfigured)
        );
    }

    /// An absolute count is measured against nothing, so it needs no plan.
    #[test]
    fn a_count_of_tokens_does_not_need_a_plan() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_NO_PLAN, &areas, &agent);

        assert!(execution.run(declaring("2M", "8h")).is_ok());
    }

    #[test]
    fn a_declaration_that_cannot_be_read_is_refused_as_a_bad_argument() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        assert_eq!(
            execution.run(declaring("50%", "8x")),
            Err(Refusal::BadValue {
                key: "time".to_owned(),
                value: "8x".to_owned()
            })
        );
        assert_eq!(
            execution.run(declaring("half", "8h")),
            Err(Refusal::BadValue {
                key: "usage".to_owned(),
                value: "half".to_owned()
            })
        );
    }

    #[test]
    fn a_stored_session_that_cannot_be_read_fails_as_a_store() {
        let sessions = Remembered::holding(StoredSessions {
            next_id: "2".to_owned(),
            sessions: vec![StoredSession {
                id: "1".to_owned(),
                state: "sprinting".to_owned(),
                stopped_reason: None,
                usage: "50%".to_owned(),
                time: "8h".to_owned(),
                model: None,
            }],
        });
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        let refused = execution.run(declaring("50%", "8h")).unwrap_err();
        assert!(matches!(refused, Refusal::Unavailable { reason } if reason.contains("sprinting")));
    }

    #[test]
    fn a_task_that_was_carried_on_ends_completed_where_it_was_worked_on() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        assert_eq!(
            areas.cut.lock().unwrap().as_slice(),
            [(
                "/work/api".to_owned(),
                "main".to_owned(),
                "cistern/1".to_owned()
            )]
        );
        assert_eq!(
            agent.asked.lock().unwrap().as_slice(),
            [("/areas/1".to_owned(), "tidy up src/utils".to_owned(), None)]
        );

        let held = tasks.first();
        assert_eq!(held.state, "Completed");
        assert_eq!(held.worktree.as_deref(), Some("/areas/1"));
        assert_eq!(held.reason, None);
    }

    #[test]
    fn an_agent_that_failed_leaves_the_task_in_error_with_what_it_said() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            done: false,
            reason: Some("it went wrong".to_owned()),
        });
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("it went wrong"));
    }

    /// The executor is called for one task at a time and several at once, so
    /// two running together have to end in their own places without either
    /// losing what the other recorded.
    #[test]
    fn two_tasks_carried_on_at_once_each_end_in_their_own_place() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        // The supervisor is what assigns more than one; until it exists a test
        // stands in for it by assigning the second task itself.
        execution.run(declaring("50%", "8h")).unwrap();
        let opened = SessionId::parse("1").unwrap();
        backlog::change(&tasks, |held| Ok(held.assign(opened))).unwrap();

        let execution = &execution;
        std::thread::scope(|threads| {
            for task in ["task:1", "task:2"] {
                threads.spawn(move || execution.carry_on(task).unwrap());
            }
        });

        let held = tasks.load().unwrap().tasks;
        assert_eq!(held[0].state, "Completed");
        assert_eq!(held[1].state, "Completed");
        assert_eq!(held[0].worktree.as_deref(), Some("/areas/1"));
        assert_eq!(held[1].worktree.as_deref(), Some("/areas/2"));
    }

    /// A task with nowhere to work has ended, and saying so is what keeps it
    /// from being read as still running.
    #[test]
    fn a_work_area_that_could_not_be_made_leaves_the_task_in_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas {
            refuse: true,
            ..Areas::default()
        };
        let agent = Standing::finishing();
        let execution = ExecutionService::new(&sessions, &tasks, &ON_A_PLAN, &areas, &agent);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("no such base branch"));
        assert!(agent.asked.lock().unwrap().is_empty());
    }
}
