//! What runs have cost, worked out from the runs there have been.
//!
//! The one place a figure about the future comes from, and the only place in the core that
//! guesses. A run's own report is exact for that run; what the next run of the same model will
//! take is read off the ones before it, by the model that ran them, since what one model's
//! runs cost differs from another's by several times over.
//!
//! Nothing here reaches outside. A ledger arrives as figures already read.

use std::collections::BTreeMap;

use super::TaskState;

/// What runs of one model have cost.
///
/// Two figures: one for while others are going and one for when none is. A session is
/// conservative in the first place and optimistic in the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizing {
    /// What to set aside while other runs are going.
    ///
    /// Never under twice what a run of this model was stopped at: a run held to a ceiling was
    /// still working, so its task takes more than that, and a figure set where it stopped
    /// would stop the next one in the same place forever.
    pub estimate: u64,
    /// What to set aside when nothing else is going.
    pub fallback: u64,
    /// How many finished runs these were worked out from.
    pub over: usize,
    /// How far to widen the estimate, from the rule these were worked out under.
    ///
    /// Carried here rather than asked for again, so that a sizing answers what it allows
    /// without whoever holds it having to remember which rule made it.
    pub widen: u64,
}
impl Sizing {
    /// What to set aside for a run of this model, widened by how few runs it came from:
    /// `estimate x (1 + widen/over)`.
    pub fn allowing(&self) -> u64 {
        self.estimate.saturating_add(
            self.estimate
                .saturating_mul(self.widen)
                .saturating_div(self.over.max(1) as u64),
        )
    }
}
/// What one run cost, and whether that figure is what its task takes.
///
/// A run stopped at its ceiling spent what it was stopped at, which is a floor under what its
/// task takes rather than a measure of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    model: Option<String>,
    spent: u64,
    stopped: bool,
}
impl Ran {
    /// A run that did the work, which cost what its task takes.
    pub fn finished(model: Option<&str>, spent: u64) -> Self {
        Self {
            model: model.map(str::to_owned),
            spent,
            stopped: false,
        }
    }

    /// A run held to a ceiling, which says only that its task takes at least this much.
    pub fn stopped(model: Option<&str>, spent: u64) -> Self {
        Self {
            model: model.map(str::to_owned),
            spent,
            stopped: true,
        }
    }
}
/// What runs have cost, by the model that ran them.
///
/// A model nothing has finished a run with has nothing here. A session that meets one starts
/// a single task and measures it rather than guessing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sizings {
    by_model: BTreeMap<String, Sizing>,
    /// Runs of a task that named no model, which the vendor answered with its own default.
    unnamed: Option<Sizing>,
}
impl Sizings {
    /// Works the figures out from what each run cost, keeping the models apart.
    ///
    /// Split by model, since what one model's runs cost differs from another's by several
    /// times over. Runs that were stopped lift the estimate rather than being averaged into
    /// it, and a model with nothing but stopped runs has no figure at all.
    pub fn under(rule: Rule, runs: impl IntoIterator<Item = Ran>) -> Self {
        let mut apart: BTreeMap<Option<String>, (Vec<u64>, u64)> = BTreeMap::new();
        for run in runs {
            let (finished, floor) = apart.entry(run.model).or_default();
            match run.stopped {
                false => finished.push(run.spent),
                true => *floor = (*floor).max(run.spent),
            }
        }
        let mut sizings = Sizings::default();
        for (model, (mut costs, floor)) in apart {
            if costs.is_empty() {
                continue;
            }
            costs.sort_unstable();
            let sizing = Sizing {
                estimate: at(&costs, rule.busy).max(floor.saturating_mul(rule.lift)),
                fallback: at(&costs, rule.alone),
                over: costs.len(),
                widen: rule.widen,
            };
            match model {
                Some(model) => {
                    sizings.by_model.insert(model, sizing);
                }
                None => sizings.unnamed = Some(sizing),
            }
        }
        sizings
    }

    /// What a run of this model has cost, or nothing where none has.
    pub fn model(&self, model: Option<&str>) -> Option<Sizing> {
        match model {
            Some(model) => self.by_model.get(model).copied(),
            None => self.unnamed,
        }
    }
}
/// The value at a quantile of a sorted list, by the rule Hyndman and Fan recommend.
///
/// `h = (n + 1/3)p + 1/3`, read between the two values it falls between and held inside the
/// list at either end. The figure it gives is as often above the quantile it was asked for as
/// below it, whatever shape the values were drawn from.
///
/// Which rule is used matters at these sizes. Taking the nearer rank asks four runs for their
/// 75th and returns the third of them, and the kth of n sorted values sits at k/(n+1) of
/// whatever they came from, so the third of four is the 60th. A ceiling set there stops two
/// runs in five rather than the one in four it was asked for. That is arithmetic about ranks
/// rather than an assumption about the shape: the position of an order statistic follows
/// Beta(k, n - k + 1) for any continuous distribution, whose mean is k/(n+1).
///
/// Held whole throughout. `h` is carried in three-hundredths, which is exact for a quantile
/// given in whole percent, and the reading between two values rounds down: a ceiling that
/// lands under a figure some run cost is the safer way to be a fraction out.
fn at(sorted: &[u64], per_cent: u64) -> u64 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return 0;
    };
    let over = 300 * sorted.len() as u64;
    let h = 3 * per_cent * sorted.len() as u64 + per_cent + 100;
    if h <= 300 {
        return sorted[0];
    }
    if h >= over {
        return sorted[last];
    }
    // Between 1 and n - 1 inclusive, since h is over 300 and under 300n.
    let under = (h / 300) as usize;
    sorted[under - 1] + (sorted[under] - sorted[under - 1]) * (h % 300) / 300
}

/// Which quantile a run is sized at while others are going, in whole percent.
///
/// The third quarter. Others are going, so a run that goes over eats budget they were counting
/// on, and a size three runs in four come in under is far enough up to make that rare.
const BUSY: u64 = 75;

/// Which quantile a run is sized at when nothing else is going, in whole percent.
///
/// The first quarter. With nothing else going there is nobody to take budget from, so a session
/// that would otherwise stop with budget in hand starts one more and is optimistic about it.
/// This is the only place a session is.
const ALONE: u64 = 25;

/// How far a stopped run lifts the estimate above where it was stopped.
///
/// Twice, which is how a backfilling scheduler grows a prediction its job has already outlived.
const LIFT: u64 = 2;

/// How far an estimate is widened for how little it was worked out from.
///
/// One, so an estimate from a single run allows twice it and one from four allows a quarter
/// more. What this should be is not something four sessions on a real repository could say:
/// none of their runs came within half of its ceiling, so any figure here would have ended them
/// the same way. It is a number to sweep rather than one to argue about.
const WIDEN: u64 = 1;

/// The figures a sizing is worked out by.
///
/// Held apart from the figure the clock is asked for, because these four are read here and that
/// one is read where a decision is made. A rule that took the whole of `Policy` would be taking
/// three fields it never looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// Which quantile a run is sized at while others are going.
    pub busy: u64,
    /// Which quantile it is sized at when nothing else is going.
    pub alone: u64,
    /// How far the size is widened for how few runs it came from: `size x (1 + widen/over)`.
    pub widen: u64,
    /// How far a run that was stopped lifts the size above what it spent. Nothing leaves
    /// stopped runs out altogether.
    pub lift: u64,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            busy: BUSY,
            alone: ALONE,
            widen: WIDEN,
            lift: LIFT,
        }
    }
}

/// Which kind of sample a run is, or nothing where it is neither.
///
/// A run that finished says what its task takes. A run stopped at its ceiling says where it was
/// stopped. A run that failed or that the vendor turned away says neither: it spent what it
/// spent before it went wrong, which is neither a measure nor a floor.
pub fn sampled(outcome: TaskState, at_ceiling: bool) -> Option<fn(Option<&str>, u64) -> Ran> {
    match (outcome, at_ceiling) {
        (TaskState::Completed, _) => Some(Ran::finished),
        (TaskState::Interrupted, true) => Some(Ran::stopped),
        _ => None,
    }
}

/// What one run says about the two units a session may be declared in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Priced {
    /// What the vendor's limit read when the run before this one ended and when this one did,
    /// where the session took both. A session declared in tokens takes neither.
    pub over: Option<(u64, u64)>,
    /// What the vendor priced the run at, in millionths of its currency.
    pub priced: u64,
}

/// How far the vendor's limit moved for every millionth the runs were priced at, as a pair to
/// multiply and divide by rather than a fraction to round.
///
/// A run's own share of the limit cannot be read off the limit. The vendor keeps one figure for
/// the account, so two runs going at once move it together and no reading tells them apart;
/// taking a reading when each run ends splits the movement into stretches of time, and the run
/// that ends first is handed whatever the others spent while it ran. What a run reported for
/// itself is the only per-run figure there is.
///
/// So a run's size in the unit a share is declared in is what it was priced at, at the rate the
/// whole ledger moved the limit. The rate is right even where the split was not: the total
/// movement is the total movement however it is divided among the runs that caused it.
///
/// Priced rather than counted, because a token of one model is not a token of another. A rate
/// taken over tokens is out for any one model by however much its tokens cost more or less than
/// the rest of the ledger's, and the vendor has already told us that much in the price. What is
/// left over is whatever the limit weighs differently from the price, which is the smaller
/// question and the one there is no answer to here.
pub fn moved_per_millionth(runs: impl IntoIterator<Item = Priced>) -> (u64, u64) {
    let (mut moved, mut priced) = (0u64, 0u64);
    for run in runs {
        // A limit that read lower afterwards is a window that began again, and what was spent
        // before it turned over is in no reading at all.
        let took = run
            .over
            .and_then(|(before, after)| after.checked_sub(before))
            .filter(|took| *took > 0);
        if let Some(took) = took {
            moved = moved.saturating_add(took);
            priced = priced.saturating_add(run.priced);
        }
    }
    (moved, priced)
}

/// What the runs before this one came to, in both figures a decision asks of them.
///
/// One value because they answer together and come from one read of the same ledger: what a
/// run of a model cost, and how long one took. A decision that had only the first would start
/// a run the clock cannot let finish.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Before {
    /// What a run of each model has cost, in the unit the session declared.
    pub cost: Sizings,
    /// How long a run of each model has taken, in seconds.
    pub lasting: Sizings,
}

#[cfg(test)]
mod tests;
