//! What `run` does.

use crate::core::{
    domain::{
        Budget, Consumption, Cost, Decision, HUNDREDTHS, NotOpened, Observation, Opening, Session,
        SessionId, SessionState, Span, Spending, Standing, StoppedReason, Task, TaskId, TaskState,
        Usage, cost_of, decide,
    },
    port::{
        inbound::{
            Carrying, Declaration, Declared, ExecutionUseCase, Happened, Listed, NotCarried, Page,
            Ran, Refusal, Report, Started, Stopped, Trail,
        },
        outbound::{
            Agent, BacklogStore, Clock, Cut, Limit, Observed, Outcome, SessionStore, Spent, Traces,
            Work, Worktrees,
        },
    },
};

use super::{backlog, sessions};

/// What running a session needs from outside.
///
/// One value rather than an argument each, so that a port added later is a line here and a
/// line where the daemon is built, and nothing in between.
pub struct Outside<'a> {
    pub sessions: &'a dyn SessionStore,
    pub tasks: &'a dyn BacklogStore,
    pub worktrees: &'a dyn Worktrees,
    pub agent: &'a dyn Agent,
    pub clock: &'a dyn Clock,
    pub limit: &'a dyn Limit,
    pub traces: &'a dyn Traces,
}

/// The commands over sessions, and what they need from outside.
pub struct ExecutionService<'a> {
    outside: Outside<'a>,
    /// The most tasks this machine has hands for.
    ///
    /// A guard on the machine rather than on the budget, so the number belongs to whoever
    /// started the daemon rather than to a rule about spending.
    at_once: usize,
}

impl<'a> ExecutionService<'a> {
    pub fn new(outside: Outside<'a>, at_once: usize) -> Self {
        ExecutionService { outside, at_once }
    }

    /// How far the vendor's limit is spent, and when the window it counts in begins again.
    ///
    /// Only a session declared as a share asks, since only a share is measured against it and
    /// asking costs something.
    ///
    /// A window that cannot be read is nothing rather than a failure. It tells one window from
    /// the next and the figure is still worth having without it.
    fn limit_now(&self) -> Result<(u64, Option<u64>), Refusal> {
        let reading = self.outside.limit.read()?;
        let used = reading
            .used
            .parse()
            .map_err(|_| sessions::unreadable("used", &reading.used))?;
        Ok((used, reading.resets_at.parse().ok()))
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

        // Asked before a session is opened.
        // A run with nothing to do would otherwise leave one behind that has to be stopped again.
        if backlog::read(self.outside.tasks)?
            .next_to_assign()
            .is_none()
        {
            return Err(Refusal::NothingToAssign);
        }

        let budget = Budget { usage, time };
        let model = declared.model.map(str::to_owned);

        // Read before the session opens.
        // A share is measured from where the vendor's limit stood when this session had spent nothing.
        let started_at = self.outside.clock.now();
        let limit_at_start = match usage {
            Usage::Share(_) => Some(self.limit_now()?.0),
            Usage::Tokens(_) => None,
        };

        let opened = sessions::change(self.outside.sessions, |sessions| {
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

    fn sessions(&self, page: Option<&str>, limit: Option<&str>) -> Result<Page, Refusal> {
        let page = counted_from("page", page, 1)?;
        let limit = counted_from("limit", limit, 20)?;

        let held = sessions::read(self.outside.sessions)?;
        let tasks = backlog::read(self.outside.tasks)?;

        // Newest first, which is the order the numbers were handed out in.
        let mut newest: Vec<&Session> = held.sessions().iter().collect();
        newest.sort_by_key(|session| std::cmp::Reverse(session.id()));

        let sessions = newest
            .into_iter()
            .skip(((page - 1) * limit) as usize)
            .take(limit as usize)
            .map(|session| Listed {
                id: session.id().labelled(),
                state: session.state().to_string(),
                consumed: session.consumed().to_string(),
                task_count: tasks.taken_by(session.id()).len(),
                updated_at: session.updated_at().to_string(),
            })
            .collect();

        Ok(Page {
            page,
            limit,
            sessions,
        })
    }

    fn session(&self, id: &str) -> Result<Report, Refusal> {
        let wanted = SessionId::parse(id).ok_or_else(|| Refusal::BadValue {
            key: "session".to_owned(),
            value: id.to_owned(),
        })?;

        let held = sessions::read(self.outside.sessions)?;
        let session = held
            .sessions()
            .iter()
            .find(|session| session.id() == wanted)
            .ok_or_else(|| Refusal::NoSuchSession {
                id: wanted.labelled(),
            })?;

        let tasks = backlog::read(self.outside.tasks)?;
        let ran = tasks
            .taken_by(wanted)
            .into_iter()
            .map(|task| Ran {
                id: task.id().labelled(),
                state: task.state().to_string(),
                title: task.title().to_owned(),
                branch: task.result_branch(),
                reason: task.reason().map(str::to_owned),
            })
            .collect();

        Ok(Report {
            session: session.id().labelled(),
            state: session.state().to_string(),
            budget: Declared {
                usage: session.budget().usage.to_string(),
                time: session.budget().time.to_string(),
            },
            consumed: Declared {
                usage: session.consumed().to_string(),
                time: self.elapsed(session).to_string(),
            },
            stopped_reason: session.stopped_reason().map(|why| why.to_string()),
            // Section 2.2 gives this to a session the vendor turned away. Every share reads it
            // now, so the reason it stopped is what decides, not whether it was ever read.
            resets_at: match session.stopped_reason() {
                Some(StoppedReason::VendorLimit) => session.resets_at().map(|at| at.to_string()),
                _ => None,
            },
            updated_at: session.updated_at().to_string(),
            tasks: ran,
        })
    }

    fn trace(&self, id: &str, since: Option<&str>) -> Result<Trail, Refusal> {
        let wanted = TaskId::parse(id).ok_or_else(|| Refusal::BadValue {
            key: "task".to_owned(),
            value: id.to_owned(),
        })?;
        let held = backlog::read(self.outside.tasks)?;
        let task = held.find(wanted).ok_or_else(|| Refusal::NoSuchTask {
            id: wanted.labelled(),
        })?;
        // Before the trace is read.
        // A run ending between the two would leave a reader holding the last of it and told there is more.
        let done = task.state() != TaskState::Running;

        let read = self
            .outside
            .traces
            .read(&wanted.to_string(), since.unwrap_or_default())?;
        Ok(Trail {
            events: read
                .events
                .into_iter()
                .map(|one| Happened {
                    at: one.at,
                    said: one.said,
                })
                .collect(),
            cursor: read.cursor,
            done,
        })
    }

    fn interrupt(&self) -> Result<Stopped, Refusal> {
        let held = sessions::read(self.outside.sessions)?;
        let running = held.running().ok_or(Refusal::NoSessionRunning)?.id();

        // Before anything is ended.
        // A task the vendor was still running reports nothing, and a share is read from the vendor.
        //
        // Measuring writes what it read. Where the vendor has stopped answering it writes
        // nothing and what was last recorded stands, since a session stopped by hand is
        // stopped either way.
        self.measure_in(running)?;
        let now = self.outside.clock.now();

        // The runs end before the tasks do, so nothing is recorded as ended while its agent is still working.
        for task in backlog::read(self.outside.tasks)?.taken_by(running) {
            if task.state() == TaskState::Running {
                self.outside.agent.stop(&task.id().to_string());
            }
        }

        let interrupted = backlog::change(self.outside.tasks, |tasks| {
            Ok(tasks.interrupt(running, &StoppedReason::Interrupted.to_string(), now))
        })?;
        sessions::change(self.outside.sessions, |sessions| {
            sessions.stop(running, StoppedReason::Interrupted, now);
            Ok(())
        })?;

        let held = sessions::read(self.outside.sessions)?;
        let session = held
            .sessions()
            .iter()
            .find(|session| session.id() == running)
            .ok_or(Refusal::NoSessionRunning)?;

        Ok(Stopped {
            session: running.labelled(),
            state: session.state().to_string(),
            interrupted_tasks: labelled(interrupted),
            consumed: Declared {
                usage: session.consumed().to_string(),
                time: self.elapsed(session).to_string(),
            },
        })
    }
}

/// Reads what the agent said it consumed.
///
/// The port answers in the core's own words already, so this only tells the two answers apart.
/// A count the adapter could not read is not a count of nothing.
/// Section 1 keeps the two apart as far as the reason a session stops.
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

/// A count a caller wrote, or what it defaults to when nobody wrote one.
///
/// Zero is refused for both.
/// Section 2.2 names `--page 0` as an argument error, and a page of nothing is the same kind of nothing.
fn counted_from(key: &str, written: Option<&str>, unless: u32) -> Result<u32, Refusal> {
    let Some(written) = written else {
        return Ok(unless);
    };
    written
        .parse()
        .ok()
        .filter(|&count| count > 0)
        .ok_or_else(|| Refusal::BadValue {
            key: key.to_owned(),
            value: written.to_owned(),
        })
}

/// The reason section 1 gives a task stopped at the ceiling on one run.
const AT_CEILING: &str = "task ceiling";

/// The reading at which the vendor has nothing left to give.
const FULL: u64 = 100 * HUNDREDTHS;

impl Carrying for ExecutionService<'_> {
    fn carry_on(&self, task: &str) -> Result<Vec<String>, NotCarried> {
        self.carrying(task).map_err(not_carried)
    }
}

impl ExecutionService<'_> {
    /// Everything `carry_on` does, in the words the rest of this file speaks.
    fn carrying(&self, task: &str) -> Result<Vec<String>, Refusal> {
        let id = TaskId::parse(task).ok_or_else(|| Refusal::BadValue {
            key: "task".to_owned(),
            value: task.to_owned(),
        })?;

        let (repository, base, branch, instruction, model) = {
            let tasks = backlog::read(self.outside.tasks)?;
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

        let at = match self.outside.worktrees.prepare(Cut {
            repository: &repository,
            base: &base,
            branch: &branch,
            task: &id.to_string(),
        }) {
            Ok(at) => at,
            // A task with nowhere to work has ended, and nothing ran, so there is nothing to have consumed.
            Err(e) => return self.ended(id, TaskState::Error, Some(e.reason), Observation::NotYet),
        };
        backlog::change(self.outside.tasks, |tasks| {
            tasks.work_area(id, at.clone());
            Ok(())
        })?;

        let trace = self.outside.traces.keeping(&id.to_string())?;
        let ended = self.outside.agent.work(Work {
            task: &id.to_string(),
            at: &at,
            trace,
            instruction: &instruction,
            model: model.as_deref(),
        });
        match ended {
            Ok(ended) => {
                let consumed = observed(ended.observed);
                match ended.outcome {
                    Outcome::Finished => self.ended(id, TaskState::Completed, None, consumed),
                    // Section 1 gives a run stopped at its ceiling a reason of its own.
                    // It also says the session carries on.
                    Outcome::AtCeiling => self.ended(
                        id,
                        TaskState::Interrupted,
                        Some(AT_CEILING.to_owned()),
                        consumed,
                    ),
                    // Only the vendor's limit tells a run it would not take from one that went wrong.
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

/// A refusal nobody asked for, as what it is to a worker.
fn not_carried(why: Refusal) -> NotCarried {
    match why {
        Refusal::NoSuchTask { id } => NotCarried::NoSuchTask { id },
        Refusal::Unavailable { reason } => NotCarried::Unavailable { reason },
        // Nothing else can reach here: the task was named by a session of this core.
        other => NotCarried::Unavailable {
            reason: format!("{other:?}"),
        },
    }
}

impl ExecutionService<'_> {
    /// How long the session has run.
    ///
    /// A session still running has run until now.
    /// One that stopped ran until the moment it last changed, which is the moment it stopped.
    fn elapsed(&self, session: &Session) -> Span {
        let until = match session.state() {
            SessionState::Running => self.outside.clock.now(),
            SessionState::Stopped => session.updated_at(),
        };
        Span::of(until.saturating_sub(session.started_at()))
    }

    /// Whether the vendor has nothing left to give.
    ///
    /// A reading this cannot take is not a limit that has been reached.
    /// The run failed either way.
    /// Calling it the vendor's doing on a question nobody could answer would stop a session that had room left.
    fn at_its_limit(&self) -> bool {
        self.limit_now().is_ok_and(|(used, _)| used >= FULL)
    }

    /// A task the vendor would not run, and the session it belonged to.
    ///
    /// The task goes back to waiting, since nothing about it was wrong.
    /// It is the vendor that has to change its mind.
    /// The session stops, because every other task in it would be turned away the same way.
    fn turned_away(&self, id: TaskId, consumed: Observation) -> Result<Vec<String>, Refusal> {
        let starts_over = self
            .outside
            .limit
            .read()
            .ok()
            .and_then(|at| at.resets_at.parse().ok());
        let now = self.outside.clock.now();
        let session = backlog::change(self.outside.tasks, |tasks| {
            tasks.record(id, consumed.clone());
            let session = tasks.find(id).and_then(Task::session);
            tasks.wait_again(id, now);
            Ok(session)
        })?;

        if let Some(session) = session {
            if let Some(at) = starts_over {
                sessions::change(self.outside.sessions, |sessions| {
                    sessions.resets_at(session, at);
                    Ok(())
                })?;
            }
            self.stop(session, StoppedReason::VendorLimit)?;
        }
        Ok(Vec::new())
    }

    /// Moves a task to the state it ended in, records what it consumed, and decides what happens next.
    ///
    /// The first two are one change, so that a task is never stored as ended with what it consumed still missing.
    fn ended(
        &self,
        id: TaskId,
        state: TaskState,
        reason: Option<String>,
        consumed: Observation,
    ) -> Result<Vec<String>, Refusal> {
        let now = self.outside.clock.now();
        let session = backlog::change(self.outside.tasks, |tasks| {
            tasks.finish(id, state, reason.clone(), now);
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

        // Section 1 stops a session whose usage can no longer be read, and for a share that
        // reading is the vendor's own limit.
        // Measuring writes what it read, so what is reported after the session stops is what
        // was read while it ran.
        let Some(spent) = self.measure(&held)? else {
            return self
                .stop(session, StoppedReason::ObservationUnreadable)
                .map(|()| Vec::new());
        };
        let now = self.outside.clock.now();

        match decide(&self.standing(&held, spent)?) {
            Decision::Stop(why) => self.stop(session, why).map(|()| Vec::new()),
            Decision::Start(room) => backlog::change(self.outside.tasks, |tasks| {
                Ok((0..room)
                    .filter_map(|_| tasks.assign(session, now))
                    .collect())
            }),
        }
    }

    /// How the session stands, read in one go.
    ///
    /// One read of the backlog rather than one per question, so that every figure the decision
    /// is made from comes from the same moment.
    fn standing(&self, held: &Session, spent: Spending) -> Result<Standing, Refusal> {
        let session = held.id();
        let tasks = backlog::read(self.outside.tasks)?;
        let cost = match (held.budget().usage, spent) {
            // Tokens mean the same in every session, so what a task costs is what tasks have cost here at all.
            (Usage::Tokens(_), _) => Some(cost_of(tasks.counted().iter().map(Consumption::tokens))),
            // A share is how far this session moved the vendor's limit, and only this session's tasks moved it.
            (Usage::Share(_), Spending::Share(spent)) => Some(Cost {
                total: spent,
                over: tasks.ended_in(session) as u64,
            }),
            (Usage::Share(_), Spending::Tokens(_)) => None,
        };

        Ok(Standing {
            left: held.budget().left(spent),
            cost,
            running: tasks.running_in(session),
            waiting: tasks.next_to_assign().is_some(),
            out_of_time: held.out_of_time(self.outside.clock.now()),
            unreadable: matches!(tasks.consumed_by(session), Observation::Unreadable { .. }),
            at_once: self.at_once,
        })
    }

    /// Stops the session and ends whatever it still had running.
    fn stop(&self, session: SessionId, why: StoppedReason) -> Result<(), Refusal> {
        let now = self.outside.clock.now();
        sessions::change(self.outside.sessions, |sessions| {
            sessions.stop(session, why, now);
            Ok(())
        })?;
        backlog::change(self.outside.tasks, |tasks| {
            Ok(!tasks.interrupt(session, &why.to_string(), now).is_empty())
        })
        .map(|_: bool| ())
    }

    /// The same, for a session named only by its number.
    fn measure_in(&self, session: SessionId) -> Result<Option<Spending>, Refusal> {
        let held = sessions::read(self.outside.sessions)?;
        let held = held
            .sessions()
            .iter()
            .find(|held| held.id() == session)
            .ok_or(Refusal::NoSessionRunning)?;
        self.measure(held)
    }

    /// Takes what the session has consumed of its usage budget and writes it down, or answers
    /// with nothing when it can no longer be read.
    ///
    /// This writes as well as reads. A share is the vendor's limit added up look by look, so
    /// the looking is what moves the figure, and a count is written here so that both units
    /// leave by the same door.
    ///
    /// A vendor that stops answering leaves a share unknown rather than zero. Section 1 stops a
    /// session in that state, and nothing is a failure for the caller to carry.
    fn measure(&self, held: &Session) -> Result<Option<Spending>, Refusal> {
        let session = held.id();
        let now = self.outside.clock.now();
        match (held.budget().usage, held.limit_at_start()) {
            (Usage::Share(_), Some(_)) => {
                let Ok((used, resets_at)) = self.limit_now() else {
                    return Ok(None);
                };
                sessions::change(self.outside.sessions, |sessions| {
                    Ok(sessions.measured(session, used, resets_at, now))
                })
            }
            // A share with nothing to measure from is a store this core cannot use.
            // Nothing else can be said about how much of it is spent.
            (Usage::Share(_), None) => Err(Refusal::Unavailable {
                reason: format!(
                    "{} declared a share and does not say what the limit was at",
                    session.labelled()
                ),
            }),
            (Usage::Tokens(_), _) => {
                let spent = Spending::Tokens(
                    Consumption::total(backlog::read(self.outside.tasks)?.counted_in(session))
                        .tokens(),
                );
                sessions::change(self.outside.sessions, |sessions| {
                    sessions.record(session, spent, now);
                    Ok(Some(spent))
                })
            }
        }
    }

    /// The session, if it is one this still decides for.
    fn held(&self, session: SessionId) -> Result<Option<Session>, Refusal> {
        let mut found = None;
        sessions::change(self.outside.sessions, |sessions| {
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

#[cfg(test)]
mod tests {
    /// The hands the composition root gives the core, fixed here.
    const AT_ONCE: usize = 4;

    use std::sync::{Mutex, PoisonError};

    use crate::core::{
        domain::SessionId,
        port::outbound::{
            Ended, Keeping, Reading, StoredBacklog, StoredSession, StoredSessions, StoredTask,
            Unavailable,
        },
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
            // Held across the read and the write, as the port promises.
            // A fake that let go would allow what the real store prevents.
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
        a_task_numbered("2")
    }

    fn a_task_numbered(id: &str) -> StoredTask {
        StoredTask {
            id: id.to_owned(),
            title: format!("tidy up, again ({id})"),
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
            started_at: None,
            ended_at: None,
            reason: None,
            consumed: None,
            unreadable: None,
            disposition: None,
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

    /// A trace store nothing looks at.
    struct Kept;

    impl Traces for Kept {
        fn keeping(&self, _task: &str) -> Result<Keeping, Unavailable> {
            Ok(Box::new(|_line: &str| {}))
        }

        fn read(
            &self,
            _task: &str,
            _from: &str,
        ) -> Result<crate::core::port::outbound::Read, Unavailable> {
            Ok(crate::core::port::outbound::Read {
                events: Vec::new(),
                cursor: "000000000000".to_owned(),
            })
        }
    }

    static NOTHING_KEPT: Kept = Kept;

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
        /// Hundredths of a percent, as the port carries it.
        used: Mutex<u64>,
        refuse: Mutex<bool>,
    }

    impl AtPercent {
        /// A limit that reads, until a test says it stops reading.
        fn at(used: u64) -> Self {
            AtPercent {
                used: Mutex::new(used),
                refuse: Mutex::new(false),
            }
        }

        fn refuse(&self) {
            *self.refuse.lock().unwrap_or_else(PoisonError::into_inner) = true;
        }
    }

    impl Limit for AtPercent {
        fn read(&self) -> Result<Reading, Unavailable> {
            if *self.refuse.lock().unwrap_or_else(PoisonError::into_inner) {
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

    /// A vendor limit that moves every time it is read.
    ///
    /// A session declared as a share is measured against a figure that grows while its tasks run.
    /// Nothing else here makes it grow.
    struct Advancing {
        used: Mutex<u64>,
        step: u64,
    }

    impl Limit for Advancing {
        fn read(&self) -> Result<Reading, Unavailable> {
            let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
            let now = *used;
            *used += self.step;
            Ok(Reading {
                used: now.to_string(),
                resets_at: "1786285800".to_owned(),
            })
        }
    }

    /// A vendor limit that reads off a list, so a test says what each look finds.
    ///
    /// This is how a window that begins again is put in front of the core: a reading lower than
    /// the one before it. The last entry stands once the list runs out, so a test writes only
    /// the looks it cares about.
    struct Turning {
        left: Mutex<Vec<u64>>,
    }

    impl Turning {
        fn over(readings: &[u64]) -> Self {
            Turning {
                left: Mutex::new(readings.to_vec()),
            }
        }
    }

    impl Limit for Turning {
        fn read(&self) -> Result<Reading, Unavailable> {
            let mut left = self.left.lock().unwrap_or_else(PoisonError::into_inner);
            let used = match left.len() {
                0 => 0,
                1 => left[0],
                _ => left.remove(0),
            };
            Ok(Reading {
                used: used.to_string(),
                resets_at: "1786285800".to_owned(),
            })
        }
    }

    /// A limit nothing asks for, since the session was declared in tokens.
    static UNTOUCHED: AtPercent = AtPercent {
        used: Mutex::new(0),
        refuse: Mutex::new(false),
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

    /// An agent that answers as told, and remembers what it was asked.
    struct Standing {
        ended: Ended,
        asked: Mutex<Vec<(String, String, Option<String>)>>,
        /// The tasks whose runs were asked to end.
        stopped: Mutex<Vec<String>>,
    }

    impl Standing {
        fn ending(ended: Ended) -> Self {
            Standing {
                ended,
                asked: Mutex::new(Vec::new()),
                stopped: Mutex::new(Vec::new()),
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
        fn stop(&self, task: &str) {
            self.stopped
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(task.to_owned());
        }

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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        execution.run(declaring("2M", "30m")).unwrap();

        let held = sessions.load();
        assert_eq!(held.next_id, "2");
        assert_eq!(held.sessions.len(), 1);
        assert_eq!(held.sessions[0].state, "running");
        assert_eq!(held.sessions[0].usage, "2000000");
    }

    /// A task that was assigned is running and belongs to the session that took it.
    /// Nothing else may take it as well.
    #[test]
    fn the_task_that_was_assigned_says_which_session_took_it() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
                limit_last_seen: None,
                consumed: "0".to_owned(),
                updated_at: "1000".to_owned(),
                resets_at: None,
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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        let refused = execution.run(declaring("50%", "8h")).unwrap_err();
        assert!(matches!(refused, Refusal::Unavailable { reason } if reason.contains("sprinting")));
    }

    #[test]
    fn a_task_that_was_carried_on_ends_completed_where_it_was_worked_on() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let counted = tasks.first().consumed.unwrap();
        assert_eq!(counted.input, "77");
        assert_eq!(counted.output, "3377");
        assert_eq!(counted.cache_written, "28879");
        assert_eq!(counted.cache_read, "263483");
        assert_eq!(counted.cost, "92170");
        assert_eq!(tasks.first().unreadable, None);

        // The backlog held one task and it is done.
        // Nothing is left to assign, and the session has nothing more to do.
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("all done"));
    }

    /// A task that never reached the agent has not consumed nothing; it has not consumed at all.
    /// Neither field says otherwise.
    #[test]
    fn a_task_that_never_ran_is_stored_with_no_count_at_all() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas {
            refuse: true,
            ..Areas::default()
        };
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.consumed, None);
        assert_eq!(held.unreadable, None);
    }

    /// A budget is a figure.
    /// A session that cannot be measured against its own would run past it without anything noticing.
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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("it went wrong"));
    }

    /// Section 1 says the session carries on when one task hits the ceiling on a single run.
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
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Interrupted");
        assert_eq!(held.reason.as_deref(), Some("task ceiling"));
        assert_eq!(sessions.load().sessions[0].state, "running");
    }

    /// A vendor that will not run one task will not run the next either, and nothing about the task was wrong.
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
            used: Mutex::new(100 * HUNDREDTHS),
            refuse: Mutex::new(false),
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &full,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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

    /// A run can fail on its own account, and the vendor having room left is what says so.
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
            used: Mutex::new(40 * HUNDREDTHS),
            refuse: Mutex::new(false),
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &room,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

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
            refuse: Mutex::new(true),
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &silent,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().state, "Error");
    }

    /// The other half of the hardlock: a session that has spent the tokens it declared stops.
    /// It stops whether or not its time is up.
    #[test]
    fn a_session_that_spent_what_it_declared_stops_and_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        // The stand-in agent reports far more than this budget allows, so the first task spends the whole of it.
        execution.run(declaring("1000", "8h")).unwrap();
        let assigned = execution.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
        // The second task was never assigned, so it is still waiting.
        assert_eq!(tasks.load().unwrap().tasks[1].state, "Pending");
    }

    /// A task moves the vendor's limit by less than a point, and for a while that read as costing nothing at all.
    ///
    /// Several tasks have to start once there is anything to divide by.
    #[test]
    fn a_share_starts_several_once_it_knows_what_a_task_costs() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![
            a_pending_task(),
            a_second_task(),
            a_task_numbered("3"),
            a_task_numbered("4"),
            a_task_numbered("5"),
        ]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        // Half a point every time it is asked, which is what a task cost when this was measured against the vendor.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 50,
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &moving,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        // The first decision has no task to go on, so one starts alone.
        let started = execution.run(declaring("5%", "8h")).unwrap();
        assert_eq!(started.assigned.len(), 1);

        // The second knows what one task cost, and the rest of the budget holds far more than a handful.
        let assigned = execution.carry_on("task:1").unwrap();
        assert_eq!(assigned.len(), 4);
    }

    /// A share is spent against a figure the vendor keeps.
    /// A session that reaches it stops however few tokens its own tasks reported.
    #[test]
    fn a_share_that_reached_what_it_declared_stops() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        // One point every time it is asked, against a budget of one point.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 100,
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &moving,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("1%", "8h")).unwrap();
        let assigned = execution.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
        assert_eq!(tasks.load().unwrap().tasks[1].state, "Pending");
    }

    /// A session declared as a share outlives the window its limit is kept in.
    ///
    /// What it spent in the window it opened in is counted towards what it declared, and it
    /// stops on that as it would have without the window turning over.
    #[test]
    fn a_share_that_crosses_a_window_counts_both_of_them() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        // Opens at 30%, climbs to 34%, and then the window begins again at 2%.
        let turning = Turning::over(&[3_000, 3_400, 200]);
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &turning,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("10%", "8h")).unwrap();
        let assigned = execution.carry_on("task:1").unwrap();

        // 4 points in the first window and 2 in the second, against the 10 it declared.
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.consumed, "600");
        assert!(assigned.is_empty());
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
    }

    /// The other half of what a window turning over used to do.
    ///
    /// A session that read as having spent nothing had no cost to divide by either, which is
    /// the state a session is in before its first task reports and which starts one at a time.
    #[test]
    fn a_share_that_crosses_a_window_does_not_fall_back_to_one_at_a_time() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![
            a_pending_task(),
            a_second_task(),
            a_task_numbered("3"),
            a_task_numbered("4"),
            a_task_numbered("5"),
        ]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let turning = Turning::over(&[3_000, 3_100, 100]);
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &turning,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        // Nothing has reported yet, so one starts alone.
        let started = execution.run(declaring("50%", "8h")).unwrap();
        assert_eq!(started.assigned.len(), 1);

        // The window turned over between the two, and two points still divide into what is left.
        let assigned = execution.carry_on("task:1").unwrap();
        assert_eq!(assigned.len(), 4);
    }

    /// Section 2.2 lists them newest first, which is the order the numbers were handed out in.
    #[test]
    fn sessions_are_listed_newest_first() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        // A budget one task overruns.
        // The first session ends after one, and the second task is left for a session of its own.
        execution.run(declaring("1000", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();
        execution.run(declaring("1000", "2h")).unwrap();

        let listed = execution.sessions(None, None).unwrap();
        let ids: Vec<&str> = listed.sessions.iter().map(|one| one.id.as_str()).collect();
        assert_eq!(ids, ["session:2", "session:1"]);
        assert_eq!(listed.page, 1);
        assert_eq!(listed.limit, 20);
    }

    #[test]
    fn a_page_holds_what_it_was_given_room_for() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("1000", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();
        execution.run(declaring("1000", "2h")).unwrap();

        let second = execution.sessions(Some("2"), Some("1")).unwrap();
        assert_eq!(second.sessions.len(), 1);
        assert_eq!(second.sessions[0].id, "session:1");
    }

    #[test]
    fn a_page_that_is_not_a_page_is_refused_as_a_bad_argument() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        for (page, limit) in [(Some("0"), None), (None, Some("0")), (Some("one"), None)] {
            assert!(matches!(
                execution.sessions(page, limit),
                Err(Refusal::BadValue { .. })
            ));
        }
    }

    #[test]
    fn nothing_has_run_and_the_list_is_empty() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        assert_eq!(execution.sessions(None, None).unwrap().sessions, Vec::new());
    }

    #[test]
    fn a_session_reports_what_it_declared_beside_what_it_consumed() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let report = execution.session("1").unwrap();
        assert_eq!(report.budget.usage, "2000000");
        assert_eq!(report.budget.time, "8h");
        // The stand-in agent reports the same count for every task.
        assert_eq!(report.consumed.usage, "295816");
        assert_eq!(report.tasks.len(), 2);
        assert_eq!(report.tasks[0].id, "task:1");
        assert_eq!(report.tasks[0].state, "Completed");
        assert_eq!(report.tasks[0].branch.as_deref(), Some("cistern/1"));
    }

    /// Section 2.2 leaves the reason empty while a session runs.
    #[test]
    fn a_running_session_says_nothing_about_why_it_stopped() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();

        let report = execution.session("1").unwrap();
        assert_eq!(report.state, "running");
        assert_eq!(report.stopped_reason, None);
        assert_eq!(report.resets_at, None);
    }

    /// Section 2.2 gives this to a session the vendor turned away, and a share reads the window
    /// at every look, so a share that stopped for any other reason has one to report and must
    /// not.
    #[test]
    fn a_share_that_stopped_for_another_reason_says_nothing_about_the_limit_starting_over() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        // One point every time it is asked, against a budget of one point.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 100,
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &moving,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("1%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        // The store holds the window every look recorded.
        assert_eq!(
            sessions.load().sessions[0].resets_at.as_deref(),
            Some("1786285800")
        );

        let report = execution.session("1").unwrap();
        assert_eq!(report.stopped_reason.as_deref(), Some("budget hardlock"));
        assert_eq!(report.resets_at, None);
    }

    #[test]
    fn a_session_the_vendor_turned_away_says_when_the_limit_starts_over() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it stopped".to_owned()),
            observed: spending(),
        });
        let full = AtPercent {
            used: Mutex::new(100 * HUNDREDTHS),
            refuse: Mutex::new(false),
        };
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &full,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let report = execution.session("1").unwrap();
        assert_eq!(report.stopped_reason.as_deref(), Some("vendor limit"));
        assert_eq!(report.resets_at.as_deref(), Some("1786285800"));
    }

    #[test]
    fn a_session_nobody_opened_is_not_there() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        assert_eq!(
            execution.session("7"),
            Err(Refusal::NoSuchSession {
                id: "session:7".to_owned()
            })
        );
    }

    #[test]
    fn interrupting_stops_the_session_and_ends_what_was_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        let stopped = execution.interrupt().unwrap();

        assert_eq!(stopped.session, "session:1");
        assert_eq!(stopped.state, "stopped");
        assert_eq!(stopped.interrupted_tasks, ["task:1"]);

        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.stopped_reason.as_deref(), Some("interrupted"));
        assert_eq!(tasks.first().state, "Interrupted");
        assert_eq!(tasks.first().reason.as_deref(), Some("interrupted"));
    }

    /// The run has to end before the task does.
    ///
    /// A task recorded as ended while its agent still works is a task nobody is watching.
    #[test]
    fn interrupting_ends_the_run_of_every_task_it_interrupts() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        execution.interrupt().unwrap();

        assert_eq!(
            agent
                .stopped
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_slice(),
            ["1"]
        );
    }

    /// The thread waiting on a killed agent comes back to say it failed.
    /// The task has already ended for a better reason.
    #[test]
    fn what_the_agent_says_afterwards_does_not_undo_the_interruption() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::ending(Ended {
            outcome: Outcome::Failed,
            reason: Some("it was killed".to_owned()),
            observed: spending(),
        });
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("2M", "8h")).unwrap();
        execution.interrupt().unwrap();
        execution.carry_on("task:1").unwrap();

        assert_eq!(tasks.first().state, "Interrupted");
        assert_eq!(tasks.first().reason.as_deref(), Some("interrupted"));
    }

    #[test]
    fn interrupting_with_nothing_running_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        assert_eq!(execution.interrupt(), Err(Refusal::NoSessionRunning));
    }

    /// A session that has run as long as it declared stops, and whatever it still had running ends where it got to.
    #[test]
    fn a_session_out_of_time_stops_and_interrupts_what_was_running() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let late = Frozen(1_000 + 8 * 3_600);
        let opened = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        opened.run(declaring("2M", "8h")).unwrap();

        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &late,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        let assigned = execution.carry_on("task:1").unwrap();

        assert!(assigned.is_empty());
        let session = sessions.load().sessions[0].clone();
        assert_eq!(session.state, "stopped");
        assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
    }

    /// The executor is called for one task at a time and several at once.
    /// Two running together have to end in their own places without either losing what the other recorded.
    #[test]
    fn two_tasks_carried_on_at_once_each_end_in_their_own_place() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_pending_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        // The supervisor is what assigns more than one.
        // Until it exists, a test stands in for it by assigning the second task itself.
        execution.run(declaring("50%", "8h")).unwrap();
        let opened = SessionId::parse("1").unwrap();
        backlog::change(&tasks, |held| Ok(held.assign(opened, 0))).unwrap();

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

    /// A task with nowhere to work has ended, and saying so is what keeps it from being read as still running.
    #[test]
    fn a_work_area_that_could_not_be_made_leaves_the_task_in_error() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas {
            refuse: true,
            ..Areas::default()
        };
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        execution.run(declaring("50%", "8h")).unwrap();
        execution.carry_on("task:1").unwrap();

        let held = tasks.first();
        assert_eq!(held.state, "Error");
        assert_eq!(held.reason.as_deref(), Some("no such base branch"));
        assert!(agent.asked.lock().unwrap().is_empty());
    }

    /// A reader is told whether more can still arrive.
    ///
    /// The run's state answers that, and the run is this service's to know.
    #[test]
    fn a_trace_says_whether_it_can_still_grow() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![StoredTask {
            session: Some("1".to_owned()),
            state: "Running".to_owned(),
            ..a_pending_task()
        }]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        assert!(!execution.trace("1", None).unwrap().done);

        tasks.stored.lock().unwrap().tasks[0].state = "Completed".to_owned();
        assert!(execution.trace("1", None).unwrap().done);
    }

    #[test]
    fn the_trace_of_a_task_nobody_registered_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(Vec::new());
        let areas = Areas::default();
        let agent = Standing::finishing();
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );

        assert_eq!(
            execution.trace("7", None),
            Err(Refusal::NoSuchTask {
                id: "task:7".to_owned()
            })
        );
    }
    /// Section 1 stops a session whose usage can no longer be read.
    ///
    /// For a share that reading is the vendor's limit, and a session that cannot be measured
    /// must not go on spending against a budget nobody can check.
    #[test]
    fn a_share_whose_limit_stops_being_readable_stops_the_session() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Standing::finishing();
        let limit = AtPercent::at(1_000);
        let execution = ExecutionService::new(
            Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &limit,
                traces: &NOTHING_KEPT,
            },
            AT_ONCE,
        );
        execution.run(declaring("50%", "8h")).unwrap();

        limit.refuse();
        execution.carry_on("1").unwrap();

        let held = sessions.load();
        assert_eq!(held.sessions[0].state, "stopped");
        assert_eq!(
            held.sessions[0].stopped_reason.as_deref(),
            Some("observation unreadable")
        );
    }
}
