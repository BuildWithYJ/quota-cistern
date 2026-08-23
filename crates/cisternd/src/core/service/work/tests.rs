use std::sync::Mutex;

use crate::core::{
    domain::{HUNDREDTHS, SessionId},
    port::{
        inbound::{Carrying, ExecutionUseCase},
        outbound::{BacklogStore, Ended, Observed, Outcome, Spent},
    },
};

use super::super::fixtures::*;
use super::super::{ExecutionService, Outside, Supervisor, WorkService};
use super::*;

#[test]
fn a_task_that_was_carried_on_ends_completed_where_it_was_worked_on() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("50%", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

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
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("50%", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

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
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("50%", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    let held = tasks.first();
    assert_eq!(held.state, "Error");
    assert_eq!(held.consumed, None);
    assert_eq!(held.unreadable, None);
}

/// The agent answered with a count, and one figure in it is not a number.
#[test]
fn a_figure_that_does_not_read_as_a_number_is_not_a_count() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::ending(Ended {
        outcome: Outcome::Finished,
        reason: None,
        conversation: None,
        turns: None,
        observed: Observed::Spent(Spent {
            input: "a lot".to_owned(),
            output: "3377".to_owned(),
            cache_written: "28879".to_owned(),
            cache_read: "263483".to_owned(),
            cost: "92170".to_owned(),
        }),
    });
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("50%", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    assert_eq!(tasks.first().consumed, None);
    assert!(tasks.first().unreadable.is_some());
}

#[test]
fn an_agent_that_failed_leaves_the_task_in_error_with_what_it_said() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::ending(Ended {
        outcome: Outcome::Failed,
        reason: Some("it went wrong".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("50%", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

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
    let agent = Answering::ending(Ended {
        outcome: Outcome::AtCeiling,
        reason: Some("the agent was cut off after 200 turns".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

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
    let agent = Answering::ending(Ended {
        outcome: Outcome::Failed,
        reason: Some("it stopped".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let full = AtPercent {
        used: Mutex::new(100 * HUNDREDTHS),
        refuse: Mutex::new(false),
    };
    let runs = Ledger::default();
    let outside = Outside {
        limit: &full,
        ..stand_ins(&sessions, &tasks, &areas, &agent, &runs)
    };
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    let held = tasks.first();
    assert_eq!(held.state, "Pending");
    assert_eq!(held.reason, None);
    // The session it was assigned to stays. It paid for whatever the refused run got
    // through, and assigning the task again is what names the next one.
    assert_eq!(held.session.as_deref(), Some("1"));

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
    let agent = Answering::ending(Ended {
        outcome: Outcome::Failed,
        reason: Some("it went wrong".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let room = AtPercent {
        used: Mutex::new(40 * HUNDREDTHS),
        refuse: Mutex::new(false),
    };
    let runs = Ledger::default();
    let outside = Outside {
        limit: &room,
        ..stand_ins(&sessions, &tasks, &areas, &agent, &runs)
    };
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

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
    let agent = Answering::ending(Ended {
        outcome: Outcome::Failed,
        reason: Some("it went wrong".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let silent = AtPercent {
        used: Mutex::new(0),
        refuse: Mutex::new(true),
    };
    let runs = Ledger::default();
    let outside = Outside {
        limit: &silent,
        ..stand_ins(&sessions, &tasks, &areas, &agent, &runs)
    };
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    assert_eq!(tasks.first().state, "Error");
}

/// The executor is called for one task at a time and several at once.
/// Two running together have to end in their own places without either losing what the other recorded.
#[test]
fn two_tasks_carried_on_at_once_each_end_in_their_own_place() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    // The supervisor is what assigns more than one.
    // Until it exists, a test stands in for it by assigning the second task itself.
    execution.run(declaring("50%", "8h")).unwrap();
    let opened = SessionId::parse("1").unwrap();
    backlog::change(&tasks, |held| {
        let waiting = held.next_to_assign().unwrap();
        Ok(held.assign(waiting, opened, u64::MAX, 0, None))
    })
    .unwrap();

    let work = &work;
    std::thread::scope(|threads| {
        for task in ["task:1", "task:2"] {
            threads.spawn(move || work.carry_on(task).unwrap());
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
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("50%", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    let held = tasks.first();
    assert_eq!(held.state, "Error");
    assert_eq!(held.reason.as_deref(), Some("no such base branch"));
    assert!(agent.asked.lock().unwrap().is_empty());
}

/// The backlog keeps one run to a task, so what a budget is worked out from is kept apart.
/// A run that was cut off leaves the conversation it was in on the task, so a later run may
/// carry it on. A run that finished leaves none: the work is done and nobody carries it on.
#[test]
fn a_run_leaves_its_conversation_on_the_task_unless_it_finished() {
    let ran = |outcome| {
        let sessions = Remembered::empty();
        let tasks = Tasks::holding(vec![a_pending_task()]);
        let areas = Areas::default();
        let agent = Answering::ending(Ended {
            outcome,
            reason: None,
            conversation: Some("a-conversation".to_owned()),
            turns: None,
            observed: spending(),
        });
        let runs = Ledger::default();
        let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
        let supervisor = Supervisor::new(outside, AT_ONCE);
        ExecutionService::new(outside, &supervisor)
            .run(declaring("2M", "8h"))
            .unwrap();
        WorkService::new(outside, &supervisor)
            .carry_on("task:1")
            .unwrap();
        tasks.first().conversation
    };

    assert_eq!(ran(Outcome::AtCeiling).as_deref(), Some("a-conversation"));
    assert_eq!(ran(Outcome::Finished), None);
}

/// One word tells a person a run hit a ceiling. Which ceiling it was goes to the ledger.
///
/// A run held back by its turns and one held back by what it may spend say different things
/// about the task, and a figure worked out from runs cannot tell them apart from one word.
#[test]
fn a_run_cut_off_at_a_ceiling_tells_the_ledger_which_one() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::ending(Ended {
        outcome: Outcome::AtCeiling,
        reason: Some("the agent was cut off after 200 turns".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    ExecutionService::new(outside, &supervisor)
        .run(declaring("2M", "8h"))
        .unwrap();
    WorkService::new(outside, &supervisor)
        .carry_on("task:1")
        .unwrap();

    // The task is left with the one word section 1 gives it.
    assert_eq!(tasks.first().reason.as_deref(), Some("task ceiling"));

    let written = runs.runs();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].reason.as_deref(), Some("task ceiling"));
    assert_eq!(
        written[0].said.as_deref(),
        Some("the agent was cut off after 200 turns")
    );
}

#[test]
fn a_run_that_ended_is_written_to_the_ledger() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let ledger = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &ledger);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    let written = ledger.runs();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0].task, "1");
    assert_eq!(written[0].session.as_deref(), Some("1"));
    assert_eq!(written[0].outcome, "Completed");
    assert_eq!(
        written[0].spent.as_ref().map(|spent| spent.cost.as_str()),
        Some("92170")
    );
    assert_eq!(written[0].unreadable, None);
}

/// A refused run is still a run, and the session it was assigned to paid for it.
#[test]
fn a_run_the_vendor_refused_is_written_to_the_ledger_with_its_session() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::ending(Ended {
        outcome: Outcome::Failed,
        reason: Some("it stopped".to_owned()),
        conversation: None,
        turns: None,
        observed: spending(),
    });
    let full = AtPercent {
        used: Mutex::new(100 * HUNDREDTHS),
        refuse: Mutex::new(false),
    };
    let ledger = Ledger::default();
    let outside = Outside {
        limit: &full,
        ..stand_ins(&sessions, &tasks, &areas, &agent, &ledger)
    };
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    let written = ledger.runs();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0].task, "1");
    assert_eq!(written[0].session.as_deref(), Some("1"));
    assert_eq!(
        written[0].spent.as_ref().map(|spent| spent.cost.as_str()),
        Some("92170")
    );
}

/// What a run consumed may be unreadable, and that is not the same as nothing.
#[test]
fn a_run_nobody_could_read_is_written_to_the_ledger_as_unreadable() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::ending(Ended {
        outcome: Outcome::Finished,
        reason: None,
        conversation: None,
        turns: None,
        observed: Observed::Unreadable {
            why: "no last line".to_owned(),
        },
    });
    let ledger = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &ledger);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    work.carry_on("task:1").unwrap();

    let written = ledger.runs();
    assert_eq!(written.len(), 1, "{written:?}");
    assert_eq!(written[0].spent, None);
    assert_eq!(written[0].unreadable.as_deref(), Some("no last line"));
}

/// A task is put on the queue when it is assigned and reached when a worker gets to it.
///
/// The session can stop in between. Starting the task then spends against a session that
/// has already reported what it spent, and nothing would end the run afterwards.
#[test]
fn a_task_whose_session_stopped_before_a_worker_reached_it_does_not_start() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution.run(declaring("2M", "8h")).unwrap();
    // Everything the run assigned is interrupted, which is what a session stopping does.
    execution.interrupt().unwrap();
    let asked = agent.asked.lock().unwrap().len();

    let assigned = work.carry_on("task:1").unwrap();

    assert!(assigned.is_empty());
    assert_eq!(
        agent.asked.lock().unwrap().len(),
        asked,
        "a task was started for a session that had stopped"
    );
    assert!(
        runs.runs().is_empty(),
        "a run was written for a task that never started"
    );
}

/// What a run cost in the unit a share is declared in.
///
/// Tokens say what it cost in the other unit, and neither converts to the other on its
/// own. The two readings are ones the session already took, so they are written down
/// rather than asked for again.
#[test]
fn a_run_of_a_share_records_where_the_limit_stood_either_side_of_it() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    // Opens at 1000, and each look after that is 300 further on.
    let climbing = Advancing {
        used: Mutex::new(1_000),
        step: 300,
    };
    let runs = Ledger::default();
    let outside = Outside {
        limit: &climbing,
        ..stand_ins(&sessions, &tasks, &areas, &agent, &runs)
    };
    let supervisor = Supervisor::new(outside, AT_ONCE);

    ExecutionService::new(outside, &supervisor)
        .run(declaring("50%", "8h"))
        .unwrap();
    WorkService::new(outside, &supervisor)
        .carry_on("task:1")
        .unwrap();

    let written = runs.runs();
    assert_eq!(written.len(), 1, "{written:?}");
    let (before, after) = (
        written[0].limit_before.as_deref(),
        written[0].limit_after.as_deref(),
    );
    assert!(before.is_some() && after.is_some(), "{written:?}");
    assert_ne!(before, after, "the run cost nothing the limit could see");
}

/// When the reading that ends a run was taken, which is not when the run ended.
///
/// Reading the limit means putting a session in front of the vendor and waiting for its
/// status line. Whatever moved the limit over that stretch is in the figure, so a line that
/// says only what the figure was cannot say over how long it was gathered. With this, what
/// sits between two runs of a session is readable, and a stretch with nothing of ours going
/// is somebody else's doing.
#[test]
fn a_run_of_a_share_says_when_the_reading_that_ended_it_was_taken() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task(), a_second_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let climbing = Advancing {
        used: Mutex::new(1_000),
        step: 300,
    };
    let runs = Ledger::default();
    // A still clock cannot tell a moment taken before the vendor was asked from one taken
    // after, which is the whole of what this is about.
    let moving = Ticking(Mutex::new(1_000));
    let outside = Outside {
        clock: &moving,
        limit: &climbing,
        ..stand_ins(&sessions, &tasks, &areas, &agent, &runs)
    };
    let supervisor = Supervisor::new(outside, AT_ONCE);

    ExecutionService::new(outside, &supervisor)
        .run(declaring("50%", "8h"))
        .unwrap();
    WorkService::new(outside, &supervisor)
        .carry_on("task:1")
        .unwrap();

    let written = runs.runs();
    assert_eq!(written.len(), 1, "{written:?}");
    let read_at: u64 = written[0]
        .limit_after_at
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();
    let ended_at: u64 = written[0].ended_at.parse().unwrap();
    assert!(
        read_at > ended_at,
        "the reading was dated before the run it ended: {written:?}"
    );
}

/// A session declared in tokens never asks how far the limit is spent, so a run of one has
/// no reading either side of it.
#[test]
fn a_run_of_a_count_records_no_reading_at_all() {
    let sessions = Remembered::empty();
    let tasks = Tasks::holding(vec![a_pending_task()]);
    let areas = Areas::default();
    let agent = Answering::finishing();
    let runs = Ledger::default();
    let outside = stand_ins(&sessions, &tasks, &areas, &agent, &runs);
    let supervisor = Supervisor::new(outside, AT_ONCE);

    ExecutionService::new(outside, &supervisor)
        .run(declaring("2M", "8h"))
        .unwrap();
    WorkService::new(outside, &supervisor)
        .carry_on("task:1")
        .unwrap();

    let written = runs.runs();
    assert_eq!(written[0].limit_before, None, "{written:?}");
    assert_eq!(written[0].limit_after, None, "{written:?}");
    assert_eq!(written[0].limit_after_at, None, "{written:?}");
}
