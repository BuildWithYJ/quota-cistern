//! What the core offers over a session and what it runs.
//!
//! Section 2.2 of `docs/cli.md` fixes the arguments and the output, and
//! section 2.3 fixes `trace`. A trace is what a run wrote, so it is answered
//! here rather than beside the task that was registered.

use super::Refusal;

/// One thing a run did or said, as `trace` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Happened {
    pub at: String,
    pub said: String,
}

/// A stretch of what a run wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trail {
    pub events: Vec<Happened>,
    /// Where to carry on from.
    pub cursor: String,
    /// True once the task has ended and the trace can grow no more.
    pub done: bool,
}

/// What `run` was given.
pub struct Declaration<'a> {
    /// What may be consumed, as it was written.
    pub usage: &'a str,
    /// How long it may run, as it was written.
    pub time: &'a str,
    /// The model tasks fall back to when they name none.
    pub model: Option<&'a str>,
}

/// The budget a session was opened with, in the words it was declared in.
///
/// Section 2.2 reports consumption in the unit that was declared, so what was
/// written is what is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub usage: String,
    pub time: String,
}

/// A session that was opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    pub session: String,
    pub state: String,
    /// The tasks assigned at the start.
    pub assigned: Vec<String>,
    pub budget: Declared,
}

/// One session, as `session ls` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub state: String,
    /// What it consumed, in the unit it was declared in.
    pub consumed: String,
    pub task_count: usize,
    pub updated_at: String,
}

/// A page of them, and which page it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub page: u32,
    pub limit: u32,
    pub sessions: Vec<Listed>,
}

/// One of a session's tasks, as `session show` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    pub id: String,
    pub state: String,
    pub title: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
}

/// One session in full, as `session show` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub session: String,
    pub state: String,
    pub budget: Declared,
    /// What it consumed, in the unit the budget was declared in.
    pub consumed: Declared,
    pub stopped_reason: Option<String>,
    /// When the vendor's limit starts over. Only a session it turned away.
    pub resets_at: Option<String>,
    pub updated_at: String,
    pub tasks: Vec<Ran>,
}

/// A session that was stopped by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub session: String,
    pub state: String,
    /// The tasks that were running and now are not.
    pub interrupted_tasks: Vec<String>,
    /// What it consumed, in the unit the budget was declared in.
    pub consumed: Declared,
}

/// `run`, and the work a run leaves behind.
pub trait ExecutionUseCase {
    /// Opens a session and assigns what may start.
    ///
    /// It answers as soon as the session exists, because section 2.2 says the
    /// command returns at once and the tasks keep running.
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal>;

    /// Runs one assigned task to the end, and answers with whatever the
    /// decision that followed assigned next.
    ///
    /// This does not return until the task has, so whoever calls it is not the
    /// same thread that answered `run`. When that happens is the composition
    /// root's to arrange; what happens is here. What comes back has to be run
    /// the same way the tasks `run` answered with are.
    fn carry_on(&self, task: &str) -> Result<Vec<String>, Refusal>;

    /// Lists sessions, newest first.
    ///
    /// Both arguments arrive as they were written, so a page number that is
    /// not one is refused where every other argument is.
    fn sessions(&self, page: Option<&str>, limit: Option<&str>) -> Result<Page, Refusal>;

    /// One session and the tasks it held.
    fn session(&self, id: &str) -> Result<Report, Refusal>;

    /// What a task's run has written so far, from `since` onwards.
    ///
    /// Section 2.3 answers the same way whether the run is still going or has
    /// ended, so nothing here asks which.
    fn trace(&self, id: &str, since: Option<&str>) -> Result<Trail, Refusal>;

    /// Stops the running session.
    ///
    /// Section 2.2 allows one session at a time, so it takes no target.
    fn interrupt(&self) -> Result<Stopped, Refusal>;
}
