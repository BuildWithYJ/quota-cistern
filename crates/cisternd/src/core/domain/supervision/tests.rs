use crate::core::domain::Span;

use super::*;

fn declaring(usage: Usage) -> Budget {
    Budget {
        usage,
        time: Span::parse("8h").unwrap(),
    }
}

fn task(n: u32) -> TaskId {
    TaskId::parse(&n.to_string()).unwrap()
}

/// A session with room, nothing wrong, one task running, and two waiting.
fn standing() -> Standing {
    Standing {
        left: 1_000,
        booked: 0,
        sizings: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 100)]),
        // Nothing has been timed, so the clock lets everything start. The tests that are
        // about the clock say what runs have taken.
        lasting: Sizings::default(),
        time_left: 8 * 60 * 60,
        pending: vec![
            (task(1), Some("opus".to_owned())),
            (task(2), Some("opus".to_owned())),
        ],
        running: 1,
        blocked: false,
        out_of_time: false,
        unreadable: false,
        at_once: 4,
    }
}

fn ceilings(decision: Decision) -> Vec<u64> {
    match decision {
        Decision::Start(allowed) => allowed.iter().map(|one| one.ceiling).collect(),
        Decision::Stop(why) => panic!("stopped for {why}"),
    }
}

// what a decision comes to

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
            out_of_time: true,
            running: 1,
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

/// And it stops once what it had going has ended.
#[test]
fn a_session_out_of_time_with_nothing_going_stops_with_budget_left() {
    assert_eq!(
        decide(&Standing {
            out_of_time: true,
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
            left: 0,
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

#[test]
fn a_session_with_no_room_left_starts_none_while_one_runs() {
    assert_eq!(
        decide(&Standing {
            left: 0,
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
            left: 500,
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
            left: 500,
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
            sizings: Sizings::default(),
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
            sizings: Sizings::default(),
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
            left: 110,
            running: 0,
            sizings: Sizings::under(
                Rule::default(),
                [
                    Ran::finished(Some("opus"), 50),
                    Ran::finished(Some("opus"), 100),
                ]
            ),
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
            left: 40,
            running: 0,
            sizings: Sizings::under(
                Rule::default(),
                [
                    Ran::finished(Some("opus"), 50),
                    Ran::finished(Some("opus"), 100),
                ]
            ),
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
            lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
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
            lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
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
            lasting: Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 900)]),
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
            lasting: Sizings::under(Rule::default(), [Ran::finished(Some("haiku"), 900)]),
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
            left: 60,
            sizings: Sizings::under(
                Rule::default(),
                [
                    Ran::finished(Some("opus"), 50),
                    Ran::finished(Some("opus"), 100),
                ]
            ),
            ..standing()
        }),
        Decision::Start(Vec::new())
    );
}

#[test]
fn no_more_than_a_handful_start_however_large_the_budget() {
    assert_eq!(
        ceilings(decide(&Standing {
            left: 1_000_000,
            running: 0,
            pending: (1..=10)
                .map(|n| (task(n), Some("opus".to_owned())))
                .collect(),
            ..standing()
        }))
        .len(),
        4
    );
}

// what a model's runs have cost

/// The middle of what one model's runs cost differs from another's by more than a factor
/// of five, so a session running one model against a figure taken over all of them is
/// measured against a size nothing it runs is.
#[test]
fn each_model_is_worked_out_from_its_own_runs() {
    let sizings = Sizings::under(
        Rule::default(),
        [
            Ran::finished(Some("haiku"), 10),
            Ran::finished(Some("haiku"), 20),
            Ran::finished(Some("opus"), 300),
            Ran::finished(Some("opus"), 400),
        ],
    );

    assert_eq!(sizings.model(Some("haiku")).unwrap().estimate, 20);
    assert_eq!(sizings.model(Some("opus")).unwrap().estimate, 400);
    assert_eq!(sizings.model(Some("sonnet")), None);
}

/// A task that named no model is answered by the vendor's own default, which is a size of
/// its own rather than any of the named ones.
#[test]
fn runs_that_named_no_model_are_their_own() {
    let sizings = Sizings::under(
        Rule::default(),
        [Ran::finished(None, 7), Ran::finished(Some("opus"), 900)],
    );

    assert_eq!(sizings.model(None).unwrap().estimate, 7);
    assert_eq!(sizings.model(Some("opus")).unwrap().estimate, 900);
}

/// Read between two runs rather than at the nearer of them.
///
/// The kth of n sorted runs sits at k/(n+1) of what they were drawn from, so the nearer
/// rank answers a question it was not asked: four runs asked for their 75th return the
/// third, which is the 60th, and a ceiling there stops two runs in five rather than one in
/// four. Four runs asked for their 95th have nothing above the largest to offer, and the one
/// they fall back to falls between the third and the fourth.
#[test]
fn the_figures_are_read_between_the_runs() {
    let sizings = Sizings::under(
        Rule::default(),
        [
            Ran::finished(Some("opus"), 100),
            Ran::finished(Some("opus"), 200),
            Ran::finished(Some("opus"), 300),
            Ran::finished(Some("opus"), 400),
        ],
    );
    let sizing = sizings.model(Some("opus")).unwrap();

    assert_eq!(
        (sizing.estimate, sizing.fallback, sizing.over),
        (358, 141, 4)
    );
}

/// Until there are runs enough for a figure to sit under the largest of them, the estimate
/// is the largest. Thirteen runs is where it starts pulling in, and a session works from a
/// handful, so what a ceiling comes to in practice is the dearest run of that model yet.
#[test]
fn the_estimate_only_falls_under_the_dearest_run_once_there_are_runs_enough() {
    let costs = |over: u64| {
        Sizings::under(
            Rule::default(),
            (1..=over).map(|each| Ran::finished(Some("opus"), each * 100)),
        )
        .model(Some("opus"))
        .unwrap()
        .estimate
    };

    assert_eq!((costs(4), costs(8), costs(13)), (358, 658, 1_033));
}

/// A run stopped at its ceiling spent what it was stopped at. Counting that as what the
/// task costs would pull the estimate down toward the ceiling that stopped it, and the
/// lower estimate would stop the next run sooner.
/// The numbers are the rule's rather than the module's, so a sweep hands in another one
/// instead of being built again for each.
#[test]
fn a_rule_of_someones_own_is_what_a_sizing_is_worked_out_by() {
    let runs = || (1..=4).map(|each| Ran::finished(Some("opus"), each * 100));
    let sized = |rule| Sizings::under(rule, runs()).model(Some("opus")).unwrap();

    let shipped = Sizings::under(Rule::default(), runs())
        .model(Some("opus"))
        .unwrap();
    let wider = sized(Rule {
        widen: 4,
        ..Rule::default()
    });
    let lower = sized(Rule {
        estimate: 50,
        ..Rule::default()
    });

    assert_eq!((shipped.estimate, shipped.allowing()), (358, 447));
    assert_eq!((wider.estimate, wider.allowing()), (358, 716));
    assert_eq!((lower.estimate, lower.allowing()), (250, 312));
}

#[test]
fn a_run_that_was_stopped_is_not_counted_as_what_its_task_costs() {
    let sizings = Sizings::under(
        Rule::default(),
        [
            Ran::finished(Some("opus"), 300),
            Ran::finished(Some("opus"), 400),
            Ran::stopped(Some("opus"), 20),
        ],
    );
    let sizing = sizings.model(Some("opus")).unwrap();

    assert_eq!(
        (sizing.estimate, sizing.fallback, sizing.over),
        (400, 300, 2)
    );
}

/// It was still working when it was stopped, so its task takes more than that. Holding the
/// estimate at where it stopped would stop the next run at the same place and stay there.
#[test]
fn a_run_that_was_stopped_lifts_the_estimate_past_where_it_stopped() {
    let sizings = Sizings::under(
        Rule::default(),
        [
            Ran::finished(Some("opus"), 300),
            Ran::finished(Some("opus"), 400),
            Ran::stopped(Some("opus"), 900),
        ],
    );
    let sizing = sizings.model(Some("opus")).unwrap();

    assert_eq!((sizing.estimate, sizing.over), (1_800, 2));
}

/// Nothing has finished, so there is nothing to size from. One task then starts with the
/// whole of what is left, which is more room than the floor would have given it.
#[test]
fn a_model_that_has_only_been_stopped_has_no_figure() {
    let sizings = Sizings::under(Rule::default(), [Ran::stopped(Some("opus"), 900)]);

    assert_eq!(sizings.model(Some("opus")), None);
}

#[test]
fn one_run_is_both_figures() {
    let sizings = Sizings::under(Rule::default(), [Ran::finished(Some("opus"), 42)]);
    let sizing = sizings.model(Some("opus")).unwrap();

    assert_eq!((sizing.estimate, sizing.fallback, sizing.over), (42, 42, 1));
}

// the budget itself

#[test]
fn a_share_is_shown_as_the_percentage_it_was_declared_in() {
    assert_eq!(Spending::Share(400).to_string(), "4%");
    assert_eq!(Spending::Share(350).to_string(), "3.5%");
    assert_eq!(Spending::Share(405).to_string(), "4.05%");
    assert_eq!(Spending::Share(0).to_string(), "0%");
}

#[test]
fn a_count_is_shown_as_the_count() {
    assert_eq!(Spending::Tokens(2_000_000).to_string(), "2000000");
}

#[test]
fn what_is_left_is_what_was_declared_less_what_was_spent() {
    let budget = declaring(Usage::Tokens(1_000));
    assert_eq!(budget.left(Spending::Tokens(400)), 600);
}

/// A share is declared in whole percent and measured in hundredths.
#[test]
fn a_share_is_left_over_in_hundredths_of_a_percent() {
    let budget = declaring(Usage::Share(50));
    assert_eq!(budget.left(Spending::Share(2_000)), 3_000);
}

/// A session can pass its budget between two decisions.
/// A decision is made when a task ends and not while one runs.
#[test]
fn spending_more_than_was_declared_leaves_nothing() {
    let budget = declaring(Usage::Tokens(1_000));
    assert_eq!(budget.left(Spending::Tokens(4_000)), 0);
}

#[test]
fn nothing_is_left_when_the_unit_is_not_the_one_declared() {
    let budget = declaring(Usage::Share(50));
    assert_eq!(budget.left(Spending::Tokens(1)), 0);
}

#[test]
fn spending_more_of_a_share_than_was_declared_leaves_nothing() {
    let budget = declaring(Usage::Share(1));
    assert_eq!(budget.left(Spending::Share(4_000)), 0);
}

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
