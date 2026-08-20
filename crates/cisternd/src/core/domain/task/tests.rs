use super::*;

fn held(id: &str, after: Option<&str>, state: &str) -> Restored {
    Restored {
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

/// The one thing that tells doing the work over from carrying it on.
///
/// Both put the task back where it started and leave the branch and the work area alone.
/// What differs is the conversation its last run was in: carrying on keeps it, so the next
/// run picks that conversation up instead of reading everything back.
#[test]
fn carrying_on_keeps_the_conversation_and_trying_again_drops_it() {
    let waiting_again = |carrying_on: bool| {
        let mut backlog = Backlog::default();
        let id = assigned(&mut backlog, a_session());
        backlog.finish(id, TaskState::Interrupted, None, AT + 60);
        backlog.conversed(id, Some("a-conversation".to_owned()));

        match carrying_on {
            true => backlog.carries_on(id).unwrap(),
            false => backlog.try_again(id).unwrap(),
        }
        let held = backlog.find(id).unwrap();
        assert_eq!(held.state(), TaskState::Pending);
        held.conversation().map(str::to_owned)
    };

    assert_eq!(waiting_again(true).as_deref(), Some("a-conversation"));
    assert_eq!(waiting_again(false), None);
}

/// Nobody carries on a conversation about work that has been decided.
#[test]
fn disposing_of_a_result_lets_its_conversation_go() {
    let mut backlog = Backlog::default();
    let id = assigned(&mut backlog, a_session());
    backlog.finish(id, TaskState::Interrupted, None, AT + 60);
    backlog.conversed(id, Some("a-conversation".to_owned()));

    backlog.dispose(id, Disposition::Discarded).unwrap();

    assert_eq!(backlog.find(id).unwrap().conversation(), None);
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
