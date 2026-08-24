use crate::core::domain::sizing::{Ran, Rule};
use crate::core::domain::{Before, Locking, Pacing, Sizings};

use super::*;

fn task(n: u32) -> TaskId {
    TaskId::parse(&n.to_string()).unwrap()
}

/// A session with room, nothing wrong, one task running, and two waiting.
fn standing() -> Standing {
    Standing {
        declared: 1_000,
        // Nothing spent and no time gone, so the rate the budget is going at says nothing and
        // the gate that reads it holds nothing back. The tests that are about that gate say
        // what has been spent.
        spent: 0,
        elapsed: 0,
        booked: 0,
        before: Before {
            cost: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 100)]),
            // Nothing has been timed, so the clock lets everything start. The tests that are
            // about the clock say what runs have taken.
            lasting: Sizings::default(),
        },
        time_left: 8 * 60 * 60,
        pending: vec![
            (task(1), Some("opus".to_owned())),
            (task(2), Some("opus".to_owned())),
        ],
        running: 1,
        blocked: false,
        unreadable: false,
        timing: Timing::Fits,
        locking: Locking::Cuts,
        pacing: Pacing::Holds,
    }
}

fn ceilings(decision: Decision) -> Vec<u64> {
    match decision {
        Decision::Start(allowed) => allowed.iter().map(|one| one.ceiling).collect(),
        Decision::Stop(why) => panic!("stopped for {why}"),
    }
}

// what a decision comes to

/// The policy is a value, so a session run under another one is another value rather than
/// another build. And the clock is part of it: what a session does about a run it may not
/// have time for is a choice, and nothing so far says which choice is right.
#[test]
fn a_policy_of_someones_own_is_what_a_session_is_run_under() {
    let out_of_reach = || Standing {
        before: Before {
            lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 9_000)]),
            ..standing().before
        },
        time_left: 100,
        running: 0,
        ..standing()
    };

    assert_eq!(
        decide(&out_of_reach()),
        Decision::Stop(StoppedReason::BudgetHardlock)
    );
    assert_eq!(
        ceilings(decide(&Standing {
            timing: Timing::Any,
            ..out_of_reach()
        }))
        .len(),
        2
    );
}

#[test]
fn a_session_with_room_and_something_waiting_starts_more() {
    assert_eq!(ceilings(decide(&standing())), [200, 200]);
}

#[test]
fn a_consumption_nobody_could_read_stops_it() {
    assert_eq!(
        decide(&Standing {
            unreadable: true,
            ..standing()
        }),
        Decision::Stop(StoppedReason::ObservationUnreadable)
    );
}

/// Time is the other half of the budget, and it runs out with money left.
///
/// What it stops is taking more work on. A run already going is one whose length we guessed
/// short, and ending it there spends everything it spent for nothing.
#[test]
fn a_session_out_of_time_takes_nothing_more_on() {
    assert_eq!(
        decide(&Standing {
            time_left: 0,
            running: 1,
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

/// A backlog that emptied is done, and the clock does not make it anything else.
///
/// The reason reaches a person. Saying the budget locked where the work simply finished is
/// wrong about the budget and about the work.
#[test]
fn a_session_out_of_time_with_nothing_left_to_do_is_all_done() {
    assert_eq!(
        decide(&Standing {
            time_left: 0,
            running: 0,
            pending: Vec::new(),
            ..standing()
        }),
        Decision::Stop(StoppedReason::AllDone)
    );
}

/// And it stops once what it had going has ended.
#[test]
fn a_session_out_of_time_with_nothing_going_stops_with_budget_left() {
    assert_eq!(
        decide(&Standing {
            time_left: 0,
            running: 0,
            ..standing()
        }),
        Decision::Stop(StoppedReason::BudgetHardlock)
    );
}

/// Nothing running, tasks waiting, and no room for any of them.
#[test]
fn nothing_running_and_no_room_is_the_end_of_the_budget() {
    assert_eq!(
        decide(&Standing {
            declared: 0,
            running: 0,
            ..standing()
        }),
        Decision::Stop(StoppedReason::BudgetHardlock)
    );
}

#[test]
fn nothing_running_and_nothing_waiting_is_all_done() {
    assert_eq!(
        decide(&Standing {
            running: 0,
            pending: Vec::new(),
            ..standing()
        }),
        Decision::Stop(StoppedReason::AllDone)
    );
}

/// A task still running may yet leave room, so the session waits for it.
#[test]
fn a_session_with_nothing_waiting_carries_on_while_one_runs() {
    assert_eq!(
        decide(&Standing {
            pending: Vec::new(),
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

/// What is left is spoken for by the run already going, so nothing more starts and the session
/// waits for it. The budget is not spent, only handed out, which is a different thing from the
/// figure running out.
#[test]
fn a_session_with_no_room_left_starts_none_while_one_runs() {
    assert_eq!(
        decide(&Standing {
            booked: 1_000,
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

// what each task is allowed

/// Every task that starts leaves with a figure it may not pass, worked out from what runs
/// of its model have cost and widened by how few of them there were. One run of 100 here,
/// so the figure is twice it.
#[test]
fn a_task_is_allowed_more_than_runs_of_its_model_have_cost() {
    assert_eq!(ceilings(decide(&standing())), [200, 200]);
}

/// What is left less what the runs already going are allowed. Whatever any of them
/// actually spends, together they cannot pass what the session declared.
#[test]
fn what_is_already_allowed_is_held_against_the_budget() {
    assert_eq!(
        ceilings(decide(&Standing {
            declared: 500,
            booked: 200,
            ..standing()
        })),
        [200],
        "300 was left, which covers one of these and not two"
    );
}

/// The list is taken in order until what is left will not cover the next one.
///
/// The leftover is not handed out. A run allowed less than its model usually takes is a
/// run that will be cut off, and a task waiting is worth more than a branch nobody can
/// use. Something is running, so room may yet come back.
#[test]
fn what_is_left_runs_out_partway_down_the_list() {
    assert_eq!(
        ceilings(decide(&Standing {
            declared: 500,
            pending: vec![
                (task(1), Some("opus".to_owned())),
                (task(2), Some("opus".to_owned())),
                (task(3), Some("opus".to_owned())),
            ],
            ..standing()
        })),
        [200, 200]
    );
}

/// A model nothing has been run with has no figure to go on. One task starts with the
/// whole of what is left, and what it costs is what the next decision is made from.
#[test]
fn with_nothing_to_go_on_one_task_starts_with_the_whole_of_it() {
    assert_eq!(
        ceilings(decide(&Standing {
            before: Before {
                cost: Sizings::default(),
                ..standing().before
            },
            running: 0,
            ..standing()
        })),
        [1_000]
    );
}

/// And nothing more starts while that one runs, since it is the sample.
#[test]
fn with_nothing_to_go_on_and_one_running_nothing_starts() {
    assert_eq!(
        decide(&Standing {
            before: Before {
                cost: Sizings::default(),
                ..standing().before
            },
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

/// Room for the middle of what this model costs but not for three runs in four of it.
/// More than half of what might start would finish inside what is left, which is worth
/// more than stopping a session that still has budget.
#[test]
fn a_session_with_nothing_running_lowers_its_bar_to_what_a_cheap_run_costs() {
    assert_eq!(
        ceilings(decide(&Standing {
            // Under a whole reservation, over what a run of this model in four comes in under.
            declared: 110,
            running: 0,
            before: Before {
                cost: Sizings::under(
                    Rule::default(),
                    [
                        Ran::finished(Some("opus"), 50),
                        Ran::finished(Some("opus"), 100),
                    ]
                ),
                ..standing().before
            },
            ..standing()
        })),
        [110]
    );
}

/// Not even a cheap run of it fits, and nothing is running that would make room.
#[test]
fn a_session_that_cannot_cover_a_cheap_run_stops() {
    assert_eq!(
        decide(&Standing {
            declared: 40,
            running: 0,
            before: Before {
                cost: Sizings::under(
                    Rule::default(),
                    [
                        Ran::finished(Some("opus"), 50),
                        Ran::finished(Some("opus"), 100),
                    ]
                ),
                ..standing().before
            },
            ..standing()
        }),
        Decision::Stop(StoppedReason::BudgetHardlock)
    );
}

/// The hardlock stops a session part way through whatever it has going, so a task started
/// with less time than a run of its model takes is one that will be stopped like that,
/// having spent what it spent and left nothing.
#[test]
fn a_task_that_cannot_finish_in_the_time_left_does_not_start() {
    assert_eq!(
        decide(&Standing {
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
                ..standing().before
            },
            time_left: 600,
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

#[test]
fn a_task_that_fits_the_time_left_starts() {
    assert_eq!(
        ceilings(decide(&Standing {
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
                ..standing().before
            },
            time_left: 3_600,
            ..standing()
        })),
        [200, 200]
    );
}

/// Waiting out the clock with tasks that may not start is not carrying on, and starting one
/// of them would spend on work the hardlock takes away again.
#[test]
fn a_session_with_nothing_running_and_no_time_for_another_run_stops() {
    assert_eq!(
        decide(&Standing {
            running: 0,
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
                ..standing().before
            },
            time_left: 600,
            ..standing()
        }),
        Decision::Stop(StoppedReason::BudgetHardlock)
    );
}

/// A model nothing has been timed on is not held to a clock it has no figure for. The first
/// run of it is the first sample, as it is for what a run costs.
#[test]
fn a_model_nothing_has_been_timed_on_starts_whatever_the_time_left() {
    assert_eq!(
        ceilings(decide(&Standing {
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("haiku"), 900)]),
                ..standing().before
            },
            time_left: 1,
            ..standing()
        })),
        [200, 200]
    );
}

/// The bar is only lowered with nothing running. A session with a run going has that run
/// to learn from and room may yet come back.
#[test]
fn the_bar_is_not_lowered_while_something_runs() {
    assert_eq!(
        decide(&Standing {
            declared: 60,
            before: Before {
                cost: Sizings::under(
                    Rule::default(),
                    [
                        Ran::finished(Some("opus"), 50),
                        Ran::finished(Some("opus"), 100),
                    ]
                ),
                ..standing().before
            },
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

/// What is left is what was declared less what was spent, and nothing once more was spent
/// than declared. A session can pass its figure between two decisions, since a decision is
/// reached when a task ends and not while one runs.
#[test]
fn what_is_left_is_what_was_declared_less_what_was_spent() {
    let standing = |spent| Standing {
        declared: 1_000,
        spent,
        ..standing()
    };
    assert_eq!(standing(400).left(), 600);
    assert_eq!(standing(4_000).left(), 0);
}

/// A run added to others that the budget will not outlast is not started.
///
/// The session has spent 900 of 1000 in an hour, so the last hundred lasts six minutes at that
/// rate. A run of this model takes ten, and adding it to the one already going would have both
/// cut off with what each did since its last commit lost.
#[test]
fn a_run_the_budget_will_not_outlast_is_not_added_to_the_others() {
    assert_eq!(
        decide(&Standing {
            spent: 900,
            elapsed: 3_600,
            running: 1,
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 600)]),
                ..standing().before
            },
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

/// The same session with nothing going starts it anyway.
///
/// One run ending at the budget is a session spending what it declared and keeping what that
/// run committed. Holding it back leaves the budget unspent and nothing done, which is the
/// worse of the two.
#[test]
fn a_session_with_nothing_going_starts_one_whatever_the_rate() {
    assert!(matches!(
        decide(&Standing {
            spent: 900,
            elapsed: 3_600,
            running: 0,
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 600)]),
                ..standing().before
            },
            ..standing()
        }),
        Decision::Start(allowed) if !allowed.is_empty()
    ));
}

/// The same session early on, where the same rate leaves the budget lasting longer than the
/// run. A rate is not a reason on its own; what it is asked is whether this run outlives the
/// budget.
#[test]
fn the_same_rate_early_on_starts_the_same_run() {
    assert!(matches!(
        decide(&Standing {
            spent: 100,
            elapsed: 400,
            running: 0,
            before: Before {
                lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 600)]),
                ..standing().before
            },
            ..standing()
        }),
        Decision::Start(allowed) if !allowed.is_empty()
    ));
}

/// A backlog that emptied is done, whether or not the budget emptied at the same moment.
///
/// The stop that ends runs is only for a session with runs to end. With none, what to call the
/// stopping is the same question it always was, and saying the budget locked where the work
/// simply finished is wrong about both.
#[test]
fn a_session_that_finished_as_the_budget_did_is_all_done() {
    assert_eq!(
        decide(&Standing {
            locking: Locking::Cuts,
            spent: 1_000,
            elapsed: 600,
            running: 0,
            pending: Vec::new(),
            ..standing()
        }),
        Decision::Stop(StoppedReason::AllDone)
    );
}

/// A session that has spent what it declared stops, and `Cuts` stops it while runs are going.
///
/// What a person declared is a figure. `Waits` is the other answer: the runs finish and the
/// session spends past it by however far they had to go.
#[test]
fn a_session_that_spent_what_it_declared_cuts_what_is_going() {
    let over = || Standing {
        spent: 1_000,
        elapsed: 600,
        running: 2,
        ..standing()
    };
    assert_eq!(
        decide(&Standing {
            locking: Locking::Cuts,
            ..over()
        }),
        Decision::Stop(StoppedReason::BudgetHardlock)
    );
    assert_eq!(
        decide(&Standing {
            locking: Locking::Waits,
            ..over()
        }),
        Decision::Start(Vec::new())
    );
}

#[test]
fn what_the_budget_covers_is_what_starts_however_many_wait() {
    let starting = |declared| {
        ceilings(decide(&Standing {
            declared,
            running: 0,
            pending: (1..=10)
                .map(|n| (task(n), Some("opus".to_owned())))
                .collect(),
            ..standing()
        }))
        .len()
    };

    // The default standing sizes a run of this model at 200, so a budget of a million covers
    // every task waiting and one of five hundred covers two.
    assert_eq!(starting(1_000_000), 10);
    assert_eq!(starting(500), 2);
}

// what a model's runs have cost

/// Tasks are left and every one of them waits on one that did not complete.
///
/// A ceiling makes this ordinary: a task cut off at one ends `Interrupted`, and a task
/// that waits on it may never start. Reporting it as everything being done says the
/// opposite of what happened.
#[test]
fn tasks_left_that_none_may_start_is_not_everything_being_done() {
    assert_eq!(
        decide(&Standing {
            running: 0,
            pending: Vec::new(),
            blocked: true,
            ..standing()
        }),
        Decision::Stop(StoppedReason::Blocked)
    );
}
