//! A sweep over the numbers a run is sized by.
//!
//! Not a test. Nothing here asserts, since what the numbers should be is the question rather
//! than something already settled. It runs sessions against a stand-in vendor and prints what
//! each rule came to, and a person reads the table.
//!
//!     cargo test -p cisternd -- --ignored --nocapture sweeping
//!
//! Why it exists. Four sessions on a real repository settled none of the three numbers: no run
//! came within half of its ceiling, so any rule would have ended them alike. A ceiling only
//! decides something where runs meet it, and meeting one costs whatever the run had spent. Here
//! it costs nothing, so the rules can be compared at the sizes where they differ.
//!
//! What the sessions are held to. A budget declared as a count of tokens rather than as a share,
//! so that the sizing is what the table is about: a share would put the token-to-limit rate in
//! the middle of it, which is its own question. The clock is frozen, so nothing is held back for
//! want of time. One vendor, one model, one ladder.
//!
//! Where the costs come from. Four shapes rather than one, and none of them measured off our own
//! account: a rule that only holds up under the sizes we happened to run is not one to ship. A
//! rule worth taking is one that reads well under all four.

use crate::core::{
    domain::Rule,
    port::{
        inbound::{Carrying, ExecutionUseCase},
        outbound::{BacklogStore, StoredTask},
    },
};

use super::fixtures::*;
use super::{ExecutionService, Outside, Supervisor, WorkService};

/// How many sessions each rule is read over, per shape and budget.
const SESSIONS: u64 = 100;

/// How many tasks wait at the start of one.
const TASKS: usize = 12;

/// What a session declares, as multiples of the middle of the shape it draws from.
///
/// Read at three sizes, because which rule is even reached depends on this. A ceiling is the
/// smaller of what the estimate allows and what is left, so a session whose budget is small
/// against its tasks is held by what is left every time and never asks the estimate anything.
/// Two runs out early, thirty-two covers the whole backlog, eight is between.
const BUDGETS: [u64; 3] = [2, 8, 32];

/// What the definition's guard is, as a multiple of the middle of the shape.
///
/// Fixed, and not a share of anything a session declared. A person raises it where runs are
/// meant to be larger.
const GUARD: u64 = 4;

/// A stream of numbers that is the same stream every time.
///
/// A rule is read against the sessions another rule was read against, so the draws have to
/// repeat. Written out rather than taken from a crate: this is thirty lines and a dependency
/// the daemon carries into production for the sake of a table is not worth it.
struct Drawn(u64);

impl Drawn {
    fn seeded(seed: u64) -> Self {
        Drawn(seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1))
    }

    /// The next draw, over zero to one.
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // The top bits, which are the ones this generator moves well.
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A shape a task's cost is drawn from.
///
/// Named for what it is rather than for a distribution, since which distribution job runtimes
/// follow is not settled and the point here is to read a rule under more than one answer.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// Tasks of a size. What a backlog of one kind of change looks like.
    Alike,
    /// Two orders of magnitude between the smallest and the largest, evenly over the logarithm.
    /// The shape the parallel-batch literature reports for job runtimes.
    Spread,
    /// A tail with no shoulder: most runs small, a few enormous. Pareto at the exponent
    /// reported for process lifetimes, cut off where a run would outlast any budget.
    Tailed,
    /// Small tasks and large ones and nothing between. A backlog of documents and features.
    Split,
}

const SHAPES: [Shape; 4] = [Shape::Alike, Shape::Spread, Shape::Tailed, Shape::Split];

impl Shape {
    fn named(self) -> &'static str {
        match self {
            Shape::Alike => "alike",
            Shape::Spread => "spread",
            Shape::Tailed => "tailed",
            Shape::Split => "split",
        }
    }

    /// What one task costs.
    fn draws(self, drawn: &mut Drawn) -> u64 {
        let one = drawn.next();
        let cost = match self {
            Shape::Alike => 1_000.0 * (0.8 + 0.4 * one),
            Shape::Spread => 100.0 * 100.0_f64.powf(one),
            Shape::Tailed => (200.0 / (1.0 - one).max(0.001)).min(200_000.0),
            Shape::Split => match one < 0.8 {
                true => 250.0 * (0.8 + 0.4 * drawn.next()),
                false => 4_000.0 * (0.8 + 0.4 * drawn.next()),
            },
        };
        cost.round().max(1.0) as u64
    }

    /// The middle of the shape, which a budget is set from.
    ///
    /// Drawn rather than worked out, so a shape whose middle is awkward to write down needs no
    /// formula here.
    fn middle(self) -> u64 {
        let mut drawn = Drawn::seeded(7);
        let mut costs: Vec<u64> = (0..1_000).map(|_| self.draws(&mut drawn)).collect();
        costs.sort_unstable();
        costs[costs.len() / 2]
    }
}

/// What came of the sessions one rule was read over.
#[derive(Debug, Default, Clone, Copy)]
struct Came {
    finished: u64,
    stopped: u64,
    /// What the finished runs spent.
    done: u64,
    /// What the stopped runs spent, which bought nothing.
    lost: u64,
    /// What the sessions declared, all told.
    declared: u64,
    /// How many of them spent more than that, which is the one thing that must not happen.
    over: u64,
    /// What the tasks put in front of them would have cost, all told.
    ///
    /// Beside the count of tasks finished, because the two disagree. A rule whose ceilings are
    /// small finishes more tasks and finishes only the cheap ones; whether that is better turns
    /// on whether a task is worth what it costs or worth the same as any other, which is a
    /// judgement about the work rather than something a table settles.
    offered: u64,
}

impl Came {
    fn and(mut self, other: Came) -> Came {
        self.finished += other.finished;
        self.stopped += other.stopped;
        self.done += other.done;
        self.lost += other.lost;
        self.declared += other.declared;
        self.over += other.over;
        self.offered += other.offered;
        self
    }
}

/// One session, from the backlog it starts with to the state it ends in.
///
/// The ledger is handed in rather than made here, so that the sessions of one reading share
/// one. A session that began with an empty ledger would be in the cold start for its first
/// runs every time, and the cold start is not what a rule is being read for: what the daemon
/// does is keep the ledger between sessions, so a sizing has every run there has ever been
/// behind it.
fn one_session(
    rule: Rule,
    shape: Shape,
    budget: u64,
    guard: u64,
    seed: u64,
    runs: &Ledger,
) -> Came {
    let mut drawn = Drawn::seeded(seed);
    let costs: Vec<u64> = (0..TASKS).map(|_| shape.draws(&mut drawn)).collect();
    let declared = shape.middle() * budget;
    let offered: u64 = costs.iter().sum();

    let sessions = Remembered::empty();
    let held = Tasks::holding(
        (1..=TASKS)
            .map(|at| a_task_numbered(&at.to_string()))
            .collect::<Vec<StoredTask>>(),
    );
    let areas = Areas::default();
    // Every run is held to the guard the definition carries, which a person set and which does
    // not move with the budget. Four times the middle of the shape, which is about where the
    // twenty dollars that ships sits against the runs a real session has had.
    let agent = Costing::taking(costs.clone()).guarded_at(shape.middle() * guard);
    let outside = Outside {
        sessions: &sessions,
        tasks: &held,
        worktrees: &areas,
        agent: &agent,
        clock: &STILL,
        limit: &UNTOUCHED,
        traces: &NOTHING_KEPT,
        runs,
    };
    // Only this session's runs are counted, and the ledger already holds those before it.
    let before = runs.runs().len();
    let supervisor = Supervisor::sizing_by(outside, AT_ONCE, rule);
    let execution = ExecutionService::new(outside, &supervisor);
    let work = WorkService::new(outside, &supervisor);

    execution
        .run(declaring(&declared.to_string(), "8h"))
        .unwrap();
    // A session assigns as tasks end, so what is running is looked at again each time.
    for _ in 0..TASKS * 4 {
        let stored = held.load().unwrap();
        let Some(running) = stored.tasks.iter().find(|task| task.state == "Running") else {
            break;
        };
        work.carry_on(&format!("task:{}", running.id)).unwrap();
    }

    let mut came = Came {
        declared,
        offered,
        ..Came::default()
    };
    for run in runs.runs().into_iter().skip(before) {
        let spent = run
            .spent
            .as_ref()
            .and_then(|spent| spent.cost.parse::<u64>().ok())
            .unwrap_or_default();
        match run.outcome.as_str() {
            "Completed" => {
                came.finished += 1;
                came.done += spent;
            }
            _ => {
                came.stopped += 1;
                came.lost += spent;
            }
        }
    }
    came.over = u64::from(came.done + came.lost > declared);
    came
}

/// Every rule read over every shape, printed one line each.
#[test]
#[ignore = "a sweep to read rather than a test to pass"]
fn sweeping_the_rule() {
    let shipped = Rule::default();
    let rules: Vec<(String, Rule)> = [0, 1, 2, 3, 4]
        .into_iter()
        .map(|widen| (format!("widen {widen}"), Rule { widen, ..shipped }))
        .chain([80, 90, 95, 99, 100].into_iter().map(|estimate| {
            (
                format!("quantile {estimate}"),
                Rule {
                    estimate,
                    ..shipped
                },
            )
        }))
        .chain(
            [0, 25, 50, 75, 90, 100]
                .into_iter()
                .map(|middle| (format!("floor {middle}"), Rule { middle, ..shipped })),
        )
        .chain(
            [0, 1, 2, 4]
                .into_iter()
                .map(|lift| (format!("lift {lift}"), Rule { lift, ..shipped })),
        )
        .collect();

    println!("\n{SESSIONS} sessions of {TASKS} tasks each, one ledger behind each column\n");
    for shape in SHAPES {
        for budget in BUDGETS {
            println!(
                "--- {} (middle {}), budget {}x ---",
                shape.named(),
                shape.middle(),
                budget
            );
            println!(
                "{:<14} {:>8} {:>8} {:>7} {:>7} {:>7} {:>7} {:>5}",
                "rule", "finished", "stopped", "cut %", "used %", "lost %", "work %", "over"
            );
            for (named, rule) in &rules {
                let runs = Ledger::default();
                let came = (0..SESSIONS)
                    .map(|seed| one_session(*rule, shape, budget, GUARD, seed, &runs))
                    .fold(Came::default(), Came::and);
                let all = (came.finished + came.stopped).max(1);
                let spent = came.done + came.lost;
                println!(
                    "{:<14} {:>8} {:>8} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>5}",
                    named,
                    came.finished,
                    came.stopped,
                    100.0 * came.stopped as f64 / all as f64,
                    100.0 * spent as f64 / came.declared.max(1) as f64,
                    100.0 * came.lost as f64 / spent.max(1) as f64,
                    100.0 * came.done as f64 / came.offered.max(1) as f64,
                    came.over,
                );
            }
            println!();
        }
    }
}
