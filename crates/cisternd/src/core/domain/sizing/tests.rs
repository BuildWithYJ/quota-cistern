use super::*;

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
    let sized = |policy| Sizings::under(policy, runs()).model(Some("opus")).unwrap();

    let shipped = Sizings::under(Rule::default(), runs())
        .model(Some("opus"))
        .unwrap();
    let wider = sized(Rule {
        widen: 4,
        ..Rule::default()
    });
    let lower = sized(Rule {
        busy: 50,
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
