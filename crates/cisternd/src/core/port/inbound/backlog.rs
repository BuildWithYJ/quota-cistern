//! What the core offers over the backlog.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output.
//! Reading what a run wrote is not here: a trace belongs to the run rather than to the task that was registered.
//! `port::inbound::execution` offers it.

use super::Refusal;

/// What `task add` was given.
///
/// The arguments arrive together because they are read together.
/// A parameter list of this length is harder to call correctly than a value with named fields.
pub struct Registration<'a> {
    /// Where the surface was run.
    /// The core runs as a daemon, so it cannot read this from its own process.
    pub cwd: &'a str,
    pub title: &'a str,
    pub instruction: &'a str,
    /// What the author wrote, where the instruction above is not it.
    ///
    /// A surface supplies it once it has asked which fill was meant: the question and the answer
    /// are two requests and the core holds nothing between them, so the text the author started
    /// from would otherwise be gone by the time a task is registered. Nothing where a surface has
    /// asked nothing, which is every request that is not the second half of a question.
    pub original: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub after: Option<&'a str>,
    pub model: Option<&'a str>,
    /// Register even when the instruction carries too little to run unattended.
    pub force: bool,
}

/// A task that was registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub id: String,
    pub title: String,
    pub base_branch: String,
    pub after: Option<String>,
    pub model: Option<String>,
    pub repository: String,
    pub state: String,
}

/// What became of a registration.
///
/// A `task add` either put a task in the backlog or came back with a question, so the two are one
/// answer rather than an answer and an error. Nothing was written in the second case: the
/// instruction was filled in more than one way and no one has said which was meant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// The task is in the backlog.
    Added(Added),
    /// Nothing was written, and this is what has to be settled first.
    Unconfirmed(Unconfirmed),
}

/// A spec nobody has accepted, as it stands.
///
/// The core cannot ask: it runs as a daemon and the person is at a surface. So it answers with
/// the spec it would have registered and what that spec still leaves for the agent, and a surface
/// that has somebody in front of it shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unconfirmed {
    /// Every part of the spec, in reading order.
    pub parts: Vec<Shown>,
    /// What is still left for the agent to decide on its own.
    pub undecided: Vec<Left>,
}

/// One part of a spec, as a surface shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// What the part is called.
    pub part: String,
    /// What it says, where anything does.
    pub said: Option<String>,
    /// Who settled it: the author, the repository, or nobody yet.
    pub settled: String,
    /// What it was drawn from, for a reader deciding whether to take it.
    pub drawn_from: Option<String>,
    /// The others the repository allows, or what to choose between where nothing was settled.
    pub others: Vec<String>,
    /// What to ask about it, in the words the author wrote in.
    pub asks: Option<String>,
}

/// One decision the spec leaves for the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    /// The part it is about, where it is about one.
    pub part: Option<String>,
    /// What the agent would settle for itself while this stands.
    pub decides: String,
}

/// A task that was taken out of the backlog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub id: String,
    pub title: String,
}

/// One task in full.
///
/// The fields a session fills in are here and empty.
/// They answer as null, which is what section 2.1 says they do before a task has been assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub id: String,
    pub session: Option<String>,
    pub state: String,
    pub title: String,
    /// What the run is given to work from.
    pub instruction: String,
    /// What the author wrote, when that is not what the run is given.
    pub original: Option<String>,
    pub base_branch: String,
    pub after: Option<String>,
    pub model: Option<String>,
    pub repository: String,
    pub branch: Option<String>,
    pub reason: Option<String>,
    pub worktree: Option<String>,
    /// The conversation its last run was in, for a task that may be carried on.
    pub conversation: Option<String>,
    pub disposition: Option<String>,
    /// What the branch holds, for a task whose run has ended.
    pub commits: Option<Vec<Made>>,
    /// Commits the base has gained since the task left it.
    pub base_ahead: Option<u64>,
}

/// One commit a task made, as `task show` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    pub sha: String,
    pub subject: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

/// The tasks waiting to be assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub items: Vec<Waiting>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    pub id: String,
    pub title: String,
    pub base_branch: String,
}

/// `task add`, `task rm`, `task show`, and `backlog`.
pub trait BacklogUseCase {
    /// Registers a task.
    fn add(&self, given: Registration<'_>) -> Result<Registered, Refusal>;

    /// Takes a task out of the backlog.
    fn remove(&self, id: &str) -> Result<Removed, Refusal>;

    /// Reads one task in full.
    fn show(&self, id: &str) -> Result<Detail, Refusal>;

    /// Lists the tasks waiting to be assigned.
    fn list(&self) -> Result<Listing, Refusal>;
}
