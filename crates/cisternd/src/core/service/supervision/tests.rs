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

/// Carries every task a session has started through to its end.
///
/// A session assigns as tasks end, so what is running has to be looked at again each time
/// rather than listed once.
fn carry_them_all(work: &WorkService<'_>, tasks: &Tasks) {
    for _ in 0..100 {
        let held = tasks.load().unwrap();
        let Some(running) = held.tasks.iter().find(|task| task.state == "Running") else {
            return;
        };
        work.carry_on(&format!("task:{}", running.id)).unwrap();
    }
    panic!("a task kept running");
}

fn states(tasks: &Tasks) -> Vec<String> {
    tasks
        .load()
        .unwrap()
        .tasks
        .iter()
        .map(|task| task.state.clone())
        .collect()
}

/// What a ceiling does to a session, with a stand-in that stops where it is told to.
///
/// The tasks take more and more, and nothing has been run before, so the session learns
/// what a run costs from the runs it has already had. The first task has nothing to go on
/// and is given the whole budget; every one after it is held to a figure worked out from
/// the runs before it. That figure is widened by how few runs it came from, which is what
/// keeps a task that takes somewhat more than the last one from being stopped.
///
/// One at a time, so that each task decides against everything before it. Four at once
/// would start them all against the first figure.
#[test]
fn a_session_carries_tasks_that_take_more_than_the_ones_before_them() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![
        a_pending_task(),
        a_second_task(),
        a_task_numbered("3"),
        a_task_numbered("4"),
        a_task_numbered("5"),
    ]);
    let areas = Areas::default();
    let agent = Costing::taking([100, 200, 300, 400, 400]);
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
    let supervisor = Supervisor::new(outside, 1);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("100000", "8h")).unwrap();
    carry_them_all(&work, &tasks);

    assert_eq!(states(&tasks), ["Completed"; 5]);
}

/// Widening is not enough where the next task takes several times the last, and then the
/// run that was stopped is what raises the figure. Nothing else does: the tasks after it
/// are the ones that climb the ladder, so a session with none left ends where it stopped.
#[test]
fn a_run_that_was_stopped_is_what_lets_the_next_one_through() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![
        a_pending_task(),
        a_second_task(),
        a_task_numbered("3"),
    ]);
    let areas = Areas::default();
    let agent = Costing::taking([100, 400, 400]);
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
    let supervisor = Supervisor::new(outside, 1);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("100000", "8h")).unwrap();
    carry_them_all(&work, &tasks);

    // 100 finishes, 400 is stopped at twice 100, and the third goes through on the floor
    // that stopping left behind.
    assert_eq!(states(&tasks), ["Completed", "Interrupted", "Completed"]);
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

    // And the 4 that are left are what the next run may take. A run that crossed the
    // turnover says nothing about what a run of that model costs -- what it spent before
    // the window began again is in no reading at all -- so this is a session with a budget
    // and nothing to go on, which starts one and measures it.
    assert_eq!(assigned.len(), 1);
    assert_eq!(session.stopped_reason, None);
}

/// What a run of a share-declared session cost is read from what it was priced at, not from
/// how far the vendor's limit moved while it ran.
///
/// The vendor keeps one figure for the account, so runs going at once move it together and
/// the reading taken when each ends hands the first to finish whatever the others spent
/// meanwhile. Two runs of the same size then look nothing alike. Here they report the same
/// price and the readings split 900 to 100 between them, which is what that looks like;
/// both are the same size all the same, and what the next task is allowed follows from
/// that size rather than from the larger slice.
#[test]
fn two_runs_of_a_size_are_sized_alike_however_the_readings_split() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger(Mutex::new(vec![
        a_run_of("1", 500_000, ("1000", "1900")),
        a_run_of("2", 500_000, ("1900", "2000")),
    ]));
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

    // A thousand points over a million millionths, so half a million is 500 points. Both
    // runs are that, so the estimate is 500 and two runs to go on widen it by half. Were
    // the readings taken for the truth, the larger slice would make it 900.
    let started = execution.run(declaring("50%", "8h")).unwrap();

    assert_eq!(started.assigned.len(), 2);
    let held = tasks.load().unwrap();
    assert_eq!(held.tasks[0].ceiling.as_deref(), Some("750"));
    assert_eq!(held.tasks[1].ceiling.as_deref(), Some("750"));
}

/// Two runs the vendor priced alike are sized alike however far apart their token counts are.
///
/// What a token costs differs between models by several times over, so a rate taken over
/// tokens is out for any one model by that much. The price already carries the difference.
/// Here one run counted ten times the other and was priced the same; both are one size.
#[test]
fn two_runs_of_a_price_are_sized_alike_however_far_apart_their_counts_are() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger(Mutex::new(vec![
        a_run_costing("1", 1_000_000, 500_000, ("1000", "1900")),
        a_run_costing("2", 100_000, 500_000, ("1900", "2000")),
    ]));
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

    // The same thousand points over the same million millionths, so both runs are 500 and the
    // estimate is 500 widened by half. Counted instead, the two would be 909 and 90.
    let started = execution.run(declaring("50%", "8h")).unwrap();

    assert_eq!(started.assigned.len(), 2);
    let held = tasks.load().unwrap();
    assert_eq!(held.tasks[0].ceiling.as_deref(), Some("750"));
    assert_eq!(held.tasks[1].ceiling.as_deref(), Some("750"));
}

/// A run that crossed a window turning over says nothing about what a run costs.
///
/// What it spent before the window began again is in no reading at all, so the difference
/// either side of it is not what it cost. It is left out rather than counted low, and the
/// session is then one with a budget and nothing to go on: one task starts and is
/// measured, and the run after it has a figure again.
///
/// Once per window at most, since only a run that spans the turnover is like this.
#[test]
fn a_run_that_crossed_a_window_is_no_sample_at_all() {
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

    // The window turned over during that run, so it is no sample, and the session starts
    // one more to get one rather than stopping.
    let assigned = work.carry_on("task:1").unwrap();
    assert_eq!(assigned.len(), 1);
    assert_eq!(sessions.load().sessions[0].stopped_reason, None);
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

        // One starts alone, and the ones after it fill the machine.
        execution.run(declaring("4M", "8h")).unwrap();
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
/// One task here consumes 295,816 tokens and is allowed twice that, the estimate having
/// been worked out from one run. Two of those fit in what is left of two million and three
/// do not, so the budget binds before the machine's four does.
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
        execution.run(declaring("2M", "8h")).unwrap();
        work.carry_on("task:1").unwrap();
        assert_eq!(running_in(&tasks), 2);

        // Both of those end together, and another would put the session past its two million.
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
            started <= 6,
            "{started} tasks started against a budget for 6"
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

/// A decision is reached when a task ends, so a session with one long run going would pass
/// the time it declared with nobody looking.
#[test]
fn a_session_past_the_time_it_declared_takes_nothing_more_on_and_ends_nothing() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let opened = Outside {
        sessions: &sessions,
        tasks: &tasks,
        worktrees: &areas,
        agent: &agent,
        clock: &STILL,
        limit: &UNTOUCHED,
        traces: &NOTHING_KEPT,
        runs: &runs,
    };
    ExecutionService::new(opened, &Supervisor::new(opened, AT_ONCE))
        .run(declaring("2M", "8h"))
        .unwrap();
    assert_eq!(sessions.load().sessions[0].state, "running");

    // Eight hours on, with nothing having ended in between.
    let late = Frozen(1_000 + 8 * 3_600);
    let now = Supervisor::new(
        Outside {
            clock: &late,
            ..opened
        },
        AT_ONCE,
    );
    assert_eq!(now.time_left().unwrap(), Some(0));
    now.stop_if_out_of_time().unwrap();

    // Left alone, since a run is still going. The time declared is a deadline for taking work
    // on, and ending a run past it spends everything it spent for nothing.
    let session = sessions.load().sessions[0].clone();
    assert_eq!(session.state, "running");
    assert_eq!(agent.stopped.lock().unwrap().len(), 0);

    // It stops once that run has ended, at the decision the ending reaches.
    let work = WorkService::new(
        Outside {
            clock: &late,
            ..opened
        },
        &now,
    );
    let running = tasks.running()[0].clone();
    work.carry_on(&format!("task:{running}")).unwrap();

    let session = sessions.load().sessions[0].clone();
    assert_eq!(session.state, "stopped");
    assert_eq!(session.stopped_reason.as_deref(), Some("budget hardlock"));
    // And nothing was ended to get there.
    assert_eq!(agent.stopped.lock().unwrap().len(), 0);
}

/// Whoever waits on it may call it whenever, and this is what decides.
#[test]
fn a_session_with_time_left_is_left_alone() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
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

    assert_eq!(supervisor.time_left().unwrap(), Some(8 * 3_600));
    supervisor.stop_if_out_of_time().unwrap();

    assert_eq!(sessions.load().sessions[0].state, "running");
}

/// Nothing is running, so there is nothing to hold to a deadline and nothing to wait on.
#[test]
fn nothing_running_has_no_deadline_to_wait_on() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let supervisor = Supervisor::new(
        Outside {
            sessions: &sessions,
            tasks: &tasks,
            worktrees: &areas,
            agent: &agent,
            clock: &STILL,
            limit: &UNTOUCHED,
            traces: &NOTHING_KEPT,
            runs: &runs,
        },
        AT_ONCE,
    );

    assert_eq!(supervisor.time_left().unwrap(), None);
    supervisor.stop_if_out_of_time().unwrap();
}
