//! A task and the rules over a backlog of them.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output, and section
//! 1 fixes the identifiers and the states. This module is private, so a value
//! that reached here was parsed on the way in.

use std::fmt::{self, Display};

use super::{Consumption, Observation, SessionId};

/// The branch a task starts from when it names neither a branch nor a
/// predecessor.
const DEFAULT_BRANCH: &str = "main";

/// A task number.
///
/// The core issues these. They only increase and are never reused, and
/// sessions count on a sequence of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(u32);

/// The five states section 1 lists.
///
/// Only `Pending` is reachable until something runs a task. The rest are
/// declared now because the rule that only a `Pending` task may be removed
/// cannot be written without them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running,
    Completed,
    Interrupted,
    Error,
}

/// The repository a task was added from.
///
/// The core never reads this as a path. It does not join, open, or walk it; it
/// keeps what an adapter handed over and hands it back. Only an adapter treats
/// it as a place on a disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository(String);

/// What a task was registered with.
///
/// `branch` and `after` are separate because they answer separate questions.
/// `after` decides when the task may be assigned and `branch` decides where it
/// starts from, so a task may carry both, one, or neither.
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
    /// Why it ended as it did, for a task that did not simply finish.
    reason: Option<String>,
    /// What running it consumed, as far as that is known.
    consumed: Observation,
}

/// A task on its way back from a store, with every value already read.
///
/// The domain does not know what a store keeps or how it spells things, so
/// whoever read it names the fields once here and the backlog checks them
/// together.
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
    pub reason: Option<String>,
    pub consumed: Observation,
}

/// Every task, and the number the next one will get.
///
/// The next number is kept rather than derived. Taking the highest number
/// present and adding one would hand out the number of a removed newest task
/// again, which section 1 forbids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backlog {
    next_id: u32,
    tasks: Vec<Task>,
}

/// A set of tasks that no backlog could be.
///
/// Each of these takes the whole set to see. A value that could not be read is
/// not here, because reading one is not the domain's work.
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

impl TaskId {
    /// Reads an identifier as a user writes it.
    ///
    /// `docs/cli.md` lets the `task:` prefix be left off in arguments, so both
    /// spellings are read here and only the number is kept.
    pub fn parse(id: &str) -> Option<Self> {
        let digits = id.strip_prefix("task:").unwrap_or(id);
        digits.parse().ok().map(TaskId)
    }

    /// The identifier as section 1 writes it.
    ///
    /// [`Display`] gives the number alone, which is what a branch name is built
    /// from. Everything a caller is shown carries the prefix, and both
    /// spellings come from here so they cannot drift apart.
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
    /// Reads a state name. One spelling is read and written, so what a store
    /// holds and what is printed cannot drift apart.
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
    /// [`Task::base_branch`] is where the task starts from, which is this only
    /// when a branch was named.
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

    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// What running it consumed, as far as that is known.
    pub fn consumed(&self) -> &Observation {
        &self.consumed
    }

    /// The branch this task's result is kept on, once one has been cut.
    ///
    /// A task that was never assigned has none, which is what section 2.1
    /// reports as null.
    pub fn result_branch(&self) -> Option<String> {
        match self.state {
            TaskState::Pending => None,
            _ => Some(result_branch_of(self.id)),
        }
    }

    /// Where the task starts from.
    ///
    /// Derived rather than stored, because a predecessor's result branch does
    /// not exist when the task is registered. Naming a branch wins over a
    /// predecessor, so a task can wait for one result and start from another.
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
/// A predecessor's is where a task that waits for it starts from, so both
/// spellings come from here and cannot drift apart.
fn result_branch_of(id: TaskId) -> String {
    format!("cistern/{id}")
}

impl Backlog {
    /// Registers a task and hands back the number it was given.
    pub fn add(
        &mut self,
        title: String,
        instruction: String,
        branch: Option<String>,
        after: Option<TaskId>,
        model: Option<String>,
        repository: Repository,
    ) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id += 1;
        self.tasks.push(Task {
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
            reason: None,
            consumed: Observation::NotYet,
        });
        id
    }

    /// Takes a task out of the backlog.
    ///
    /// Whatever waited for it now waits for what it waited for. Leaving those
    /// alone would name a task that is not there, which is the one thing a
    /// stored backlog may not do, so removing one would make a file this core
    /// cannot read.
    ///
    /// The next number is left where it is. A number that was handed out is
    /// spent whether or not the task it named is still here.
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
    /// A task waiting for one that has not completed is not eligible, and
    /// neither is one that is no longer waiting to be assigned. Nothing is
    /// answered when none may start, which is what an empty backlog and a
    /// backlog of blocked tasks both look like from here.
    pub fn assign(&mut self, to: SessionId) -> Option<TaskId> {
        let id = self.next_to_assign()?;
        for task in &mut self.tasks {
            if task.id == id {
                task.state = TaskState::Running;
                task.session = Some(to);
            }
        }
        Some(id)
    }

    /// The task `assign` would hand over, without handing it over.
    ///
    /// A run that has nothing to start is refused before a session is opened,
    /// and asking that question is what this is for.
    pub fn next_to_assign(&self) -> Option<TaskId> {
        self.tasks
            .iter()
            .filter(|task| task.state == TaskState::Pending)
            .find(|task| match task.after {
                None => true,
                Some(after) => self
                    .find(after)
                    .is_some_and(|held| held.state == TaskState::Completed),
            })
            .map(|task| task.id)
    }

    /// Records where a task is being worked on.
    pub fn work_area(&mut self, id: TaskId, at: String) {
        for task in &mut self.tasks {
            if task.id == id {
                task.worktree = Some(at.clone());
            }
        }
    }

    /// Moves a task to the state it ended in.
    ///
    /// Every terminal state leaves the branch alone, so what changes here is
    /// what the task says about itself.
    ///
    /// Only a task that is still running ends. A task interrupted by hand
    /// ends the moment the user asks, and the thread that was waiting on its
    /// agent comes back afterwards to say the agent failed. The first answer
    /// is the true one.
    pub fn finish(&mut self, id: TaskId, state: TaskState, reason: Option<String>) {
        for task in &mut self.tasks {
            if task.id == id && task.state == TaskState::Running {
                task.state = state;
                task.reason.clone_from(&reason);
            }
        }
    }

    /// Records what running a task consumed.
    ///
    /// Kept apart from [`Backlog::finish`] because a task can end without ever
    /// having run, and what it consumed is then not nothing but unknown.
    pub fn record(&mut self, id: TaskId, consumed: Observation) {
        for task in &mut self.tasks {
            if task.id == id {
                task.consumed = consumed.clone();
            }
        }
    }

    /// What a session has consumed, added up over the tasks it assigned.
    ///
    /// Derived rather than kept beside the session, so that the figure cannot
    /// disagree with the tasks it is the sum of.
    ///
    /// One task whose answer could not be read leaves the whole sum unreadable.
    /// What is missing from a total is not visible in the total, and a session
    /// held to a budget it cannot measure would be held to the wrong one. A
    /// session none of whose tasks has run has consumed nothing, which is a
    /// figure rather than a gap.
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
    /// For a task nobody would run. It keeps its number and whatever it was
    /// registered with, and waits for a session that can start it.
    pub fn wait_again(&mut self, id: TaskId) {
        for task in &mut self.tasks {
            if task.id == id {
                task.state = TaskState::Pending;
                task.session = None;
                task.reason = None;
            }
        }
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
    /// Whatever the agent committed by then is on the branch already, so
    /// nothing is undone here. Section 1 says an interrupted task keeps its
    /// partial work.
    pub fn interrupt(&mut self, session: SessionId, reason: &str) -> Vec<TaskId> {
        let mut ended = Vec::new();
        for task in &mut self.tasks {
            if task.session == Some(session) && task.state == TaskState::Running {
                task.state = TaskState::Interrupted;
                task.reason = Some(reason.to_owned());
                ended.push(task.id);
            }
        }
        ended
    }

    /// What each task that reported a count consumed, over the whole backlog.
    ///
    /// A task from an earlier session counts. Tokens mean the same thing
    /// whenever they were spent, and a session that has run nothing of its own
    /// yet can still tell what a task around here costs.
    pub fn counted(&self) -> Vec<Consumption> {
        self.tasks
            .iter()
            .filter_map(|task| match &task.consumed {
                Observation::Spent(spent) => Some(*spent),
                _ => None,
            })
            .collect()
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

    /// How many of a session's tasks have ended, whatever they ended as.
    pub fn ended_in(&self, session: SessionId) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.session == Some(session) && task.state != TaskState::Running)
            .count()
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
    /// Every value has been read by the time it arrives here, so what is left
    /// is what only the whole set can show: whether the numbers are distinct,
    /// whether every predecessor is present, and whether following them ever
    /// returns to where it started.
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
                reason: held.reason,
                consumed: held.consumed,
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

    /// `task add` cannot build a cycle, since a task may only name one that
    /// already exists and identifiers only increase. A file edited by hand can,
    /// which is why this is checked here rather than where the argument
    /// arrives.
    ///
    /// Walking at most as many steps as there are tasks is enough: a chain
    /// longer than that has visited something twice.
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
            reason: None,
            consumed: Observation::NotYet,
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
        backlog.add(
            "a task".to_owned(),
            "do it".to_owned(),
            branch.map(str::to_owned),
            after,
            None,
            Repository::new("/work/api"),
        )
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

    /// The two answer different questions, so a task can wait for one result
    /// and start from another.
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

    /// Section 1 says a number is never reused. Deriving the next one from the
    /// tasks present would hand this one out again.
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

    /// The branch the removed task would have produced is never made, so what
    /// waited for it starts from where that one would have.
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

    /// No command produces another state yet, so the task is built here. The
    /// rule is what is being checked, not the path that reaches it.
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

    /// `task add` cannot build this, since a task may only name one that
    /// already exists. A file edited by hand can, which is the only way here.
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

    /// A count with the same figure in every kind, so that a sum that dropped
    /// one of them is visible.
    fn spent(each: u64) -> Observation {
        Observation::Spent(Consumption {
            input: each,
            output: each,
            cache_written: each,
            cache_read: each,
            cost: each,
        })
    }

    fn assigned(backlog: &mut Backlog, to: SessionId) -> TaskId {
        registered(backlog, None, None);
        backlog.assign(to).unwrap()
    }

    fn a_session() -> SessionId {
        SessionId::parse("1").unwrap()
    }

    #[test]
    fn what_a_session_consumed_is_what_its_tasks_did() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let first = assigned(&mut backlog, session);
        backlog.finish(first, TaskState::Completed, None);
        let second = assigned(&mut backlog, session);

        backlog.record(first, spent(10));
        backlog.record(second, spent(20));

        assert_eq!(backlog.consumed_by(session), spent(30));
    }

    /// A task another session assigned is that session's, however recently it
    /// ran.
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

    /// What is missing from a total is not visible in the total, so one task
    /// nobody could read leaves the whole figure unreadable rather than low.
    #[test]
    fn one_task_that_could_not_be_read_leaves_the_session_unreadable() {
        let mut backlog = Backlog::default();
        let session = a_session();
        let first = assigned(&mut backlog, session);
        backlog.finish(first, TaskState::Completed, None);
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
}
