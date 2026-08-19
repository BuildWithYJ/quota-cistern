//! The judgement a session is run under.
//!
//! Whether a session carries on and with what is one decision, and this is where it is made.
//! `agent.rs` says a run's ceiling is the supervisor's and `consumption.rs` says what counts
//! towards a budget is the supervisor's; this is that role, in one place rather than in the
//! margins of the commands that happen to ask for it.
//!
//! It answers no command and implements no inbound port. A person types `run` and `interrupt`,
//! and the daemon's own workers carry tasks on; both reach a decision through here.

use crate::core::{
    domain::{
        Backlog, Consumption, Cost, Decision, HUNDREDTHS, Observation, Session, SessionId,
        SessionState, Spending, Standing, StoppedReason, TaskId, TaskState, Usage, cost_of, decide,
    },
    port::{
        inbound::Refusal,
        outbound::{Agent, BacklogStore, Clock, Limit, Runs, SessionStore, Traces, Worktrees},
    },
};

use super::{backlog, sessions};

/// What running a session needs from outside.
///
/// One value rather than an argument each, so that a port added later is a line here and a
/// line where the daemon is built, and nothing in between.
///
/// Copied rather than shared, since it holds nothing but references. Each service that needs
/// the outside takes its own, so that none of them reaches the outside through another.
#[derive(Clone, Copy)]
pub struct Outside<'a> {
    pub sessions: &'a dyn SessionStore,
    pub tasks: &'a dyn BacklogStore,
    pub worktrees: &'a dyn Worktrees,
    pub agent: &'a dyn Agent,
    pub clock: &'a dyn Clock,
    pub limit: &'a dyn Limit,
    pub traces: &'a dyn Traces,
    pub runs: &'a dyn Runs,
}

/// The reading at which the vendor has nothing left to give.
const FULL: u64 = 100 * HUNDREDTHS;

/// Who decides what a session does next.
///
/// The commands and the workers each hold their own ports and ask this one for a decision.
/// Nothing reaches the outside through here: a service that did would be borrowing a role's
/// state rather than holding its own.
///
/// What may be asked is `settle`, `stop`, `spending_of`, `limit_now`, and `at_its_limit`.
/// Everything else is this one's own.
pub struct Supervisor<'a> {
    outside: Outside<'a>,
    /// The most tasks this machine has hands for.
    ///
    /// A guard on the machine rather than on the budget, so the number belongs to whoever
    /// started the daemon rather than to a rule about spending.
    at_once: usize,
}

impl<'a> Supervisor<'a> {
    pub fn new(outside: Outside<'a>, at_once: usize) -> Self {
        Supervisor { outside, at_once }
    }
}

impl Supervisor<'_> {
    /// How far the vendor's limit is spent, and when the window it counts in begins again.
    ///
    /// Only a session declared as a share asks, since only a share is measured against it and
    /// asking costs something.
    ///
    /// A window that cannot be read is nothing rather than a failure. It tells one window from
    /// the next and the figure is still worth having without it.
    pub(super) fn limit_now(&self) -> Result<(u64, Option<u64>), Refusal> {
        let reading = self.outside.limit.read()?;
        let used = reading
            .used
            .parse()
            .map_err(|_| sessions::unreadable("used", &reading.used))?;
        Ok((used, reading.resets_at.parse().ok()))
    }

    /// Whether the vendor has nothing left to give.
    ///
    /// A reading this cannot take is not a limit that has been reached.
    /// The run failed either way.
    /// Calling it the vendor's doing on a question nobody could answer would stop a session that had room left.
    pub(super) fn at_its_limit(&self) -> bool {
        self.limit_now().is_ok_and(|(used, _)| used >= FULL)
    }

    /// One decision: whether the session carries on, and with what.
    ///
    /// Section 2.2 says assignment is dynamic and this is the whole of it.
    /// When this is called is the composition root's; what it decides is here.
    pub(super) fn settle(&self, session: SessionId) -> Result<Vec<TaskId>, Refusal> {
        let Some(held) = self.held(session)? else {
            return Ok(Vec::new());
        };

        // A share is read from the vendor, which is outside and slow, so it is read before the
        // hold below. Section 1 stops a session whose usage can no longer be read.
        let read = match held.budget().usage {
            Usage::Share(_) => match self.spending(&held)? {
                Some(spent) => Some(spent),
                None => {
                    return self
                        .stop(session, StoppedReason::ObservationUnreadable)
                        .map(|()| Vec::new());
                }
            },
            // A count is the backlog's own sum, so it is taken under the hold rather than
            // before it. Another task ending in between would leave this one deciding from a
            // budget that has since gone down.
            Usage::Tokens(_) => None,
        };

        let now = self.outside.clock.now();
        let settled = backlog::change(self.outside.tasks, |tasks| {
            let spent = match read {
                Some(spent) => spent,
                None => Spending::Tokens(Consumption::total(tasks.counted_in(session)).tokens()),
            };
            Ok(
                match decide(&standing(tasks, &held, spent, self.at_once, now)) {
                    // Stopping takes this store again and the sessions store with it, so it happens
                    // once this hold is given up rather than under it.
                    Decision::Stop(why) => Settled::Stop(spent, why),
                    Decision::Start(room) => Settled::Started(
                        spent,
                        (0..room)
                            .filter_map(|_| tasks.assign(session, now))
                            .collect(),
                    ),
                },
            )
        })?;

        // A share cannot be worked out again once the session has stopped.
        // What was read here is what is reported for it afterwards.
        //
        // A store that would not take it does not undo the assignment. The tasks are already
        // written down as running and their numbers are what the caller starts them by, so
        // dropping the numbers here leaves tasks nobody picks up and nothing to pick them up
        // with. The figure is written again at the next task to end, and the assignment is
        // not: one of the two comes back by itself.
        let spent = settled.spent();
        let recorded = sessions::change(self.outside.sessions, |sessions| {
            sessions.record(session, spent, now);
            Ok(())
        });

        match settled {
            // Stopping writes to the same store, so a store that would not take the figure
            // will not take the stopping either. That one is reported.
            Settled::Stop(_, why) => {
                recorded.and_then(|()| self.stop(session, why).map(|()| Vec::new()))
            }
            Settled::Started(_, assigned) => Ok(assigned),
        }
    }

    /// Stops the session and ends whatever it still had running.
    ///
    /// Marking a task interrupted does not end the run behind it. The run is a process this
    /// core started, and a session that stopped while one was still going would go on spending
    /// against a budget it had already reported as spent.
    pub(super) fn stop(&self, session: SessionId, why: StoppedReason) -> Result<(), Refusal> {
        let now = self.outside.clock.now();
        for task in backlog::read(self.outside.tasks)?.taken_by(session) {
            if task.state() == TaskState::Running {
                self.outside.agent.stop(&task.id().to_string());
            }
        }
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
    pub(super) fn spending_of(&self, session: SessionId) -> Result<Option<Spending>, Refusal> {
        let held = sessions::read(self.outside.sessions)?;
        let held = held
            .sessions()
            .iter()
            .find(|held| held.id() == session)
            .ok_or(Refusal::NoSessionRunning)?;
        self.spending(held)
    }

    /// What the session has consumed of its usage budget, or nothing where it can no longer
    /// be read.
    ///
    /// A share is the vendor's limit added up look by look, so the looking is what moves the
    /// figure and this writes as well as reads. That is why it is taken before the hold the
    /// decision is made under: asking the vendor is slow, and the adding up has to happen in
    /// the order the looks were taken rather than the order they arrive.
    ///
    /// A count is the backlog's own sum and is taken under that hold instead, so the arm here
    /// only serves a caller asking outside a decision.
    ///
    /// A vendor that stops answering leaves a share unknown rather than zero. Section 1 stops a
    /// session in that state, and nothing is a failure for the caller to carry.
    fn spending(&self, held: &Session) -> Result<Option<Spending>, Refusal> {
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
            (Usage::Tokens(_), _) => Ok(Some(Spending::Tokens(
                Consumption::total(backlog::read(self.outside.tasks)?.counted_in(session)).tokens(),
            ))),
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

/// What one decision came to, carried out of the hold it was made under.
enum Settled {
    Stop(Spending, StoppedReason),
    Started(Spending, Vec<TaskId>),
}

impl Settled {
    /// What the session had consumed when the decision was made.
    fn spent(&self) -> Spending {
        match self {
            Settled::Stop(spent, _) | Settled::Started(spent, _) => *spent,
        }
    }
}

/// How the session stands, from the backlog as it is held.
///
/// Every figure taken from the backlog comes from the one the assignment is made against, so
/// none of those can be from before another thread assigned.
///
/// What a share spent is not one of them. It is the vendor's, read before this hold, and put
/// beside a count of what ended that is read under it. A task ending between the two is
/// counted as having ended without what it spent being in the figure, so the cost of a task
/// reads low and `room_for` reads high. It is the wrong way to be wrong, and the alternative
/// is holding the store for the ninety seconds the reading takes.
fn standing(
    tasks: &Backlog,
    held: &Session,
    spent: Spending,
    at_once: usize,
    now: u64,
) -> Standing {
    let session = held.id();
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

    Standing {
        left: held.budget().left(spent),
        cost,
        running: tasks.running_in(session),
        waiting: tasks.next_to_assign().is_some(),
        out_of_time: held.out_of_time(now),
        unreadable: matches!(tasks.consumed_by(session), Observation::Unreadable { .. }),
        at_once,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use crate::core::{
        domain::{SessionId, StoppedReason},
        port::{
            inbound::{Carrying, ExecutionUseCase},
            outbound::{BacklogStore, Ended, Observed, Outcome},
        },
    };

    use super::super::fixtures::*;
    use super::super::{ExecutionService, Outside, Supervisor, WorkService};

    /// A budget is a figure.
    /// A session that cannot be measured against its own would run past it without anything noticing.
    #[test]
    fn a_session_whose_count_could_not_be_read_stops_and_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: Observed::Unreadable {
                why: "the answer said nothing about it".to_owned(),
            },
        });
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("50%", "8h")).unwrap();
        work.carry_on("task:1").unwrap();

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

    /// The other half of the hardlock: a session that has spent the tokens it declared stops.
    /// It stops whether or not its time is up.
    #[test]
    fn a_session_that_spent_what_it_declared_stops_and_says_so() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        // The stand-in agent reports far more than this budget allows, so the first task spends the whole of it.
        execution.run(declaring("1000", "8h")).unwrap();
        let assigned = work.carry_on("task:1").unwrap();

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
        let agent = Answering::finishing();
        // Half a point every time it is asked, which is what a task cost when this was measured against the vendor.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 50,
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &moving,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        // The first decision has no task to go on, so one starts alone.
        let started = execution.run(declaring("5%", "8h")).unwrap();
        assert_eq!(started.assigned.len(), 1);

        // The second knows what one task cost, and the rest of the budget holds far more than a handful.
        let assigned = work.carry_on("task:1").unwrap();
        assert_eq!(assigned.len(), 4);
    }

    /// A share is spent against a figure the vendor keeps.
    /// A session that reaches it stops however few tokens its own tasks reported.
    #[test]
    fn a_share_that_reached_what_it_declared_stops() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        // One point every time it is asked, against a budget of one point.
        let moving = Advancing {
            used: Mutex::new(0),
            step: 100,
        };
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &moving,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("1%", "8h")).unwrap();
        let assigned = work.carry_on("task:1").unwrap();

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
        let agent = Answering::finishing();
        // Opens at 30%, climbs to 34%, and then the window begins again at 2%.
        let turning = Turning::over(&[3_000, 3_400, 200]);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &turning,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        execution.run(declaring("10%", "8h")).unwrap();
        let assigned = work.carry_on("task:1").unwrap();

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
        let agent = Answering::finishing();
        let turning = Turning::over(&[3_000, 3_100, 100]);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &turning,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);

        // Nothing has reported yet, so one starts alone.
        let started = execution.run(declaring("50%", "8h")).unwrap();
        assert_eq!(started.assigned.len(), 1);

        // The window turned over between the two, and two points still divide into what is left.
        let assigned = work.carry_on("task:1").unwrap();
        assert_eq!(assigned.len(), 4);
    }

    /// Two of a session's tasks ending at once each decide how many more fit.
    ///
    /// A count of what is running that was read before the backlog was held is a count from
    /// before the other thread assigned, and both threads then assign against it. What follows
    /// is more running than the machine was told to run.
    ///
    /// The interleaving is the machine's to choose, so this runs the scene many times rather
    /// than once.
    #[test]
    fn two_tasks_ending_at_once_do_not_start_more_than_the_machine_takes() {
        for _ in 0..64 {
            let sessions = Remembered::empty();
            let tasks = Tasks::holding((1..=10).map(|n| a_task_numbered(&n.to_string())).collect());
            let areas = Areas::default();
            let agent = Answering::finishing();
            let runs = Ledger::default();
            let outside = Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
                runs: &runs,
            };
            let supervisor = Supervisor::new(outside, AT_ONCE);
            let execution = ExecutionService::new(outside, &supervisor);
            let work = WorkService::new(outside, &supervisor);

            // One starts alone, and the one after it fills the machine.
            execution.run(declaring("2M", "8h")).unwrap();
            work.carry_on("task:1").unwrap();
            assert_eq!(running_in(&tasks), AT_ONCE);

            // Two of those four end together.
            std::thread::scope(|threads| {
                threads.spawn(|| work.carry_on("task:2"));
                threads.spawn(|| work.carry_on("task:3"));
            });

            assert!(
                running_in(&tasks) <= AT_ONCE,
                "{} running where the machine takes {AT_ONCE}",
                running_in(&tasks)
            );
        }
    }

    /// The budget binds before the ceiling on the machine does.
    ///
    /// What a session declared in tokens has spent is the backlog's own sum, and a task ending
    /// records into that same backlog. A count read before the hold is a count from before the
    /// other thread recorded, and the budget left over then reads higher than it is.
    ///
    /// One task here consumes 295,816 tokens, so three fit in a million and four do not.
    #[test]
    fn two_tasks_ending_at_once_do_not_assign_past_what_is_left() {
        for _ in 0..64 {
            let sessions = Remembered::empty();
            let tasks = Tasks::holding((1..=10).map(|n| a_task_numbered(&n.to_string())).collect());
            let areas = Areas::default();
            let agent = Answering::finishing();
            let runs = Ledger::default();
            let outside = Outside {
                sessions: &sessions,
                tasks: &tasks,
                worktrees: &areas,
                agent: &agent,
                clock: &STILL,
                limit: &UNTOUCHED,
                traces: &NOTHING_KEPT,
                runs: &runs,
            };
            // Room enough that what is left is what decides, not the machine.
            let supervisor = Supervisor::new(outside, 8);
            let execution = ExecutionService::new(outside, &supervisor);
            let work = WorkService::new(outside, &supervisor);

            // One alone, then two once there is a cost to divide by.
            execution.run(declaring("1M", "8h")).unwrap();
            work.carry_on("task:1").unwrap();
            assert_eq!(running_in(&tasks), 2);

            // Both of those end together, and a fourth would put the session past its million.
            std::thread::scope(|threads| {
                threads.spawn(|| work.carry_on("task:2"));
                threads.spawn(|| work.carry_on("task:3"));
            });

            let started = tasks
                .load()
                .unwrap()
                .tasks
                .iter()
                .filter(|task| task.state != "Pending")
                .count();
            assert!(
                started <= 3,
                "{started} tasks started against a budget for 3"
            );
        }
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
        let agent = Answering::finishing();
        let limit = AtPercent::at(1_000);
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &limit,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        let execution = ExecutionService::new(outside, &supervisor);
        let work = WorkService::new(outside, &supervisor);
        execution.run(declaring("50%", "8h")).unwrap();

        limit.refuse();
        work.carry_on("1").unwrap();

        let held = sessions.load();
        assert_eq!(held.sessions[0].state, "stopped");
        assert_eq!(
            held.sessions[0].stopped_reason.as_deref(),
            Some("observation unreadable")
        );
    }

    /// Marking a task interrupted does not end the run behind it.
    ///
    /// A session that stopped while one was still going would go on spending against a budget
    /// it had already reported as spent, and nothing else would end it.
    #[test]
    fn stopping_a_session_ends_the_runs_it_still_had_going() {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
        let areas = Areas::default();
        let agent = Answering::finishing();
        let runs = Ledger::default();
        let outside = Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        };
        let supervisor = Supervisor::new(outside, AT_ONCE);
        ExecutionService::new(outside, &supervisor)
            .run(declaring("2M", "8h"))
            .unwrap();

        let running = tasks.running();
        assert!(!running.is_empty(), "nothing was running to end");
        supervisor
            .stop(SessionId::parse("1").unwrap(), StoppedReason::AllDone)
            .unwrap();

        let ended = agent.stopped.lock().unwrap().clone();
        assert_eq!(ended, running, "a run outlived the session that started it");
    }
}
