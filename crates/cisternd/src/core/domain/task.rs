//! A task and the rules over a backlog of them.
//!
//! Section 2.1 of `docs/cli.md` fixes the arguments and the output, and section
//! 1 fixes the identifiers and the states. This module is private, so a value
//! that reached here was parsed on the way in.

use std::fmt::{self, Display};

use crate::core::port::{StoredBacklog, StoredTask};

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

/// A store handed back something no backlog could be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotABacklog {
    /// A field held a value its shape does not allow.
    BadValue { field: String, value: String },
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
    fn parse(state: &str) -> Option<Self> {
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

/// One spelling for what a store holds and what is printed, so the two cannot
/// drift apart.
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

    /// Every task waiting to be assigned, in the order they were registered.
    pub fn pending(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|task| task.state == TaskState::Pending)
            .collect()
    }

    /// Reads what a store handed back.
    ///
    /// A store holds text, not entities, and a backlog file can be edited by
    /// hand, so what comes back is read the same way an argument is.
    pub fn from_stored(stored: StoredBacklog) -> Result<Self, NotABacklog> {
        let next_id = number("next_id", &stored.next_id)?;
        let mut tasks = Vec::with_capacity(stored.tasks.len());
        for held in stored.tasks {
            tasks.push(task_from(held)?);
        }

        let backlog = Backlog { next_id, tasks };
        backlog.no_repeated_id()?;
        backlog.every_predecessor_exists()?;
        backlog.no_cycle()?;
        Ok(backlog)
    }

    /// Hands the backlog to a store as text.
    pub fn to_stored(&self) -> StoredBacklog {
        StoredBacklog {
            next_id: self.next_id.to_string(),
            tasks: self
                .tasks
                .iter()
                .map(|task| StoredTask {
                    id: task.id.to_string(),
                    title: task.title.clone(),
                    instruction: task.instruction.clone(),
                    branch: task.branch.clone(),
                    after: task.after.map(|after| after.to_string()),
                    model: task.model.clone(),
                    repository: task.repository.0.clone(),
                    state: task.state.to_string(),
                })
                .collect(),
        }
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

fn task_from(held: StoredTask) -> Result<Task, NotABacklog> {
    Ok(Task {
        id: identifier("id", &held.id)?,
        title: held.title,
        instruction: held.instruction,
        branch: held.branch,
        after: held
            .after
            .as_deref()
            .map(|after| identifier("after", after))
            .transpose()?,
        model: held.model,
        repository: Repository(held.repository),
        state: TaskState::parse(&held.state).ok_or_else(|| NotABacklog::BadValue {
            field: "state".to_owned(),
            value: held.state.clone(),
        })?,
    })
}

fn identifier(field: &str, value: &str) -> Result<TaskId, NotABacklog> {
    TaskId::parse(value).ok_or_else(|| NotABacklog::BadValue {
        field: field.to_owned(),
        value: value.to_owned(),
    })
}

fn number(field: &str, value: &str) -> Result<u32, NotABacklog> {
    value.parse().map_err(|_| NotABacklog::BadValue {
        field: field.to_owned(),
        value: value.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(id: &str, after: Option<&str>, state: &str) -> StoredTask {
        StoredTask {
            id: id.to_owned(),
            title: "a task".to_owned(),
            instruction: "do it".to_owned(),
            branch: None,
            after: after.map(str::to_owned),
            model: None,
            repository: "/work/api".to_owned(),
            state: state.to_owned(),
        }
    }

    fn holding(tasks: Vec<StoredTask>) -> StoredBacklog {
        StoredBacklog {
            next_id: "9".to_owned(),
            tasks,
        }
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
        let mut backlog = Backlog::from_stored(holding(vec![held("1", None, "Running")])).unwrap();
        let running = TaskId::parse("1").unwrap();
        assert_eq!(backlog.remove(running), Err(RemovalRefused::NotPending));
        assert!(backlog.find(running).is_some());
    }

    #[test]
    fn only_pending_tasks_are_waiting() {
        let backlog = Backlog::from_stored(holding(vec![
            held("1", None, "Pending"),
            held("2", None, "Completed"),
        ]))
        .unwrap();
        assert_eq!(backlog.pending().len(), 1);
    }

    #[test]
    fn what_goes_to_a_store_comes_back_the_same() {
        let mut backlog = Backlog::default();
        registered(&mut backlog, Some("develop"), None);
        let first = registered(&mut backlog, None, None);
        registered(&mut backlog, None, Some(first));
        assert_eq!(Backlog::from_stored(backlog.to_stored()), Ok(backlog));
    }

    #[test]
    fn an_empty_store_reads_as_a_backlog_nobody_has_added_to() {
        let stored = StoredBacklog {
            next_id: "1".to_owned(),
            tasks: Vec::new(),
        };
        assert_eq!(
            Backlog::from_stored(stored),
            Ok(Backlog {
                next_id: 1,
                tasks: Vec::new()
            })
        );
    }

    #[test]
    fn a_stored_state_no_task_can_be_in_is_refused() {
        let stored = holding(vec![held("1", None, "Sleeping")]);
        assert_eq!(
            Backlog::from_stored(stored),
            Err(NotABacklog::BadValue {
                field: "state".to_owned(),
                value: "Sleeping".to_owned()
            })
        );
    }

    #[test]
    fn a_stored_identifier_that_is_not_a_number_is_refused() {
        let stored = holding(vec![held("first", None, "Pending")]);
        assert!(matches!(
            Backlog::from_stored(stored),
            Err(NotABacklog::BadValue { .. })
        ));
    }

    #[test]
    fn a_store_holding_one_number_twice_is_refused() {
        let stored = holding(vec![held("1", None, "Pending"), held("1", None, "Pending")]);
        assert_eq!(
            Backlog::from_stored(stored),
            Err(NotABacklog::RepeatedId {
                id: TaskId::parse("1").unwrap()
            })
        );
    }

    #[test]
    fn a_task_waiting_for_one_the_store_does_not_hold_is_refused() {
        let stored = holding(vec![held("1", Some("7"), "Pending")]);
        assert_eq!(
            Backlog::from_stored(stored),
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
        let stored = holding(vec![
            held("1", Some("2"), "Pending"),
            held("2", Some("1"), "Pending"),
        ]);
        assert!(matches!(
            Backlog::from_stored(stored),
            Err(NotABacklog::Cycle { .. })
        ));
    }

    #[test]
    fn a_task_waiting_for_itself_is_refused() {
        let stored = holding(vec![held("1", Some("1"), "Pending")]);
        assert_eq!(
            Backlog::from_stored(stored),
            Err(NotABacklog::Cycle {
                task: TaskId::parse("1").unwrap()
            })
        );
    }

    #[test]
    fn a_chain_that_ends_is_not_a_cycle() {
        let stored = holding(vec![
            held("1", None, "Pending"),
            held("2", Some("1"), "Pending"),
            held("3", Some("2"), "Pending"),
        ]);
        assert!(Backlog::from_stored(stored).is_ok());
    }
}
