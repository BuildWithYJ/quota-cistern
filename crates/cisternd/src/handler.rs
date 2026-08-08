//! What answers a request.
//!
//! `docs/ipc.md` records the envelope and `docs/cli.md` the commands. Only
//! what is implemented appears here.

use cistern_contract::{
    Answer, Failure, Request, Response,
    code::{CORE_ERROR, NOT_FOUND, STATE_CONFLICT, USAGE_ERROR},
};
use serde_json::Value;

use crate::core::{
    Added, Applied, Detail, Listing, Refusal, Removed, View,
    port::outbound::{BacklogStore, ConfigurationStore, RepositoryRoots},
    service::{self, Registration},
};

/// Answers one request.
///
/// Every port a command may need arrives here, since this is where a request
/// is routed to one. Gathering them into a single value is a change of its own
/// and is not made here.
///
/// A request of another version never arrives, and neither does `core_version`.
/// Both depend on the envelope alone, so `cistern_contract::exchange` settles
/// them before this is reached.
pub fn respond(
    settings: &impl ConfigurationStore,
    tasks: &impl BacklogStore,
    repository: &impl RepositoryRoots,
    request: Request,
) -> Response {
    match request.command.as_str() {
        "config_set" => {
            let outcome = match (text(&request, "key"), text(&request, "value")) {
                (Some(key), Some(value)) => service::set(settings, key, value).map(said),
                _ => return missing("config_set takes a key and a value, both strings"),
            };
            answer(request.command, outcome)
        }
        "config_get" => {
            // A key is optional here. Null is how a surface says it has none,
            // so only a key that is present and is neither is malformed.
            if request
                .params
                .get("key")
                .is_some_and(|k| !k.is_string() && !k.is_null())
            {
                return missing("the key config_get was given is not a string");
            }
            let outcome = service::get(settings, text(&request, "key")).map(shown);
            answer(request.command, outcome)
        }
        "task_add" => {
            // cwd is not an argument a user types. A surface adds it, so a
            // request without one was built wrong rather than typed wrong.
            let given = match (
                text(&request, "cwd"),
                text(&request, "title"),
                text(&request, "instruction"),
            ) {
                (Some(cwd), Some(title), Some(instruction)) => Registration {
                    cwd,
                    title,
                    instruction,
                    branch: text(&request, "branch"),
                    after: text(&request, "after"),
                    model: text(&request, "model"),
                },
                _ => return missing("task_add takes cwd, title, and instruction, all strings"),
            };
            let outcome = service::add(tasks, repository, given).map(registered);
            answer(request.command, outcome)
        }
        "task_rm" => {
            let Some(id) = text(&request, "task") else {
                return missing("task_rm takes a task, as a string");
            };
            let outcome = service::remove(tasks, id).map(taken);
            answer(request.command, outcome)
        }
        "task_show" => {
            let Some(id) = text(&request, "task") else {
                return missing("task_show takes a task, as a string");
            };
            let outcome = service::show(tasks, id).map(detailed);
            answer(request.command, outcome)
        }
        "backlog" => {
            let outcome = service::list(tasks).map(waiting);
            answer(request.command, outcome)
        }
        other => Response::Error(Failure::new(
            USAGE_ERROR,
            format!("unknown request type {other}"),
        )),
    }
}

fn text<'a>(request: &'a Request, field: &str) -> Option<&'a str> {
    request.params.get(field)?.as_str()
}

/// A request whose envelope does not carry what the command needs.
///
/// It never reaches the core, because what arrived is the envelope's business
/// and the envelope is read here.
fn missing(why: &str) -> Response {
    Response::Error(Failure::new(USAGE_ERROR, why.to_owned()))
}

/// Puts what the core decided into the envelope it travels in.
fn answer(command: String, outcome: Result<Value, Refusal>) -> Response {
    match outcome {
        Ok(data) => Response::Data(Answer { command, data }),
        Err(refusal) => Response::Error(Failure::new(code_for(&refusal), message_for(&refusal))),
    }
}

/// Which exit code a refusal becomes.
///
/// The core says only what was wrong, so this is the one place the two meet.
fn code_for(refusal: &Refusal) -> u8 {
    match refusal {
        Refusal::UnknownKey { .. } | Refusal::BadValue { .. } => USAGE_ERROR,
        Refusal::NoSuchTask { .. } => NOT_FOUND,
        Refusal::NotPending { .. } | Refusal::NotARepository { .. } => STATE_CONFLICT,
        Refusal::Unavailable { .. } => CORE_ERROR,
    }
}

fn message_for(refusal: &Refusal) -> String {
    match refusal {
        Refusal::UnknownKey { key } => format!("no such key {key}"),
        Refusal::BadValue { key, value } => format!("{key} does not take {value}"),
        Refusal::NoSuchTask { id } => format!("{id} does not exist"),
        Refusal::NotPending { id } => format!("{id} is not pending"),
        Refusal::NotARepository { at } => format!("{at} is not inside a repository"),
        Refusal::Unavailable { reason } => format!("the store cannot be read: {reason}"),
    }
}

fn said(applied: Applied) -> Value {
    serde_json::json!({ "key": applied.key, "value": applied.value })
}

fn registered(added: Added) -> Value {
    serde_json::json!({
        "id": added.id,
        "title": added.title,
        "base_branch": added.base_branch,
        "after": added.after,
        "model": added.model,
        "repository": added.repository,
        "state": added.state,
    })
}

fn taken(removed: Removed) -> Value {
    serde_json::json!({ "id": removed.id, "title": removed.title })
}

fn detailed(detail: Detail) -> Value {
    serde_json::json!({
        "id": detail.id,
        "session": detail.session,
        "state": detail.state,
        "title": detail.title,
        "base_branch": detail.base_branch,
        "after": detail.after,
        "model": detail.model,
        "repository": detail.repository,
        "branch": detail.branch,
        "reason": detail.reason,
        "worktree": detail.worktree,
        "disposition": detail.disposition,
    })
}

fn waiting(listing: Listing) -> Value {
    let items: Vec<Value> = listing
        .items
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "title": item.title,
                "base_branch": item.base_branch,
            })
        })
        .collect();
    serde_json::json!({ "backlog": items })
}

fn shown(view: View) -> Value {
    match view {
        View::One { key, value } => serde_json::json!({ "key": key, "value": value }),
        View::All { entries } => {
            let entries: Vec<Value> = entries
                .into_iter()
                .map(|(key, value)| serde_json::json!({ "key": key, "value": value }))
                .collect();
            serde_json::json!({ "entries": entries })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cistern_contract::VERSION;

    use crate::core::port::outbound::{StoredBacklog, Unavailable};

    use super::*;

    /// Everything a request may reach, held together.
    ///
    /// `respond` needs every port, so a test that only exercises one still has
    /// to hand over the rest.
    #[derive(Default)]
    struct Fakes {
        settings: RememberedSettings,
        tasks: RememberedTasks,
        repository: Somewhere,
    }

    impl Fakes {
        /// A backlog is registered from somewhere that is a repository.
        fn in_a_repository() -> Self {
            Fakes {
                repository: Somewhere {
                    root: Some("/work/api".to_owned()),
                },
                ..Default::default()
            }
        }

        fn answer(&self, request: Request) -> Response {
            respond(&self.settings, &self.tasks, &self.repository, request)
        }
    }

    #[derive(Default)]
    struct RememberedSettings {
        stored: RefCell<crate::core::port::outbound::StoredConfiguration>,
    }

    impl ConfigurationStore for RememberedSettings {
        fn load(&self) -> Result<crate::core::port::outbound::StoredConfiguration, Unavailable> {
            Ok(self.stored.borrow().clone())
        }

        fn store(
            &self,
            stored: &crate::core::port::outbound::StoredConfiguration,
        ) -> Result<(), Unavailable> {
            *self.stored.borrow_mut() = stored.clone();
            Ok(())
        }
    }

    struct RememberedTasks {
        stored: RefCell<StoredBacklog>,
    }

    impl Default for RememberedTasks {
        fn default() -> Self {
            RememberedTasks {
                stored: RefCell::new(StoredBacklog {
                    next_id: "1".to_owned(),
                    tasks: Vec::new(),
                }),
            }
        }
    }

    impl BacklogStore for RememberedTasks {
        fn load(&self) -> Result<StoredBacklog, Unavailable> {
            Ok(self.stored.borrow().clone())
        }

        fn store(&self, backlog: &StoredBacklog) -> Result<(), Unavailable> {
            *self.stored.borrow_mut() = backlog.clone();
            Ok(())
        }
    }

    /// Answers with one root, or with none for anywhere outside a repository.
    #[derive(Default)]
    struct Somewhere {
        root: Option<String>,
    }

    impl RepositoryRoots for Somewhere {
        fn root_of(&self, _from: &str) -> Result<Option<String>, Unavailable> {
            Ok(self.root.clone())
        }
    }

    fn registering(title: &str) -> Value {
        serde_json::json!({ "cwd": "/work/api/src", "title": title, "instruction": "do it" })
    }

    fn asked(command: &str, params: Value) -> Request {
        Request {
            version: VERSION.to_owned(),
            command: command.to_owned(),
            params,
        }
    }

    fn failure(response: Response) -> Failure {
        match response {
            Response::Error(failure) => failure,
            Response::Data(answer) => panic!("expected a refusal, got {answer:?}"),
        }
    }

    fn data(response: Response) -> Value {
        match response {
            Response::Data(answer) => answer.data,
            Response::Error(failure) => panic!("expected an answer, got {failure:?}"),
        }
    }

    #[test]
    fn a_command_the_core_does_not_know_is_a_usage_error() {
        let refusal = failure(Fakes::default().answer(asked("banana", Value::Null)));
        assert_eq!(refusal.code, USAGE_ERROR);
        assert!(refusal.message.contains("banana"), "{}", refusal.message);
    }

    #[test]
    fn setting_a_key_answers_with_what_was_stored() {
        let fakes = Fakes::default();
        let response = fakes.answer(asked(
            "config_set",
            serde_json::json!({ "key": "plan", "value": "max-20x" }),
        ));
        assert_eq!(
            data(response),
            serde_json::json!({ "key": "plan", "value": "max-20x" })
        );
    }

    #[test]
    fn reading_one_key_answers_with_its_value() {
        let fakes = Fakes::default();
        fakes.answer(asked(
            "config_set",
            serde_json::json!({ "key": "vendor", "value": "claude" }),
        ));
        let response = fakes.answer(asked("config_get", serde_json::json!({ "key": "vendor" })));
        assert_eq!(
            data(response),
            serde_json::json!({ "key": "vendor", "value": "claude" })
        );
    }

    #[test]
    fn reading_with_no_key_answers_with_every_key_that_holds_something() {
        let fakes = Fakes::default();
        fakes.answer(asked(
            "config_set",
            serde_json::json!({ "key": "plan", "value": "pro" }),
        ));
        let response = fakes.answer(asked("config_get", serde_json::json!({})));
        assert_eq!(
            data(response),
            serde_json::json!({ "entries": [{ "key": "plan", "value": "pro" }] })
        );
    }

    /// A surface passes what it was given without checking it, so this is what
    /// reaching the core directly looks like.
    #[test]
    fn a_bad_value_sent_straight_to_the_core_is_refused() {
        let refusal = failure(Fakes::default().answer(asked(
            "config_set",
            serde_json::json!({ "key": "vendor", "value": "codex" }),
        )));
        assert_eq!(refusal.code, USAGE_ERROR);
        assert!(refusal.message.contains("codex"), "{}", refusal.message);
    }

    #[test]
    fn an_unknown_key_is_a_usage_error() {
        let refusal = failure(Fakes::default().answer(asked(
            "config_set",
            serde_json::json!({ "key": "colour", "value": "red" }),
        )));
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    #[test]
    fn a_request_without_the_fields_the_command_needs_never_reaches_the_core() {
        let refusal = failure(
            Fakes::default().answer(asked("config_set", serde_json::json!({ "key": "plan" }))),
        );
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    #[test]
    fn a_null_key_is_how_a_surface_says_it_has_none() {
        let fakes = Fakes::default();
        let response = fakes.answer(asked("config_get", serde_json::json!({ "key": null })));
        assert_eq!(data(response), serde_json::json!({ "entries": [] }));
    }

    #[test]
    fn a_key_that_is_not_a_string_is_told_from_no_key_at_all() {
        let refusal =
            failure(Fakes::default().answer(asked("config_get", serde_json::json!({ "key": 7 }))));
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    #[test]
    fn registering_a_task_answers_with_what_was_stored() {
        let fakes = Fakes::in_a_repository();
        let response = fakes.answer(asked("task_add", registering("refactor X")));

        let data = data(response);
        assert_eq!(data["id"], "task:1");
        assert_eq!(data["title"], "refactor X");
        assert_eq!(data["base_branch"], "main");
        assert_eq!(data["repository"], "/work/api");
        assert_eq!(data["state"], "Pending");
    }

    #[test]
    fn the_backlog_answers_with_what_was_registered() {
        let fakes = Fakes::in_a_repository();
        fakes.answer(asked("task_add", registering("refactor X")));

        let data = data(fakes.answer(asked("backlog", serde_json::json!({}))));
        let items = data["backlog"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "refactor X");
    }

    #[test]
    fn an_empty_backlog_answers_with_an_empty_list() {
        let fakes = Fakes::in_a_repository();
        let data = data(fakes.answer(asked("backlog", serde_json::json!({}))));
        assert_eq!(data["backlog"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn showing_a_task_answers_with_every_field_section_two_one_lists() {
        let fakes = Fakes::in_a_repository();
        fakes.answer(asked("task_add", registering("refactor X")));

        let data = data(fakes.answer(asked("task_show", serde_json::json!({ "task": "1" }))));
        for field in [
            "id",
            "session",
            "state",
            "title",
            "base_branch",
            "after",
            "model",
            "repository",
            "branch",
            "reason",
            "worktree",
            "disposition",
        ] {
            assert!(data.get(field).is_some(), "{field} is missing");
        }
    }

    #[test]
    fn removing_a_task_answers_with_what_was_taken() {
        let fakes = Fakes::in_a_repository();
        fakes.answer(asked("task_add", registering("refactor X")));

        let data = data(fakes.answer(asked("task_rm", serde_json::json!({ "task": "task:1" }))));
        assert_eq!(
            data,
            serde_json::json!({ "id": "task:1", "title": "refactor X" })
        );
    }

    #[test]
    fn a_task_add_without_a_title_never_reaches_the_core() {
        let refusal = failure(Fakes::in_a_repository().answer(asked(
            "task_add",
            serde_json::json!({ "cwd": "/work/api", "instruction": "do it" }),
        )));
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    /// cwd is not an argument a user types. A request without one was built
    /// wrong rather than typed wrong, so it is refused here.
    #[test]
    fn a_task_add_without_a_working_directory_never_reaches_the_core() {
        let refusal = failure(Fakes::in_a_repository().answer(asked(
            "task_add",
            serde_json::json!({ "title": "refactor X", "instruction": "do it" }),
        )));
        assert_eq!(refusal.code, USAGE_ERROR);
    }

    #[test]
    fn a_task_nobody_registered_is_not_found() {
        let refusal = failure(
            Fakes::in_a_repository().answer(asked("task_show", serde_json::json!({ "task": "7" }))),
        );
        assert_eq!(refusal.code, NOT_FOUND);
        assert!(refusal.message.contains("task:7"), "{}", refusal.message);
    }

    #[test]
    fn registering_outside_a_repository_is_a_state_conflict() {
        // The default fake answers with no root, which is what anywhere outside
        // a repository looks like.
        let refusal =
            failure(Fakes::default().answer(asked("task_add", registering("refactor X"))));
        assert_eq!(refusal.code, STATE_CONFLICT);
    }

    /// A surface passes what it was given without checking it, so this is what
    /// reaching the core directly looks like.
    #[test]
    fn an_identifier_that_is_not_a_number_sent_straight_to_the_core_is_refused() {
        let refusal = failure(
            Fakes::in_a_repository()
                .answer(asked("task_rm", serde_json::json!({ "task": "seven" }))),
        );
        assert_eq!(refusal.code, USAGE_ERROR);
    }
}
