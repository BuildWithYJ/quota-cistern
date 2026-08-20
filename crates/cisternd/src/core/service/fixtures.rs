//! What the service tests run against.
//!
//! One set of stand-ins rather than one per service, since the three of them decide, run, and
//! report over the same session. A test that reaches for a different stand-in would be testing
//! a different session.

/// The hands the composition root gives the core, fixed here.
pub(super) const AT_ONCE: usize = 4;

use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

use crate::core::{
    port::inbound::Declaration,
    port::outbound::{
        Agent, BacklogStore, Clock, Cut, Ended, Keeping, Limit, Observed, Outcome, Reading, Run,
        Runs, SessionStore, Spent, StoredBacklog, StoredConsumption, StoredSessions, StoredTask,
        Traces, Unavailable, Work, Worktrees,
    },
};

/// A run that finished, reported what it cost, and left a reading either side of it.
///
/// The two readings are what the vendor's limit stood at when the run before this one ended
/// and when this one did, which is not the same as what this run moved it by.
///
/// Priced at what it counted, which is the run of a single model. `a_run_costing` is where the
/// two figures come apart.
pub(super) fn a_run_of(task: &str, tokens: u64, over: (&str, &str)) -> Run {
    a_run_costing(task, tokens, tokens, over)
}

/// The same, priced at something other than what it counted.
///
/// What a token costs differs between models, so two runs of one price are two runs of one
/// size however far apart their counts are.
pub(super) fn a_run_costing(task: &str, tokens: u64, cost: u64, over: (&str, &str)) -> Run {
    Run {
        task: task.to_owned(),
        session: Some("1".to_owned()),
        model: None,
        started_at: "1000".to_owned(),
        ended_at: "1100".to_owned(),
        outcome: "Completed".to_owned(),
        reason: None,
        said: None,
        spent: Some(StoredConsumption {
            input: "0".to_owned(),
            output: tokens.to_string(),
            cache_written: "0".to_owned(),
            cache_read: "0".to_owned(),
            cost: cost.to_string(),
        }),
        unreadable: None,
        ceiling: None,
        limit_before: Some(over.0.to_owned()),
        limit_after: Some(over.1.to_owned()),
    }
}

/// Sessions held in memory, so the steps can be checked without a file.
pub(super) struct Remembered {
    pub(super) stored: Mutex<StoredSessions>,
}

impl Remembered {
    pub(super) fn empty() -> Self {
        Remembered::holding(StoredSessions {
            next_id: "1".to_owned(),
            sessions: Vec::new(),
        })
    }

    pub(super) fn holding(stored: StoredSessions) -> Self {
        Remembered {
            stored: Mutex::new(stored),
        }
    }

    pub(super) fn load(&self) -> StoredSessions {
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
pub(super) struct Tasks {
    pub(super) stored: Mutex<StoredBacklog>,
}

impl Tasks {
    pub(super) fn holding(tasks: Vec<StoredTask>) -> Self {
        Tasks {
            stored: Mutex::new(StoredBacklog {
                next_id: (tasks.len() + 1).to_string(),
                tasks,
            }),
        }
    }

    /// The tasks the store says are running, in the order it holds them.
    pub(super) fn running(&self) -> Vec<String> {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .tasks
            .iter()
            .filter(|task| task.state == "Running")
            .map(|task| task.id.clone())
            .collect()
    }

    pub(super) fn first(&self) -> StoredTask {
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

pub(super) fn a_second_task() -> StoredTask {
    a_task_numbered("2")
}

pub(super) fn a_task_numbered(id: &str) -> StoredTask {
    StoredTask {
        id: id.to_owned(),
        title: format!("tidy up, again ({id})"),
        ..a_pending_task()
    }
}

pub(super) fn a_pending_task() -> StoredTask {
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
        attempts: None,
        ceiling: None,
        consumed: None,
        unreadable: None,
        disposition: None,
    }
}

/// Work areas that are only remembered, so no repository is needed.
#[derive(Default)]
pub(super) struct Areas {
    pub(super) cut: Mutex<Vec<(String, String, String)>>,
    pub(super) taken: Mutex<Vec<String>>,
    pub(super) refuse: bool,
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

    fn remove(&self, _repository: &str, at: &str) -> Result<(), Unavailable> {
        self.taken.lock().unwrap().push(at.to_owned());
        Ok(())
    }
}

/// A trace store nothing looks at.
pub(super) struct Kept;

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

pub(super) static NOTHING_KEPT: Kept = Kept;

/// A clock that does not move, for the tests that do not care.
pub(super) struct Frozen(pub(super) u64);

impl Clock for Frozen {
    fn now(&self) -> u64 {
        self.0
    }
}

pub(super) static STILL: Frozen = Frozen(1_000);

/// A vendor limit that stands where a test put it.
pub(super) struct AtPercent {
    /// Hundredths of a percent, as the port carries it.
    pub(super) used: Mutex<u64>,
    pub(super) refuse: Mutex<bool>,
}

impl AtPercent {
    /// A limit that reads, until a test says it stops reading.
    pub(super) fn at(used: u64) -> Self {
        AtPercent {
            used: Mutex::new(used),
            refuse: Mutex::new(false),
        }
    }

    pub(super) fn refuse(&self) {
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
pub(super) struct Advancing {
    pub(super) used: Mutex<u64>,
    pub(super) step: u64,
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
///
/// The window is left unnamed, since a reading that fell is only a window turning over
/// where there is no name to tell one from the next. A vendor that names them says so, and
/// a fallen reading from a window it just named is a look that arrived late.
pub(super) struct Turning {
    pub(super) left: Mutex<Vec<u64>>,
}

impl Turning {
    pub(super) fn over(readings: &[u64]) -> Self {
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
            resets_at: String::new(),
        })
    }
}

/// A limit nothing asks for, since the session was declared in tokens.
pub(super) static UNTOUCHED: AtPercent = AtPercent {
    used: Mutex::new(0),
    refuse: Mutex::new(false),
};

/// What an agent that answered with a count it could read reports.
pub(super) fn spending() -> Observed {
    Observed::Spent(Spent {
        input: "77".to_owned(),
        output: "3377".to_owned(),
        cache_written: "28879".to_owned(),
        cache_read: "263483".to_owned(),
        cost: "92170".to_owned(),
    })
}

/// An agent that answers as told, and remembers what it was asked.
pub(super) struct Answering {
    pub(super) ended: Ended,
    pub(super) asked: Mutex<Vec<(String, String, Option<String>)>>,
    /// The tasks whose runs were asked to end.
    pub(super) stopped: Mutex<Vec<String>>,
}

impl Answering {
    pub(super) fn ending(ended: Ended) -> Self {
        Answering {
            ended,
            asked: Mutex::new(Vec::new()),
            stopped: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn finishing() -> Self {
        Answering::ending(Ended {
            outcome: Outcome::Finished,
            reason: None,
            observed: spending(),
        })
    }
}

impl Agent for Answering {
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

/// An agent whose runs cost what their task takes, held to the ceiling they were given.
///
/// A stand-in that answered the same way whatever it was allowed would show every task
/// finishing however low a ceiling was set, which is the one thing a ceiling decides. The
/// vendor stops a run at the figure it is told, and the run ends having spent up to there with
/// nothing done, so that is what this does.
///
/// One token to one of whatever the vendor prices in, so that a ceiling worked out in tokens
/// reaches the run as the same figure and a test can read the two as one.
pub(super) struct Costing {
    /// What each task takes, by the task it belongs to.
    pub(super) takes: BTreeMap<String, u64>,
    /// What the vendor holds every run to, as a definition carries one.
    ///
    /// The session's own figure never reaches the vendor: it is worked out from a meter that
    /// reads in whole percent and carries usage the session did not cause. What holds a run
    /// is the guard a person put in the definition. Nothing here stands in for that unless a
    /// test says so.
    pub(super) guard: Option<u64>,
}

impl Costing {
    /// Tasks numbered from one, each taking what is given for it.
    pub(super) fn taking(each: impl IntoIterator<Item = u64>) -> Self {
        Costing {
            takes: each
                .into_iter()
                .enumerate()
                .map(|(at, takes)| ((at + 1).to_string(), takes))
                .collect(),
            guard: None,
        }
    }

    /// The same, with the guard a definition carries.
    pub(super) fn guarded_at(self, guard: u64) -> Self {
        Costing {
            guard: Some(guard),
            ..self
        }
    }
}

impl Agent for Costing {
    fn stop(&self, _task: &str) {}

    fn work(&self, work: Work<'_>) -> Result<Ended, Unavailable> {
        let takes = self.takes.get(work.task).copied().unwrap_or_default();
        let (outcome, spent) = match self.guard {
            Some(allowed) if allowed < takes => (Outcome::AtCeiling, allowed),
            _ => (Outcome::Finished, takes),
        };
        Ok(Ended {
            outcome,
            reason: None,
            observed: Observed::Spent(Spent {
                input: "0".to_owned(),
                output: spent.to_string(),
                cache_written: "0".to_owned(),
                cache_read: "0".to_owned(),
                cost: spent.to_string(),
            }),
        })
    }
}

pub(super) fn declaring<'a>(usage: &'a str, time: &'a str) -> Declaration<'a> {
    Declaration {
        usage,
        time,
        model: None,
    }
}

pub(super) fn a_second_pending_task() -> StoredTask {
    StoredTask {
        id: "2".to_owned(),
        ..a_pending_task()
    }
}

/// How many of a store's tasks are running.
pub(super) fn running_in(tasks: &Tasks) -> usize {
    tasks
        .load()
        .unwrap()
        .tasks
        .iter()
        .filter(|task| task.state == "Running")
        .count()
}

/// Every run there has been, kept in memory.
#[derive(Default)]
pub(super) struct Ledger(pub(super) Mutex<Vec<Run>>);

impl Runs for Ledger {
    fn append(&self, run: Run) -> Result<(), Unavailable> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(run);
        Ok(())
    }

    fn read(&self) -> Result<Vec<Run>, Unavailable> {
        Ok(self.runs())
    }
}

impl Ledger {
    pub(super) fn runs(&self) -> Vec<Run> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
