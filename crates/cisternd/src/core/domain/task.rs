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

/// The five states section 1 lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running,
    Completed,
    Interrupted,
    Error,
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
    branch: Option<String>,
    after: Option<TaskId>,
    model: Option<String>,
    repository: Repository,
    state: TaskState,
    /// The session that assigned it, once one has.
    session: Option<SessionId>,
    /// Where it is being worked on, once a work area was made.
    worktree: Option<String>,
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

/// A task on its way back from a store, with every value already read.
///
/// Whoever read it names the fields once here, and the backlog checks them together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub id: TaskId,
    pub title: String,
    pub instruction: String,
    pub branch: Option<String>,
    pub after: Option<TaskId>,
    pub model: Option<String>,
    pub repository: Repository,
    pub state: TaskState,
    pub session: Option<SessionId>,
    pub worktree: Option<String>,
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
        instruction: String,
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
            instruction,
            branch,
            after,
            model,
            repository,
            state: TaskState::Pending,
            session: None,
            worktree: None,
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

    /// Hands the first task that may start to a session, and says which.
    ///
    /// Nothing is answered when none may start, which an empty backlog and a blocked one both look like from here.
    /// Hands one named task to a session, with what its run may take.
    ///
    /// Named rather than taken from the front, since what each may take was decided over the
    /// whole list and the decision says which ones.
    pub fn assign(&mut self, id: TaskId, to: SessionId, ceiling: u64, now: u64) -> Option<TaskId> {
        let held = self
            .tasks
            .iter_mut()
            .find(|task| task.id == id && task.state == TaskState::Pending)?;
        held.state = TaskState::Running;
        held.session = Some(to);
        held.ceiling = Some(ceiling);
        held.attempts += 1;
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
                branch: held.branch,
                after: held.after,
                model: held.model,
                repository: held.repository,
                state: held.state,
                session: held.session,
                worktree: held.worktree,
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
mod tests {
    use super::*;

    fn held(id: &str, after: Option<&str>, state: &str) -> Restored {
        Restored {
            session: None,
            worktree: None,
            started_at: None,
            ended_at: None,
            reason: None,
            attempts: 0,
            ceiling: None,
            consumed: Observation::NotYet,
            disposition: None,
            id: TaskId::parse(id).unwrap(),
            title: "a task".to_owned(),
            instruction: "do it".to_owned(),
            branch: None,
            after: after.map(|after| TaskId::parse(after).unwrap()),
            model: None,
            repository: Repository::new("/work/api"),
            state: TaskState::parse(state).unwrap(),
        }
    }

    fn holding(tasks: Vec<Restored>) -> Result<Backlog, NotABacklog> {
        Backlog::restore(9, tasks)
    }

    fn registered(backlog: &mut Backlog, branch: Option<&str>, after: Option<TaskId>) -> TaskId {
        backlog
            .add(
                "a task".to_owned(),
                "do it".to_owned(),
                branch.map(str::to_owned),
                after,
                None,
                Repository::new("/work/api"),
            )
            .id()
    }

    #[test]
    fn an_identifier_is_read_with_or_without_its_prefix() {
        assert_eq!(TaskId::parse("task:3"), TaskId::parse("3"));
        assert_eq!(TaskId::parse("three"), None);
        assert_eq!(TaskId::parse("task:"), None);
    }

    #[test]
    fn an_identifier_is_shown_the_way_section_one_writes_it() {
        let id = TaskId::parse("3").unwrap();
        assert_eq!(id.labelled(), "task:3");
        // A branch name is built from the number alone.
        assert_eq!(id.to_string(), "3");
    }

    #[test]
    fn a_state_outside_the_specification_is_not_a_state() {
        assert_eq!(TaskState::parse("Sleeping"), None);
    }

    #[test]
    fn a_task_naming_neither_starts_from_main() {
        let mut backlog = Backlog::default();
        let id = registered(&mut backlog, None, None);
        assert_eq!(backlog.find(id).unwrap().base_branch(), "main");
    }

    #[test]
    fn a_task_naming_a_predecessor_starts_from_its_result_branch() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, Some(first));
        assert_eq!(
            backlog.find(second).unwrap().base_branch(),
            format!("cistern/{first}")
        );
    }

    /// The two answer different questions, so a task can wait for one result and start from another.
    #[test]
    fn naming_a_branch_wins_over_a_predecessor() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, Some("develop"), Some(first));
        let task = backlog.find(second).unwrap();
        assert_eq!(task.base_branch(), "develop");
        assert_eq!(task.after(), Some(first));
    }

    #[test]
    fn a_registered_task_is_pending_and_waiting() {
        let mut backlog = Backlog::default();
        let id = registered(&mut backlog, None, None);
        assert_eq!(backlog.find(id).unwrap().state(), TaskState::Pending);
        assert_eq!(backlog.pending().len(), 1);
    }

    #[test]
    fn identifiers_increase() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, None);
        assert!(second > first);
    }

    /// Section 1 says a number is never reused.
    #[test]
    fn the_number_of_a_removed_task_is_not_handed_out_again() {
        let mut backlog = Backlog::default();
        registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, None);
        backlog.remove(second).unwrap();

        let third = registered(&mut backlog, None, None);
        assert_ne!(third, second);
    }

    #[test]
    fn what_waited_for_a_removed_task_waits_for_what_that_one_waited_for() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, Some(first));
        let third = registered(&mut backlog, None, Some(second));

        backlog.remove(second).unwrap();
        assert_eq!(backlog.find(third).unwrap().after(), Some(first));
    }

    /// The branch the removed task would have produced is never made.
    /// What waited for it starts from where that one would have.
    #[test]
    fn removing_the_first_leaves_what_waited_for_it_waiting_for_nothing() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, Some(first));

        backlog.remove(first).unwrap();
        let task = backlog.find(second).unwrap();
        assert_eq!(task.after(), None);
        assert_eq!(task.base_branch(), "main");
    }

    /// Two tasks may name the same predecessor, and both are rebound.
    #[test]
    fn everything_that_waited_for_a_removed_task_is_rebound() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, Some(first));
        let third = registered(&mut backlog, None, Some(first));

        backlog.remove(first).unwrap();
        assert_eq!(backlog.find(second).unwrap().after(), None);
        assert_eq!(backlog.find(third).unwrap().after(), None);
    }

    #[test]
    fn removing_a_task_nobody_registered_says_so() {
        let mut backlog = Backlog::default();
        let absent = TaskId::parse("7").unwrap();
        assert_eq!(backlog.remove(absent), Err(RemovalRefused::NoSuchTask));
    }

    /// No command produces another state yet, so the task is built here.
    /// The rule is what is being checked, not the path that reaches it.
    #[test]
    fn a_task_that_is_not_pending_is_not_removed() {
        let mut backlog = holding(vec![held("1", None, "Running")]).unwrap();
        let running = TaskId::parse("1").unwrap();
        assert_eq!(backlog.remove(running), Err(RemovalRefused::NotPending));
        assert!(backlog.find(running).is_some());
    }

    #[test]
    fn only_pending_tasks_are_waiting() {
        let backlog = holding(vec![
            held("1", None, "Pending"),
            held("2", None, "Completed"),
        ])
        .unwrap();
        assert_eq!(backlog.pending().len(), 1);
    }

    #[test]
    fn a_restored_backlog_keeps_what_it_was_given() {
        let backlog = holding(vec![held("1", None, "Pending")]).unwrap();
        assert_eq!(backlog.next_id(), 9);
        assert_eq!(backlog.tasks().len(), 1);
    }

    #[test]
    fn nothing_restored_is_a_backlog_nobody_has_added_to() {
        assert_eq!(
            Backlog::restore(1, Vec::new()),
            Ok(Backlog {
                next_id: 1,
                tasks: Vec::new()
            })
        );
    }

    #[test]
    fn one_number_twice_is_refused() {
        assert_eq!(
            holding(vec![held("1", None, "Pending"), held("1", None, "Pending")]),
            Err(NotABacklog::RepeatedId {
                id: TaskId::parse("1").unwrap()
            })
        );
    }

    #[test]
    fn a_task_waiting_for_one_that_is_not_there_is_refused() {
        assert_eq!(
            holding(vec![held("1", Some("7"), "Pending")]),
            Err(NotABacklog::NoSuchPredecessor {
                task: TaskId::parse("1").unwrap(),
                after: TaskId::parse("7").unwrap()
            })
        );
    }

    /// `task add` cannot build this, since a task may only name one that already exists.
    /// A file edited by hand can, which is the only way here.
    #[test]
    fn two_tasks_waiting_for_each_other_are_refused() {
        assert!(matches!(
            holding(vec![
                held("1", Some("2"), "Pending"),
                held("2", Some("1"), "Pending"),
            ]),
            Err(NotABacklog::Cycle { .. })
        ));
    }

    #[test]
    fn a_task_waiting_for_itself_is_refused() {
        assert_eq!(
            holding(vec![held("1", Some("1"), "Pending")]),
            Err(NotABacklog::Cycle {
                task: TaskId::parse("1").unwrap()
            })
        );
    }

    /// A count with the same figure in every kind, so that a sum that dropped one of them is visible.
    fn spent(each: u64) -> Observation {
        Observation::Spent(Consumption {
            input: each,
            output: each,
            cache_written: each,
            cache_read: each,
            cost: each,
        })
    }

    /// A moment for a test that does not care which one.
    const AT: u64 = 1_700_000_000;

    fn assigned(backlog: &mut Backlog, to: SessionId) -> TaskId {
        registered(backlog, None, None);
        let waiting = backlog.next_to_assign().unwrap();
        backlog.assign(waiting, to, 0, AT).unwrap()
    }

    fn a_session() -> SessionId {
        SessionId::parse("1").unwrap()
    }

    /// The start is stamped when the task is assigned rather than when it registered, since a
    /// task waits in the backlog for as long as it waits and that is not part of its run.
    #[test]
    fn a_task_carries_when_its_run_started_and_when_it_stopped() {
        let mut backlog = Backlog::default();
        let id = assigned(&mut backlog, a_session());
        assert_eq!(backlog.find(id).unwrap().started_at(), Some(AT));
        assert_eq!(backlog.find(id).unwrap().ended_at(), None);

        backlog.finish(id, TaskState::Completed, None, AT + 900);
        assert_eq!(backlog.find(id).unwrap().ended_at(), Some(AT + 900));
    }

    /// A task the vendor turned away runs again, and the second run is the one to measure.
    #[test]
    fn a_task_assigned_again_is_stamped_with_the_run_it_is_starting_now() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let id = assigned(&mut backlog, session);

        backlog.wait_again(id, AT + 60);
        assert_eq!(backlog.find(id).unwrap().ended_at(), Some(AT + 60));

        assert_eq!(backlog.assign(id, session, 0, AT + 300), Some(id));
        let held = backlog.find(id).unwrap();
        assert_eq!(held.started_at(), Some(AT + 300));
        assert_eq!(held.ended_at(), None);
    }

    /// The vendor refusing a run does not refund what the run got through first.
    #[test]
    fn a_task_sent_back_to_waiting_still_counts_against_the_session_that_ran_it() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let id = assigned(&mut backlog, session);
        backlog.record(id, spent(40));

        backlog.wait_again(id, AT + 60);

        assert_eq!(backlog.find(id).unwrap().state(), TaskState::Pending);
        assert_eq!(backlog.consumed_by(session), spent(40));
    }

    /// Assigning it again is what moves it, and the next session is what it costs.
    #[test]
    fn a_task_assigned_again_counts_against_the_session_that_took_it() {
        let mut backlog = Backlog::default();
        let first = a_session();
        let id = assigned(&mut backlog, first);
        backlog.record(id, spent(40));
        backlog.wait_again(id, AT + 60);

        let next = SessionId::parse("2").unwrap();
        assert_eq!(backlog.assign(id, next, 0, AT + 300), Some(id));
        assert_eq!(backlog.consumed_by(first), spent(0));
        assert_eq!(backlog.consumed_by(next), spent(40));
    }

    /// A session stopped by hand ends its tasks, and they took as long as they took.
    #[test]
    fn a_task_interrupted_with_its_session_is_stamped_too() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let id = assigned(&mut backlog, session);

        backlog.interrupt(session, "interrupted", AT + 120);
        assert_eq!(backlog.find(id).unwrap().ended_at(), Some(AT + 120));
    }

    #[test]
    fn what_a_session_consumed_is_what_its_tasks_did() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let first = assigned(&mut backlog, session);
        backlog.finish(first, TaskState::Completed, None, AT);
        let second = assigned(&mut backlog, session);

        backlog.record(first, spent(10));
        backlog.record(second, spent(20));

        assert_eq!(backlog.consumed_by(session), spent(30));
    }

    /// A task another session assigned is that session's, however recently it ran.
    #[test]
    fn what_another_session_consumed_is_not_counted() {
        let mut backlog = Backlog::default();
        let mine = assigned(&mut backlog, a_session());
        backlog.record(mine, spent(10));

        let theirs = SessionId::parse("2").unwrap();
        assert_eq!(backlog.consumed_by(theirs), spent(0));
    }

    #[test]
    fn a_session_none_of_whose_tasks_has_run_consumed_nothing() {
        let mut backlog = Backlog::default();
        let session = a_session();
        assigned(&mut backlog, session);

        assert_eq!(
            backlog.consumed_by(session),
            Observation::Spent(Consumption::default())
        );
    }

    /// A total that quietly dropped this task would look low, not missing.
    #[test]
    fn one_task_that_could_not_be_read_leaves_the_session_unreadable() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let first = assigned(&mut backlog, session);
        backlog.finish(first, TaskState::Completed, None, AT);
        let second = assigned(&mut backlog, session);

        backlog.record(first, spent(10));
        backlog.record(
            second,
            Observation::Unreadable {
                why: "the answer said nothing about it".to_owned(),
            },
        );

        assert_eq!(
            backlog.consumed_by(session),
            Observation::Unreadable {
                why: "the answer said nothing about it".to_owned()
            }
        );
    }

    #[test]
    fn a_registered_task_has_not_consumed_anything_yet() {
        let mut backlog = Backlog::default();
        let id = registered(&mut backlog, None, None);
        assert_eq!(backlog.find(id).unwrap().consumed(), &Observation::NotYet);
    }

    #[test]
    fn every_task_that_ended_and_was_not_decided_about_is_waiting_for_review() {
        let backlog = holding(vec![
            held("1", None, "Pending"),
            held("2", None, "Running"),
            held("3", None, "Completed"),
            held("4", None, "Interrupted"),
            held("5", None, "Error"),
        ])
        .unwrap();

        let waiting: Vec<String> = backlog
            .awaiting_review()
            .iter()
            .map(|task| task.id().labelled())
            .collect();
        assert_eq!(waiting, ["task:3", "task:4", "task:5"]);
    }

    #[test]
    fn a_task_that_was_decided_about_leaves_the_queue() {
        let mut backlog = holding(vec![held("1", None, "Completed")]).unwrap();
        let id = TaskId::parse("1").unwrap();

        backlog.dispose(id, Disposition::Applied).unwrap();
        assert!(backlog.awaiting_review().is_empty());
        assert_eq!(
            backlog.find(id).unwrap().disposition(),
            Some(Disposition::Applied)
        );
    }

    /// Section 2.4 keeps the branch either way, so a discarded result is still there to be applied.
    #[test]
    fn a_discarded_result_can_be_applied_afterwards() {
        let mut backlog = holding(vec![held("1", None, "Completed")]).unwrap();
        let id = TaskId::parse("1").unwrap();

        backlog.dispose(id, Disposition::Discarded).unwrap();
        backlog.dispose(id, Disposition::Applied).unwrap();
        assert_eq!(
            backlog.find(id).unwrap().disposition(),
            Some(Disposition::Applied)
        );
    }

    #[test]
    fn a_run_that_has_not_ended_cannot_be_decided_about() {
        let mut backlog = holding(vec![held("1", None, "Running")]).unwrap();
        assert_eq!(
            backlog.dispose(TaskId::parse("1").unwrap(), Disposition::Applied),
            Err(DisposalRefused::NotEnded)
        );
    }

    #[test]
    fn deciding_about_a_task_nobody_registered_says_so() {
        let mut backlog = Backlog::default();
        assert_eq!(
            backlog.dispose(TaskId::parse("7").unwrap(), Disposition::Applied),
            Err(DisposalRefused::NoSuchTask)
        );
    }

    /// A disposition says nothing about the state, and section 2.4 says `discard` leaves the task state alone.
    #[test]
    fn deciding_about_a_result_leaves_the_state_where_it_was() {
        let mut backlog = holding(vec![held("1", None, "Interrupted")]).unwrap();
        let id = TaskId::parse("1").unwrap();

        backlog.dispose(id, Disposition::Discarded).unwrap();
        assert_eq!(backlog.find(id).unwrap().state(), TaskState::Interrupted);
    }

    #[test]
    fn a_chain_that_ends_is_not_a_cycle() {
        assert!(
            holding(vec![
                held("1", None, "Pending"),
                held("2", Some("1"), "Pending"),
                held("3", Some("2"), "Pending"),
            ])
            .is_ok()
        );
    }

    /// A task cut off at a ceiling ends `Interrupted`, and nothing else moves it back.
    /// `dispose` takes it off the review queue and leaves the state where it was.
    #[test]
    fn a_task_that_ended_can_wait_again() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let id = assigned(&mut backlog, session);
        backlog.finish(
            id,
            TaskState::Interrupted,
            Some("task ceiling".to_owned()),
            AT + 60,
        );

        backlog.try_again(id).unwrap();

        let held = backlog.find(id).unwrap();
        assert_eq!(held.state(), TaskState::Pending);
        assert_eq!(held.reason(), None);
        assert_eq!(held.ceiling(), None);
        // And it is what the next decision would take.
        assert_eq!(backlog.next_to_assign(), Some(id));
    }

    /// A run that is still going is not one to start again.
    #[test]
    fn a_task_still_running_cannot_wait_again() {
        let mut backlog = Backlog::default();
        let id = assigned(&mut backlog, a_session());

        assert_eq!(backlog.try_again(id), Err(DisposalRefused::NotEnded));
    }

    /// Assignments rather than failures. A run cut off at a ceiling leaves no record of its
    /// own, so counting what went wrong counts less than what was tried.
    #[test]
    fn every_assignment_is_counted() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let id = assigned(&mut backlog, session);
        assert_eq!(backlog.find(id).unwrap().attempts(), 1);

        backlog.finish(id, TaskState::Interrupted, None, AT + 60);
        backlog.try_again(id).unwrap();
        backlog.assign(id, session, 0, AT + 120);

        assert_eq!(backlog.find(id).unwrap().attempts(), 2);
    }

    /// Every task left waits on one that did not complete.
    #[test]
    fn a_backlog_whose_successors_all_wait_on_a_task_that_did_not_complete_is_blocked() {
        let mut backlog = Backlog::default();
        let first = registered(&mut backlog, None, None);
        let second = registered(&mut backlog, None, Some(first));
        let session = a_session();
        backlog.assign(first, session, 0, AT);
        backlog.finish(first, TaskState::Interrupted, None, AT + 60);

        assert_eq!(backlog.next_to_assign(), None);
        assert!(
            backlog.blocked(),
            "{second:?} waits on one that did not complete"
        );
    }

    /// An empty backlog is not blocked, it is done.
    #[test]
    fn a_backlog_with_nothing_left_is_not_blocked() {
        let mut backlog = Backlog::default();
        let id = assigned(&mut backlog, a_session());
        backlog.finish(id, TaskState::Completed, None, AT + 60);

        assert!(!backlog.blocked());
    }
}
