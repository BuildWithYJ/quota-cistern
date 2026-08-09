//! What `run` does.

use crate::core::{
    domain::{
        Budget, Consumption, Cost, Held, NotASessionSet, NotOpened, Observation, Opening, Session,
        SessionId, SessionState, Sessions, Span, Spending, StoppedReason, Task, TaskId, TaskState,
        Usage, cost_of, room_for,
    },
    port::{
        inbound::{Declaration, Declared, ExecutionUseCase, Refusal, Started},
        outbound::{
            Agent, BacklogStore, Clock, Cut, Limit, Observed, Outcome, SessionStore, Spent,
            StoredSession, StoredSessions, Work, Worktrees,
        },
    },
};

use super::backlog;

/// The commands over sessions, and what they need from outside.
pub struct ExecutionService<'a> {
    sessions: &'a dyn SessionStore,
    tasks: &'a dyn BacklogStore,
    worktrees: &'a dyn Worktrees,
    agent: &'a dyn Agent,
    clock: &'a dyn Clock,
    limit: &'a dyn Limit,
}

impl<'a> ExecutionService<'a> {
    pub fn new(
        sessions: &'a dyn SessionStore,
        tasks: &'a dyn BacklogStore,
        worktrees: &'a dyn Worktrees,
        agent: &'a dyn Agent,
        clock: &'a dyn Clock,
        limit: &'a dyn Limit,
    ) -> Self {
        ExecutionService {
            sessions,
            tasks,
            worktrees,
            agent,
            clock,
            limit,
        }
    }

    /// How far the vendor's limit is spent, as a whole number of percent.
    ///
    /// Only a session declared as a share asks, since only a share is measured
    /// against it and asking costs something.
    fn limit_now(&self) -> Result<u32, Refusal> {
        let reading = self.limit.read()?;
        reading
            .used
            .parse()
            .map_err(|_| unreadable("used", &reading.used))
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

        // Asked before a session is opened, so that a run with nothing to do
        // does not leave one behind that has to be stopped again.
        if backlog::read(self.tasks)?.next_to_assign().is_none() {
            return Err(Refusal::NothingToAssign);
        }

        let budget = Budget { usage, time };
        let model = declared.model.map(str::to_owned);

        // Read before the session opens, so that a share is measured from
        // where the vendor's limit stood when this session had spent nothing.
        let started_at = self.clock.now();
        let limit_at_start = match usage {
            Usage::Share(_) => Some(self.limit_now()?),
            Usage::Tokens(_) => None,
        };

        let opened = change(self.sessions, |sessions| {
            sessions
                .open(Opening {
                    budget,
                    model,
                    started_at,
                    limit_at_start,
                })
                .map_err(|NotOpened::AlreadyRunning { id }| Refusal::AlreadyRunning {
                    id: id.labelled(),
                })
        })?;

        let assigned = self.settle(opened)?;

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

    fn carry_on(&self, task: &str) -> Result<Vec<String>, Refusal> {
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
            // Nothing ran, so there is nothing to have consumed.
            Err(e) => return self.ended(id, TaskState::Error, Some(e.reason), Observation::NotYet),
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
            Ok(ended) => {
                let consumed = observed(ended.observed);
                match ended.outcome {
                    Outcome::Finished => self.ended(id, TaskState::Completed, None, consumed),
                    // Section 1 gives a run stopped at its ceiling a reason of
                    // its own, and says the session carries on.
                    Outcome::AtCeiling => self.ended(
                        id,
                        TaskState::Interrupted,
                        Some(AT_CEILING.to_owned()),
                        consumed,
                    ),
                    // A run the vendor would not take fails the same way as
                    // one that went wrong, and only the vendor's limit tells
                    // them apart. It is asked here rather than on every task,
                    // since asking costs a turn and a task rarely fails.
                    Outcome::Failed => match self.at_its_limit() {
                        true => self.turned_away(id, consumed),
                        false => self.ended(id, TaskState::Error, ended.reason, consumed),
                    },
                }
            }
            Err(e) => self.ended(id, TaskState::Error, Some(e.reason), Observation::NotYet),
        }
    }
}

/// Reads what the agent said it consumed.
///
/// The port answers in the core's own words already, so this only tells the two
/// answers apart. A count the adapter could not read is not a count of nothing,
/// and section 1 keeps the two apart as far as the reason a session stops.
fn observed(observed: Observed) -> Observation {
    match observed {
        Observed::Unreadable { why } => Observation::Unreadable { why },
        Observed::Spent(spent) => match counted(&spent) {
            Some(counted) => Observation::Spent(counted),
            None => Observation::Unreadable {
                why: "what the agent counted does not read as a number".to_owned(),
            },
        },
    }
}

/// A count as the port hands it over, if every figure in it is one.
fn counted(spent: &Spent) -> Option<Consumption> {
    Some(Consumption {
        input: spent.input.parse().ok()?,
        output: spent.output.parse().ok()?,
        cache_written: spent.cache_written.parse().ok()?,
        cache_read: spent.cache_read.parse().ok()?,
        cost: spent.cost.parse().ok()?,
    })
}

/// The reason section 1 gives a task stopped at the ceiling on one run.
const AT_CEILING: &str = "task ceiling";

/// The reading at which the vendor has nothing left to give.
const FULL: u32 = 100;

impl ExecutionService<'_> {
    /// Whether the vendor has nothing left to give.
    ///
    /// A reading this cannot take is not a limit that has been reached. The
    /// run failed either way, and calling it the vendor's doing on a question
    /// nobody could answer would stop a session that had room left.
    fn at_its_limit(&self) -> bool {
        self.limit_now().is_ok_and(|used| used >= FULL)
    }

    /// A task the vendor would not run, and the session it belonged to.
    ///
    /// The task goes back to waiting, since nothing about it was wrong and it
    /// is the vendor that has to change its mind. The session stops, because
    /// every other task in it would be turned away the same way.
    fn turned_away(&self, id: TaskId, consumed: Observation) -> Result<Vec<String>, Refusal> {
        let session = backlog::change(self.tasks, |tasks| {
            tasks.record(id, consumed.clone());
            let session = tasks.find(id).and_then(Task::session);
            tasks.wait_again(id);
            Ok(session)
        })?;

        if let Some(session) = session {
            self.stop(session, StoppedReason::VendorLimit)?;
        }
        Ok(Vec::new())
    }

    /// Moves a task to the state it ended in, records what it consumed, and
    /// decides what happens next.
    ///
    /// The first two are one change, so that a task is never stored as ended
    /// with what it consumed still missing.
    fn ended(
        &self,
        id: TaskId,
        state: TaskState,
        reason: Option<String>,
        consumed: Observation,
    ) -> Result<Vec<String>, Refusal> {
        let session = backlog::change(self.tasks, |tasks| {
            tasks.finish(id, state, reason.clone());
            tasks.record(id, consumed.clone());
            Ok(tasks.find(id).and_then(Task::session))
        })?;

        let Some(session) = session else {
            return Ok(Vec::new());
        };
        self.settle(session).map(labelled)
    }

    /// One decision: whether the session carries on, and with what.
    ///
    /// Section 2.2 says assignment is dynamic and this is the whole of it.
    /// When this is called is the composition root's; what it decides is here.
    fn settle(&self, session: SessionId) -> Result<Vec<TaskId>, Refusal> {
        let Some(held) = self.held(session)? else {
            return Ok(Vec::new());
        };

        let spent = self.spending(&held)?;
        let (left, running, waiting, cost) = backlog::read(self.tasks).map(|tasks| {
            let cost = match (held.budget().usage, spent) {
                // Tokens mean the same in every session, so what a task costs
                // is what tasks have cost here at all.
                (Usage::Tokens(_), _) => {
                    Some(cost_of(tasks.counted().iter().map(Consumption::tokens)))
                }
                // A share is how far this session moved the vendor's limit,
                // and only this session's tasks moved it.
                (Usage::Share(_), Spending::Share(points)) => Some(Cost {
                    total: u64::from(points),
                    over: tasks.ended_in(session) as u64,
                }),
                (Usage::Share(_), Spending::Tokens(_)) => None,
            };
            (
                held.budget().left(spent),
                tasks.running_in(session),
                tasks.next_to_assign().is_some(),
                cost,
            )
        })?;

        if let Some(why) = self.why_it_stops(&held, left, running, waiting, cost)? {
            return self.stop(session, why).map(|()| Vec::new());
        }
        let room = room_for(left, cost, running);
        backlog::change(self.tasks, |tasks| {
            Ok((0..room).filter_map(|_| tasks.assign(session)).collect())
        })
    }

    /// Why the session stops here, if it does.
    fn why_it_stops(
        &self,
        held: &Session,
        left: u64,
        running: usize,
        waiting: bool,
        cost: Option<Cost>,
    ) -> Result<Option<StoppedReason>, Refusal> {
        // A count nobody could read leaves a budget that cannot be measured,
        // and a budget that cannot be measured cannot be held to.
        if matches!(
            backlog::read(self.tasks)?.consumed_by(held.id()),
            Observation::Unreadable { .. }
        ) {
            return Ok(Some(StoppedReason::ObservationUnreadable));
        }
        if held.out_of_time(self.clock.now()) {
            return Ok(Some(StoppedReason::BudgetHardlock));
        }
        // Nothing more fits, and nothing is running that would make room.
        // Waiting for a task that will never start is not carrying on.
        if running == 0 && room_for(left, cost, 0) == 0 {
            return Ok(Some(StoppedReason::BudgetHardlock));
        }
        if running == 0 && !waiting {
            return Ok(Some(StoppedReason::AllDone));
        }
        Ok(None)
    }

    /// Stops the session and ends whatever it still had running.
    fn stop(&self, session: SessionId, why: StoppedReason) -> Result<(), Refusal> {
        change(self.sessions, |sessions| {
            sessions.stop(session, why);
            Ok(())
        })?;
        backlog::change(self.tasks, |tasks| {
            Ok(!tasks.interrupt(session, &why.to_string()).is_empty())
        })
        .map(|_: bool| ())
    }

    /// What the session has consumed of its usage budget.
    fn spending(&self, held: &Session) -> Result<Spending, Refusal> {
        match (held.budget().usage, held.limit_at_start()) {
            (Usage::Share(_), Some(at_start)) => {
                Ok(Spending::Share(self.limit_now()?.saturating_sub(at_start)))
            }
            // A share with nothing to measure from is a store this core cannot
            // use. Nothing else can be said about how much of it is spent.
            (Usage::Share(_), None) => Err(Refusal::Unavailable {
                reason: format!(
                    "{} declared a share and does not say what the limit was at",
                    held.id().labelled()
                ),
            }),
            (Usage::Tokens(_), _) => Ok(Spending::Tokens(
                Consumption::total(backlog::read(self.tasks)?.counted_in(held.id())).tokens(),
            )),
        }
    }

    /// The session, if it is one this still decides for.
    fn held(&self, session: SessionId) -> Result<Option<Session>, Refusal> {
        let mut found = None;
        change(self.sessions, |sessions| {
            found = sessions
                .sessions()
                .iter()
                .find(|held| held.id() == session)
                .filter(|held| held.state() == SessionState::Running)
                .cloned();
            Ok(())
        })?;
        Ok(found)
    }
}

fn labelled(ids: Vec<TaskId>) -> Vec<String> {
    ids.iter().map(TaskId::labelled).collect()
}

fn stored_count(field: &str, value: &str) -> Result<u64, Refusal> {
    value.parse().map_err(|_| unreadable(field, value))
}

fn stored_share(field: &str, value: &str) -> Result<u32, Refusal> {
    value.parse().map_err(|_| unreadable(field, value))
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
        started_at: stored_count("started_at", &one.started_at)?,
        limit_at_start: one
            .limit_at_start
            .as_deref()
            .map(|used| stored_share("limit_at_start", used))
            .transpose()?,
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
                started_at: session.started_at().to_string(),
                limit_at_start: session.limit_at_start().map(|used| used.to_string()),
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
    use std::sync::{Mutex, PoisonError};

    use crate::core::{
        domain::SessionId,
        port::outbound::{Ended, Reading, StoredBacklog, StoredTask, Unavailable},
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

    fn a_second_task() -> StoredTask {
        StoredTask {
            id: "2".to_owned(),
            title: "tidy up again".to_owned(),
            ..a_pending_task()
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
            consumed: None,
            unreadable: None,
        }
    }

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

    /// A clock that does not move, for the tests that do not care.
    struct Frozen(u64);

    impl Clock for Frozen {
        fn now(&self) -> u64 {
            self.0
        }
    }

    static STILL: Frozen = Frozen(1_000);

    /// A vendor limit that stands where a test put it.
    struct AtPercent {
        used: Mutex<u32>,
        refuse: bool,
    }

    impl Limit for AtPercent {
        fn read(&self) -> Result<Reading, Unavailable> {
            if self.refuse {
                return Err(Unavailable::new("the status line said nothing"));
            }
            Ok(Reading {
                used: self
                    .used
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .to_string(),
                resets_at: "1786285800".to_owned(),
            })
        }
    }

    /// A limit nothing asks for, since the session was declared in tokens.
    static UNTOUCHED: AtPercent = AtPercent {
        used: Mutex::new(0),
        refuse: false,
    };

    /// What an agent that answered with a count it could read reports.
    fn spending() -> Observed {
        Observed::Spent(Spent {
            input: "77".to_owned(),
            output: "3377".to_owned(),
            cache_written: "28879".to_owned(),
            cache_read: "263483".to_owned(),
            cost: "92170".to_owned(),
        })
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
                outcome: Outcome::Finished,
                reason: None,
                observed: spending(),
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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);
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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);
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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);
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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::NothingToAssign)
        );
        assert!(sessions.load().sessions.is_empty());
    }

    #[test]
    fn a_declaration_that_cannot_be_read_is_refused_as_a_bad_argument() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

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
                started_at: "1000".to_owned(),
                limit_at_start: None,
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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        let refused = execution.run(declaring("50%", "8h")).unwrap_err();
        assert!(matches!(refused, Refusal::Unavailable { reason } if reason.contains("sprinting")));
    }

    #[test]
    fn a_task_that_was_carried_on_ends_completed_where_it_was_worked_on() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

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
    fn a_task_that_ran_is_stored_with_what_it_consumed() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let counted = tasks.first().consumed.unwrap();
        assert_eq!(counted.input, "77");
        assert_eq!(counted.output, "3377");
        assert_eq!(counted.cache_written, "28879");
        assert_eq!(counted.cache_read, "263483");
        assert_eq!(counted.cost, "92170");
        assert_eq!(tasks.first().unreadable, None);

        // The backlog held one task and it is done, so nothing is left to
        // assign and the session has nothing more to do.
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("all done"));
    }

    /// A task that never reached the agent has not consumed nothing; it has not
    /// consumed at all, and neither field says otherwise.
    #[test]
    fn a_task_that_never_ran_is_stored_with_no_count_at_all() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas {
            refuse: true,
            ..Areas::default()
        };
        let agent = Standing::finishing();
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.consumed, None);
        assert_eq!(held.unreadable, None);
    }

    /// A budget is a figure, and a session that cannot be measured against its
    /// own would run past it without anything noticing.
    #[test]
    fn a_session_whose_count_could_not_be_read_stops_and_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: Observed::Unreadable {
                why: "the answer said nothing about it".to_owned(),
            },
        });
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.consumed, None);
        assert_eq!(
            held.unreadable.as_deref(),
            Some("the answer said nothing about it")
        );

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(
            session.stopped_reason.as_deref(),
            Some("observation unreadable")
        );
    }

    /// The agent answered with a count, and one figure in it is not a number.
    #[test]
    fn a_figure_that_does_not_read_as_a_number_is_not_a_count() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: Observed::Spent(Spent {
                input: "a lot".to_owned(),
                output: "3377".to_owned(),
                cache_written: "28879".to_owned(),
                cache_read: "263483".to_owned(),
                cost: "92170".to_owned(),
            }),
        });
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().consumed, None);
        assert!(tasks.first().unreadable.is_some());
    }

    #[test]
    fn an_agent_that_failed_leaves_the_task_in_error_with_what_it_said() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it went wrong".to_owned()),
            observed: spending(),
        });
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("it went wrong"));
    }

    /// Section 1 says the session carries on when one task hits the ceiling
    /// on a single run.
    #[test]
    fn a_task_stopped_at_its_ceiling_says_so_and_the_session_carries_on() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::AtCeiling,
            reason: Some("the agent was cut off after 200 turns".to_owned()),
            observed: spending(),
        });
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Interrupted");
        assert_eq!(held.reason.as_deref(), Some("task ceiling"));
        assert_eq!(sessions.load().sessions[0].state, "running");
    }

    /// A vendor that will not run one task will not run the next either, and
    /// nothing about the task was wrong.
    #[test]
    fn a_task_the_vendor_would_not_run_waits_again_and_the_session_stops() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it stopped".to_owned()),
            observed: spending(),
        });
        let full = AtPercent {
            used: Mutex::new(100),
            refuse: false,
        };
        let execution = ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &full);

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Pending");
        assert_eq!(held.session, None);
        assert_eq!(held.reason, None);

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("vendor limit"));
    }

    /// A run can fail on its own account, and the vendor having room left is
    /// what says so.
    #[test]
    fn a_task_that_failed_with_room_left_is_an_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it went wrong".to_owned()),
            observed: spending(),
        });
        let room = AtPercent {
            used: Mutex::new(40),
            refuse: false,
        };
        let execution = ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &room);

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("it went wrong"));
        assert_eq!(sessions.load().sessions[0].state, "running");
    }

    /// A reading nobody could take is not a limit that has been reached.
    #[test]
    fn a_task_that_failed_with_no_reading_to_be_had_is_an_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it went wrong".to_owned()),
            observed: spending(),
        });
        let silent = AtPercent {
            used: Mutex::new(0),
            refuse: true,
        };
        let execution = ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &silent);

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().state, "Error");
    }

    /// The other half of the hardlock: a session that has spent the tokens it
    /// declared stops, whether or not its time is up.
    #[test]
    fn a_session_that_spent_what_it_declared_stops_and_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        // The stand-in agent reports far more than this budget allows, so the
        // first task spends the whole of it.
        execution.run(declaring("1000", "8h")).unwrap();
        let assigned = execution.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
        // The second task was never assigned, so it is still waiting.
        assert_eq!(tasks.load().unwrap().tasks[1].state, "Pending");
    }

    /// A session that has run as long as it declared stops, and whatever it
    /// still had running ends where it got to.
    #[test]
    fn a_session_out_of_time_stops_and_interrupts_what_was_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let late = Frozen(1_000 + 8 * 3_600);
        let opened = ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);
        opened.run(declaring("2M", "8h")).unwrap();

        let execution = ExecutionService::new(&sessions, &tasks, &areas, &agent, &late, &UNTOUCHED);
        let assigned = execution.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

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
        let execution =
            ExecutionService::new(&sessions, &tasks, &areas, &agent, &STILL, &UNTOUCHED);

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("no such base branch"));
        assert!(agent.asked.lock().unwrap().is_empty());
    }
}
