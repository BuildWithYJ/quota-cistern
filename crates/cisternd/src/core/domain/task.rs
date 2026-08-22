//! A task and the rules over a backlog of them.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output, and section 1 fixes the identifiers and the states.
//! This module is private, so a value that reached here was parsed on the way in.

use std::fmt::{self, Display};

use super::{Consumption, Observation, SessionId};

/// The branch a task starts from when it names neither a branch nor a predecessor.
const DEFAULT_BRANCH: &str = "main";

/// A task number.
///
/// The core issues these.
/// They only increase and are never reused, and sessions count on a sequence of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(u32);

/// The reason section 1 of `docs/cli.md` gives a task stopped at the ceiling on one run.
///
/// A word rather than a state, since the state is `Interrupted` whichever way a run was cut
/// off. Beside the states because two roles read it: what a person is told, and what the
/// supervisor reads back off the ledger, where a run that ended this way says where it was
/// stopped rather than what its task takes.
pub const AT_CEILING: &str = "task ceiling";

/// The five states section 1 lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running,
    Completed,
    Interrupted,
    Error,
}

/// Whether a task waiting again keeps the conversation its last run was in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carrying {
    /// It does, so the next run carries that conversation on.
    On,
    /// It does not, so the next run starts one.
    Afresh,
}

/// What was decided about a task's result.
///
/// Apart from the state, which says how the run ended rather than what was done about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Applied,
    Discarded,
}

/// The repository a task was added from.
///
/// Never read as a path here.
/// The core keeps what an adapter handed over and hands it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository(String);

/// What a task was registered with.
///
/// `after` decides when it may be assigned and `branch` decides where it starts from.
/// A task may carry both, one, or neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    id: TaskId,
    title: String,
    instruction: String,
    /// What the author wrote, for a task whose instruction was filled in from something else.
    ///
    /// Absent when the instruction is theirs as they wrote it, so its presence is what says a
    /// fill happened at all.
    original: Option<String>,
    branch: Option<String>,
    after: Option<TaskId>,
    model: Option<String>,
    repository: Repository,
    state: TaskState,
    /// The session that assigned it, once one has.
    session: Option<SessionId>,
    /// Where it is being worked on, once a work area was made.
    worktree: Option<String>,
    /// The conversation its last run was in, for a run that may be carried on.
    conversation: Option<String>,
    /// When its most recent run started, in seconds since the epoch.
    ///
    /// The most recent rather than the first, since a task the vendor turned away runs again.
    started_at: Option<u64>,
    /// When that run stopped, however it ended.
    ended_at: Option<u64>,
    /// Why it ended as it did, for a task that did not simply finish.
    reason: Option<String>,
    /// How many times it has been assigned.
    ///
    /// Assignments rather than failures. A run cut off at a ceiling leaves no record of its
    /// own, and a run the vendor turned away leaves none either, so counting what went wrong
    /// counts less than what was tried.
    attempts: u32,
    /// What the run going now is allowed to consume, in the unit its session declared.
    ///
    /// Held against that session's budget until the run ends, so that runs starting together
    /// cannot together pass what the session declared. Absent for a task nothing is running
    /// for, and for one assigned before this was kept.
    ceiling: Option<u64>,
    /// What running it consumed, as far as that is known.
    consumed: Observation,
    /// What was decided about its result, once anyone decided.
    disposition: Option<Disposition>,
}

/// What a run is given to work from, and what the author wrote when the two differ.
///
/// The two travel together: an original without the instruction it grew from says nothing, and
/// which of them a reader wants depends on whether they are reading the run or reviewing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// What the run is given.
    pub given: String,
    /// What the author wrote, when the run is given something else.
    pub original: Option<String>,
}

/// A task on its way back from a store, with every value already read.
///
/// Whoever read it names the fields once here, and the backlog checks them together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub id: TaskId,
    pub title: String,
    pub instruction: String,
    pub original: Option<String>,
    pub branch: Option<String>,
    pub after: Option<TaskId>,
    pub model: Option<String>,
    pub repository: Repository,
    pub state: TaskState,
    pub session: Option<SessionId>,
    pub worktree: Option<String>,
    pub conversation: Option<String>,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub reason: Option<String>,
    pub attempts: u32,
    pub ceiling: Option<u64>,
    pub consumed: Observation,
    pub disposition: Option<Disposition>,
}

/// Every task, and the number the next one will get.
///
/// The number is kept rather than derived.
/// The highest one present plus one would hand out a removed task's number again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backlog {
    next_id: u32,
    tasks: Vec<Task>,
}

/// A set of tasks that no backlog could be.
///
/// A value that could not be read is not here, because reading one is not the domain's work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotABacklog {
    /// Two tasks carry the same number.
    RepeatedId { id: TaskId },
    /// A task waits for one that is not there.
    NoSuchPredecessor { task: TaskId, after: TaskId },
    /// Following `after` from this task leads back to it.
    Cycle { task: TaskId },
}

/// Why a task could not be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemovalRefused {
    NoSuchTask,
    NotPending,
}

/// Why a task's result could not be disposed of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisposalRefused {
    NoSuchTask,
    /// The run has not ended, so there is no result to decide about.
    NotEnded,
}

impl TaskId {
    /// Reads an identifier as a user writes it.
    ///
    /// `docs/cli.md` lets the `task:` prefix be left off in arguments.
    /// Both spellings are read here and only the number is kept.
    pub fn parse(id: &str) -> Option<Self> {
        let digits = id.strip_prefix("task:").unwrap_or(id);
        digits.parse().ok().map(TaskId)
    }

    /// The identifier as section 1 writes it.
    ///
    /// [`Display`] gives the number alone, which is what a branch name is built from.
    /// Both spellings come from here so they cannot drift apart.
    pub fn labelled(&self) -> String {
        format!("task:{}", self.0)
    }
}

impl Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TaskState {
    /// Reads a state name.
    ///
    /// One spelling is read and written, so what a store holds and what is printed cannot drift apart.
    pub fn parse(state: &str) -> Option<Self> {
        match state {
            "Pending" => Some(TaskState::Pending),
            "Running" => Some(TaskState::Running),
            "Completed" => Some(TaskState::Completed),
            "Interrupted" => Some(TaskState::Interrupted),
            "Error" => Some(TaskState::Error),
            _ => None,
        }
    }

    /// Whether the run is over, whatever it ended as.
    ///
    /// Section 1 calls these the terminal states.
    /// Section 1 says every one of them leaves a branch and enters the review queue.
    pub fn ended(self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Interrupted | TaskState::Error
        )
    }
}

impl Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TaskState::Pending => "Pending",
            TaskState::Running => "Running",
            TaskState::Completed => "Completed",
            TaskState::Interrupted => "Interrupted",
            TaskState::Error => "Error",
        })
    }
}

impl Disposition {
    /// Reads a disposition.
    ///
    /// One spelling is read and written, so what a store holds and what is printed cannot drift apart.
    pub fn parse(disposition: &str) -> Option<Self> {
        match disposition {
            "applied" => Some(Disposition::Applied),
            "discarded" => Some(Disposition::Discarded),
            _ => None,
        }
    }
}

impl Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Disposition::Applied => "applied",
            Disposition::Discarded => "discarded",
        })
    }
}

impl Repository {
    pub fn new(named: impl Into<String>) -> Self {
        Repository(named.into())
    }
}

impl Display for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Task {
    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn instruction(&self) -> &str {
        &self.instruction
    }

    /// What the author wrote, for a task whose instruction was filled in from something else.
    pub fn original(&self) -> Option<&str> {
        self.original.as_deref()
    }

    /// The branch that was named, if one was.
    ///
    /// [`Task::base_branch`] is where the task starts from, which is this only when a branch was named.
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn after(&self) -> Option<TaskId> {
        self.after
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub fn state(&self) -> TaskState {
        self.state
    }

    pub fn session(&self) -> Option<SessionId> {
        self.session
    }

    /// The conversation its last run was in, once one has left one.
    ///
    /// A run of a task that was cut off left the work it had done in the work area and on the
    /// branch, and its conversation nowhere. Reading all of that back is most of what a second
    /// run of the same task costs. This is what lets the next run carry that conversation on
    /// instead, and it is the task's rather than the run's because it is the next run that
    /// needs it and runs do not outlive themselves.
    ///
    /// Nothing for a task nobody has run, and nothing again once one has finished: what is
    /// kept is a conversation somebody may still want to carry on.
    pub fn conversation(&self) -> Option<&str> {
        self.conversation.as_deref()
    }

    pub fn worktree(&self) -> Option<&str> {
        self.worktree.as_deref()
    }

    /// When its most recent run started, once one has.
    pub fn started_at(&self) -> Option<u64> {
        self.started_at
    }

    /// When that run stopped, once it has.
    pub fn ended_at(&self) -> Option<u64> {
        self.ended_at
    }

    /// What the run going now is allowed to consume, once one was assigned.
    pub fn ceiling(&self) -> Option<u64> {
        self.ceiling
    }

    /// How many times it has been assigned.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// What running it consumed, as far as that is known.
    pub fn consumed(&self) -> &Observation {
        &self.consumed
    }

    /// What was decided about its result, once anyone decided.
    pub fn disposition(&self) -> Option<Disposition> {
        self.disposition
    }

    /// The branch this task's result is kept on, once one has been cut.
    ///
    /// A task that was never assigned has none, which is what section 2.1 reports as null.
    pub fn result_branch(&self) -> Option<String> {
        match self.state {
            TaskState::Pending => None,
            _ => Some(result_branch_of(self.id)),
        }
    }

    /// Where the task starts from.
    ///
    /// Derived rather than stored, since a predecessor's result branch does not exist yet when the task is registered.
    pub fn base_branch(&self) -> String {
        match (&self.branch, self.after) {
            (Some(branch), _) => branch.clone(),
            (None, Some(after)) => result_branch_of(after),
            (None, None) => DEFAULT_BRANCH.to_owned(),
        }
    }
}

/// The branch a task's result is kept on.
///
/// A predecessor's is where a task that waits for it starts from.
/// Both spellings come from here and cannot drift apart.
fn result_branch_of(id: TaskId) -> String {
    format!("cistern/{id}")
}

impl Backlog {
    /// Registers a task and hands back a copy of it.
    ///
    /// The task rather than its number.
    /// Whoever registered it does not look it up again and answer for not finding what it just added.
    pub fn add(
        &mut self,
        title: String,
        instruction: Instruction,
        branch: Option<String>,
        after: Option<TaskId>,
        model: Option<String>,
        repository: Repository,
    ) -> Task {
        let id = TaskId(self.next_id);
        self.next_id += 1;
        let registered = Task {
            id,
            title,
            instruction: instruction.given,
            original: instruction.original,
            branch,
            after,
            model,
            repository,
            state: TaskState::Pending,
            session: None,
            worktree: None,
            conversation: None,
            started_at: None,
            ended_at: None,
            reason: None,
            attempts: 0,
            ceiling: None,
            consumed: Observation::NotYet,
            disposition: None,
        };
        self.tasks.push(registered.clone());
        registered
    }

    /// Takes a task out of the backlog.
    ///
    /// Whatever waited for it now waits for what it waited for.
    /// A task naming one that is not there is a backlog this core cannot read.
    pub fn remove(&mut self, id: TaskId) -> Result<Task, RemovalRefused> {
        let Some(at) = self.tasks.iter().position(|task| task.id == id) else {
            return Err(RemovalRefused::NoSuchTask);
        };
        if self.tasks[at].state != TaskState::Pending {
            return Err(RemovalRefused::NotPending);
        }

        let removed = self.tasks.remove(at);
        for waiting in &mut self.tasks {
            if waiting.after == Some(id) {
                waiting.after = removed.after;
            }
        }
        Ok(removed)
    }

    /// Hands one named task to a session, with what its run may take.
    ///
    /// Named rather than taken from the front, since what each may take was decided over the
    /// whole list and the decision says which ones.
    ///
    /// Nothing is answered for a task the backlog does not hold, and for one that is not
    /// waiting: a task another thread took first is not this session's to assign.
    pub fn assign(
        &mut self,
        id: TaskId,
        to: SessionId,
        ceiling: u64,
        now: u64,
        fallback: Option<&str>,
    ) -> Option<TaskId> {
        let held = self
            .tasks
            .iter_mut()
            .find(|task| task.id == id && task.state == TaskState::Pending)?;
        held.state = TaskState::Running;
        held.session = Some(to);
        held.ceiling = Some(ceiling);
        held.attempts += 1;
        // Section 2.2 of `docs/cli.md` says a session's `--model` is what a task that named
        // none falls back to, and section 2.1 says a task reports the model it ran on. Written
        // down here, where what it runs on is settled, rather than read again everywhere a run
        // is started or recorded: the run that follows and the line the ledger keeps for it
        // then say the same thing, which is what the sizing reads them for.
        if held.model.is_none() {
            held.model = fallback.map(str::to_owned);
        }
        // A task the vendor turned away is assigned again, and the run that
        // starts now is the one these two describe.
        held.started_at = Some(now);
        held.ended_at = None;
        Some(id)
    }

    /// The task `assign` would hand over, without handing it over.
    ///
    /// A run that has nothing to start is refused before a session is opened.
    /// Asking that question is what this is for.
    pub fn next_to_assign(&self) -> Option<TaskId> {
        self.waiting().first().map(|(id, _)| *id)
    }

    /// Every task that may start, in the order `assign` would take them, each with the model
    /// it named.
    ///
    /// What each may be allowed depends on its model, so a decision needs the list rather than
    /// a count of it.
    pub fn waiting(&self) -> Vec<(TaskId, Option<String>)> {
        self.tasks
            .iter()
            .filter(|task| task.state == TaskState::Pending)
            .filter(|task| match task.after {
                None => true,
                Some(after) => self
                    .find(after)
                    .is_some_and(|held| held.state == TaskState::Completed),
            })
            .map(|task| (task.id, task.model.clone()))
            .collect()
    }

    /// What the runs already going are allowed to take, together.
    ///
    /// A task that was assigned before this figure was kept has none, and counts as nothing.
    /// The session it belongs to has already spent whatever that run spent, which the budget
    /// sees; what is missing is only the room held for the rest of it.
    pub fn booked_in(&self, session: SessionId) -> u64 {
        self.tasks
            .iter()
            .filter(|task| task.session == Some(session) && task.state == TaskState::Running)
            .filter_map(|task| task.ceiling)
            .sum()
    }

    /// Records where a task is being worked on.
    pub fn work_area(&mut self, id: TaskId, at: String) {
        for task in &mut self.tasks {
            if task.id == id {
                task.worktree = Some(at.clone());
            }
        }
    }

    /// The tasks whose work areas may be taken away, each with the repository it belongs to
    /// and where it is.
    ///
    /// Read rather than taken. Removing one is slow and the daemon can be killed part way
    /// through, so a backlog written as having lost them all before the first is gone would
    /// leave every directory that was still there claimed by nobody and never looked at
    /// again. Each is forgotten as it goes instead, by `work_area_gone`.
    ///
    /// What keeps a `retry` from being handed a directory about to go is not this. One core
    /// holds the socket, so a hold inside the process covers it, and a hold is not something
    /// a crash can leave behind.
    pub fn tidyable(&self) -> Vec<(TaskId, String, String)> {
        self.tasks
            .iter()
            .filter(|task| task.state.ended() && task.disposition.is_some())
            .filter_map(|task| Some((task.id, task.repository.to_string(), task.worktree.clone()?)))
            .collect()
    }

    /// Forgets where a task worked, for one whose work area has been taken away.
    pub fn work_area_gone(&mut self, id: TaskId) {
        for task in &mut self.tasks {
            if task.id == id {
                task.worktree = None;
            }
        }
    }

    /// Moves a task to the state it ended in, leaving the branch alone.
    ///
    /// Only a task that is still running ends.
    /// One interrupted by hand ends the moment the user asks.
    /// The thread waiting on its agent comes back afterwards to say it failed.
    /// The first answer is the true one.
    pub fn finish(&mut self, id: TaskId, state: TaskState, reason: Option<String>, now: u64) {
        for task in &mut self.tasks {
            if task.id == id && task.state == TaskState::Running {
                task.state = state;
                task.reason.clone_from(&reason);
                task.ended_at = Some(now);
            }
        }
    }

    /// Records what running a task consumed.
    ///
    /// Kept apart from [`Backlog::finish`].
    /// Records the conversation a run left, for a task that may be carried on.
    ///
    /// Beside `finish` rather than in it: what a run consumed and what conversation it was in
    /// are two things a vendor may answer about separately, and one of them being absent is
    /// not the other being absent.
    pub fn conversed(&mut self, id: TaskId, conversation: Option<String>) {
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
            task.conversation = conversation;
        }
    }

    /// A task can end without ever having run, and what it consumed is then not nothing but unknown.
    pub fn record(&mut self, id: TaskId, consumed: Observation) {
        for task in &mut self.tasks {
            if task.id == id {
                task.consumed = consumed.clone();
            }
        }
    }

    /// Records what was decided about a task's result.
    ///
    /// It may be decided again, since section 2.4 keeps the branch either way.
    /// What is refused is deciding about a run that has not ended.
    pub fn dispose(&mut self, id: TaskId, disposition: Disposition) -> Result<(), DisposalRefused> {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return Err(DisposalRefused::NoSuchTask);
        };
        if !task.state.ended() {
            return Err(DisposalRefused::NotEnded);
        }
        task.disposition = Some(disposition);
        // Nobody carries on a conversation about work that has been decided.
        task.conversation = None;
        Ok(())
    }

    /// Puts a task that ended back where it started.
    ///
    /// A ceiling cuts runs off, and a task cut off ends `Interrupted` with whatever it did on
    /// its branch. Nothing else moves it back: `dispose` takes it off the review queue and
    /// leaves the state where it was, so a task left there is one nothing will pick up and one
    /// whose successors wait forever.
    ///
    /// The branch stays. What the cut-off run wrote is on it, and a run that starts again
    /// starts from it.
    pub fn try_again(&mut self, id: TaskId) -> Result<(), DisposalRefused> {
        self.waits_again(id, Carrying::Afresh)
    }

    /// Puts a task that ended back where it started, keeping the conversation its last run
    /// was in, so the run that starts next carries that conversation on.
    ///
    /// Apart from `try_again` because they are different things to ask for. Trying again is
    /// doing the work over; carrying on is the same work continuing, and what it saves is
    /// reading back everything the last run had read.
    pub fn carries_on(&mut self, id: TaskId) -> Result<(), DisposalRefused> {
        self.waits_again(id, Carrying::On)
    }

    fn waits_again(&mut self, id: TaskId, carrying: Carrying) -> Result<(), DisposalRefused> {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return Err(DisposalRefused::NoSuchTask);
        };
        if !task.state.ended() {
            return Err(DisposalRefused::NotEnded);
        }
        task.state = TaskState::Pending;
        task.reason = None;
        task.ceiling = None;
        task.disposition = None;
        if carrying == Carrying::Afresh {
            task.conversation = None;
        }
        Ok(())
    }

    /// Every task waiting to be disposed of, in the order they were registered.
    ///
    /// Across sessions rather than within one: what is waiting for the user is waiting whichever run left it.
    pub fn awaiting_review(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|task| task.state.ended() && task.disposition.is_none())
            .collect()
    }

    /// What a session has consumed, added up over the tasks it assigned.
    ///
    /// One task nobody could read leaves the whole sum unreadable.
    /// What is missing from a total is not visible in it.
    pub fn consumed_by(&self, session: SessionId) -> Observation {
        let mut counted = Vec::new();
        for task in self
            .tasks
            .iter()
            .filter(|task| task.session == Some(session))
        {
            match &task.consumed {
                Observation::NotYet => {}
                Observation::Unreadable { why } => {
                    return Observation::Unreadable { why: why.clone() };
                }
                Observation::Spent(spent) => counted.push(*spent),
            }
        }
        Observation::Spent(Consumption::total(counted))
    }

    /// Puts a task back where it was before it was assigned.
    ///
    /// For a task nobody would run.
    /// The session it was assigned to stays, since that session paid for whatever the refused run
    /// got through and `counted_in` is what says so. Assigning it again names the next one.
    pub fn wait_again(&mut self, id: TaskId, now: u64) {
        for task in &mut self.tasks {
            if task.id == id {
                task.state = TaskState::Pending;
                task.reason = None;
                // The run it had is over even though the task waits again, and it
                // consumed whatever it consumed before the vendor refused it.
                task.ended_at = Some(now);
            }
        }
    }

    /// Whether tasks are left that none of them may start.
    ///
    /// A task waits on one that did not complete, and nothing will complete it while the
    /// session runs. Telling this from an empty backlog is what keeps a session from reporting
    /// that everything is done while tasks are still waiting.
    pub fn blocked(&self) -> bool {
        self.waiting().is_empty()
            && self
                .tasks
                .iter()
                .any(|task| task.state == TaskState::Pending)
    }

    /// How many of a session's tasks are still running.
    pub fn running_in(&self, session: SessionId) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.session == Some(session) && task.state == TaskState::Running)
            .count()
    }

    /// Ends every task a session still has running.
    ///
    /// Nothing is undone.
    /// Section 1 says an interrupted task keeps its partial work, which is on the branch already.
    pub fn interrupt(&mut self, session: SessionId, reason: &str, now: u64) -> Vec<TaskId> {
        let mut ended = Vec::new();
        for task in &mut self.tasks {
            if task.session == Some(session) && task.state == TaskState::Running {
                task.state = TaskState::Interrupted;
                task.reason = Some(reason.to_owned());
                task.ended_at = Some(now);
                ended.push(task.id);
            }
        }
        ended
    }

    /// What a session's own tasks consumed.
    pub fn counted_in(&self, session: SessionId) -> Vec<Consumption> {
        self.tasks
            .iter()
            .filter(|task| task.session == Some(session))
            .filter_map(|task| match &task.consumed {
                Observation::Spent(spent) => Some(*spent),
                _ => None,
            })
            .collect()
    }

    /// Every task a session took, in the order they were registered.
    pub fn taken_by(&self, session: SessionId) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|task| task.session == Some(session))
            .collect()
    }

    pub fn find(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    /// Every task, in the order they were registered.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// The number the next task will get.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Every task waiting to be assigned, in the order they were registered.
    pub fn pending(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|task| task.state == TaskState::Pending)
            .collect()
    }

    /// Rebuilds a backlog that was stored, and refuses one that does not add up.
    ///
    /// Every value arrives already read.
    /// What is left is what only the whole set shows: distinct numbers, every predecessor present, and no cycle.
    pub fn restore(next_id: u32, restored: Vec<Restored>) -> Result<Self, NotABacklog> {
        let tasks = restored
            .into_iter()
            .map(|held| Task {
                id: held.id,
                title: held.title,
                instruction: held.instruction,
                original: held.original,
                branch: held.branch,
                after: held.after,
                model: held.model,
                repository: held.repository,
                state: held.state,
                session: held.session,
                worktree: held.worktree,
                conversation: held.conversation,
                started_at: held.started_at,
                ended_at: held.ended_at,
                reason: held.reason,
                attempts: held.attempts,
                ceiling: held.ceiling,
                consumed: held.consumed,
                disposition: held.disposition,
            })
            .collect();

        let backlog = Backlog { next_id, tasks };
        backlog.no_repeated_id()?;
        backlog.every_predecessor_exists()?;
        backlog.no_cycle()?;
        Ok(backlog)
    }

    fn no_repeated_id(&self) -> Result<(), NotABacklog> {
        for (at, task) in self.tasks.iter().enumerate() {
            if self.tasks[..at].iter().any(|held| held.id == task.id) {
                return Err(NotABacklog::RepeatedId { id: task.id });
            }
        }
        Ok(())
    }

    fn every_predecessor_exists(&self) -> Result<(), NotABacklog> {
        for task in &self.tasks {
            if let Some(after) = task.after
                && self.find(after).is_none()
            {
                return Err(NotABacklog::NoSuchPredecessor {
                    task: task.id,
                    after,
                });
            }
        }
        Ok(())
    }

    /// `task add` cannot build a cycle, since a task may only name one that already exists, but a hand-edited file can.
    ///
    /// Walking as many steps as there are tasks is enough: a longer chain has visited something twice.
    fn no_cycle(&self) -> Result<(), NotABacklog> {
        for task in &self.tasks {
            let mut at = task.id;
            for _ in 0..self.tasks.len() {
                match self.find(at).and_then(Task::after) {
                    Some(next) => at = next,
                    None => break,
                }
                if at == task.id {
                    return Err(NotABacklog::Cycle { task: task.id });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
