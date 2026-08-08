//! A task and the rules over a backlog of them.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output, and section
//! 1 fixes the identifiers and the states. This module is private, so a value
//! that reached here was parsed on the way in.

use std::fmt::{self, Display};

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

    /// Where the task starts from.
    ///
    /// Derived rather than stored, because a predecessor's result branch does
    /// not exist when the task is registered. Naming a branch wins over a
    /// predecessor, so a task can wait for one result and start from another.
    pub fn base_branch(&self) -> String {
        match (&self.branch, self.after) {
            (Some(branch), _) => branch.clone(),
            (None, Some(after)) => format!("cistern/{after}"),
            (None, None) => DEFAULT_BRANCH.to_owned(),
        }
    }
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
        });
        id
    }

    /// Takes a task out of the backlog.
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
        Ok(self.tasks.remove(at))
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
