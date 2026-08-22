//! The envelope for `task add`, `task rm`, `task show`, and `backlog`.

use cistern_contract::{Request, Response};
use serde_json::Value;

use crate::core::port::inbound::{Added, BacklogUseCase, Detail, Listing, Registration, Removed};

use super::{answer, flag, missing, text};

/// Answers the commands this group owns.
///
/// A command it does not own comes back untouched.
/// That way, whoever is routing can offer the request to the next group without keeping a list of names.
pub fn respond(backlog: &impl BacklogUseCase, request: Request) -> Result<Response, Request> {
    match request.command.as_str() {
        "task_add" => Ok(add(backlog, request)),
        "task_rm" => Ok(remove(backlog, request)),
        "task_show" => Ok(show(backlog, request)),
        "backlog" => Ok(list(backlog, request)),
        _ => Err(request),
    }
}

fn add(backlog: &impl BacklogUseCase, request: Request) -> Response {
    // Read before the rest, because a flag that cannot be read is the envelope's business and
    // says nothing about whether the arguments beside it are there.
    let force = match flag(&request, "force") {
        Ok(force) => force,
        Err(refusal) => return answer(request.command, Err(refusal)),
    };

    // cwd is not an argument a user types.
    // A surface adds it, so a request without one was built wrong rather than typed wrong.
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
            force,
        },
        _ => return missing("task_add takes cwd, title, and instruction, all strings"),
    };
    let outcome = backlog.add(given).map(registered);
    answer(request.command, outcome)
}

fn remove(backlog: &impl BacklogUseCase, request: Request) -> Response {
    let Some(id) = text(&request, "task") else {
        return missing("task_rm takes a task, as a string");
    };
    let outcome = backlog.remove(id).map(taken);
    answer(request.command, outcome)
}

fn show(backlog: &impl BacklogUseCase, request: Request) -> Response {
    let Some(id) = text(&request, "task") else {
        return missing("task_show takes a task, as a string");
    };
    let outcome = backlog.show(id).map(detailed);
    answer(request.command, outcome)
}

fn list(backlog: &impl BacklogUseCase, request: Request) -> Response {
    let outcome = backlog.list().map(waiting);
    answer(request.command, outcome)
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
        "instruction": detail.instruction,
        "original": detail.original,
        "base_branch": detail.base_branch,
        "after": detail.after,
        "model": detail.model,
        "repository": detail.repository,
        "branch": detail.branch,
        "reason": detail.reason,
        "worktree": detail.worktree,
        "conversation": detail.conversation,
        "disposition": detail.disposition,
        "commits": detail.commits.map(|made| made.into_iter().map(|one| serde_json::json!({
            "sha": one.sha,
            "subject": one.subject,
            "added": one.added,
            "removed": one.removed,
        })).collect::<Vec<_>>()),
        "base_ahead": detail.base_ahead,
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use cistern_contract::code::{NOT_FOUND, USAGE_ERROR};

    use super::super::tests::{asked, data, failure};
    use crate::core::port::inbound::{Refusal, Waiting};

    use super::*;

    /// Stands in for the core.
    ///
    /// It answers rather than deciding, and records whether it was reached.
    /// That way, what is checked here is the envelope and nothing else.
    /// No store is involved, because no command reaches one through this file.
    #[derive(Default)]
    struct Core {
        /// What every command ends in, when it is meant to end badly.
        refuses: Option<Refusal>,
        reached: Cell<bool>,
        /// What `force` was, on the registration this core was given.
        forced: Cell<Option<bool>>,
    }

    impl Core {
        fn refusing(refusal: Refusal) -> Self {
            Core {
                refuses: Some(refusal),
                ..Core::default()
            }
        }

        fn refused<T>(&self) -> Option<Result<T, Refusal>> {
            self.reached.set(true);
            self.refuses.clone().map(Err)
        }
    }

    impl BacklogUseCase for Core {
        fn add(&self, given: Registration<'_>) -> Result<Added, Refusal> {
            self.forced.set(Some(given.force));
            self.refused().unwrap_or(Ok(Added {
                id: "task:1".to_owned(),
                title: given.title.to_owned(),
                base_branch: "main".to_owned(),
                after: None,
                model: None,
                repository: "/work/api".to_owned(),
                state: "Pending".to_owned(),
            }))
        }

        fn remove(&self, id: &str) -> Result<Removed, Refusal> {
            self.refused().unwrap_or(Ok(Removed {
                id: id.to_owned(),
                title: "refactor X".to_owned(),
            }))
        }

        fn show(&self, id: &str) -> Result<Detail, Refusal> {
            self.refused().unwrap_or(Ok(Detail {
                id: id.to_owned(),
                session: None,
                state: "Pending".to_owned(),
                title: "refactor X".to_owned(),
                instruction: "tidy up src/utils/format.rs".to_owned(),
                original: None,
                base_branch: "main".to_owned(),
                after: None,
                model: None,
                repository: "/work/api".to_owned(),
                branch: None,
                reason: None,
                worktree: None,
                conversation: None,
                disposition: None,
                commits: None,
                base_ahead: None,
            }))
        }

        fn list(&self) -> Result<Listing, Refusal> {
            self.refused().unwrap_or(Ok(Listing {
                items: vec![Waiting {
                    id: "task:1".to_owned(),
                    title: "refactor X".to_owned(),
                    base_branch: "main".to_owned(),
                }],
            }))
        }
    }

    fn registering(title: &str) -> Value {
        serde_json::json!({ "cwd": "/work/api/src", "title": title, "instruction": "do it" })
    }

    /// The same, with whatever a surface put in `force`.
    fn registering_with_force(force: Value) -> Value {
        let mut params = registering("refactor X");
        params["force"] = force;
        params
    }

    fn answered(core: &Core, command: &str, params: Value) -> Response {
        match respond(core, asked(command, params)) {
            Ok(response) => response,
            Err(request) => panic!("this group does not own {}", request.command),
        }
    }

    #[test]
    fn registering_a_task_answers_with_what_was_registered() {
        let core = Core::default();
        let data = data(answered(&core, "task_add", registering("refactor X")));

        assert_eq!(data["id"], "task:1");
        assert_eq!(data["title"], "refactor X");
        assert_eq!(data["base_branch"], "main");
        assert_eq!(data["repository"], "/work/api");
        assert_eq!(data["state"], "Pending");
    }

    /// The path from the envelope to the registration, which nothing else crosses.
    #[test]
    fn a_force_that_is_a_boolean_reaches_the_core_as_one() {
        let core = Core::default();
        data(answered(
            &core,
            "task_add",
            registering_with_force(Value::Bool(true)),
        ));
        assert_eq!(core.forced.get(), Some(true));

        let core = Core::default();
        data(answered(
            &core,
            "task_add",
            registering_with_force(Value::Bool(false)),
        ));
        assert_eq!(core.forced.get(), Some(false));
    }

    /// An argument nobody gave is absent, or null where a surface writes one.
    #[test]
    fn a_force_nobody_gave_reaches_the_core_as_false() {
        let core = Core::default();
        data(answered(&core, "task_add", registering("refactor X")));
        assert_eq!(core.forced.get(), Some(false));

        let core = Core::default();
        data(answered(
            &core,
            "task_add",
            registering_with_force(Value::Null),
        ));
        assert_eq!(core.forced.get(), Some(false));
    }

    /// Reading it as false would turn the task back with nothing said about the value.
    #[test]
    fn a_force_that_is_not_a_boolean_never_reaches_the_core() {
        for value in [
            Value::String("true".to_owned()),
            serde_json::json!(1),
            serde_json::json!([]),
        ] {
            let core = Core::default();
            let refused = failure(answered(
                &core,
                "task_add",
                registering_with_force(value.clone()),
            ));

            assert_eq!(refused.code, USAGE_ERROR, "{value}");
            assert!(refused.message.contains("force"), "{}", refused.message);
            assert!(
                refused.message.contains(&value.to_string()),
                "{}",
                refused.message
            );
            assert!(!core.reached.get(), "the core was reached with {value}");
            assert_eq!(core.forced.get(), None, "{value}");
        }
    }

    /// What `docs/ipc.md` promises about a value of the wrong type, held for both kinds.
    ///
    /// A flag is refused. A string argument is read as one nobody gave, which is what `text`
    /// does and what the page now says it does, so a `branch` that arrived as a number leaves
    /// the task on its default base rather than turning the command back.
    #[test]
    fn a_string_argument_of_another_type_reads_as_one_nobody_gave() {
        let core = Core::default();
        let mut params = registering("refactor X");
        params["branch"] = serde_json::json!(7);

        let registered = data(answered(&core, "task_add", params));

        assert!(core.reached.get(), "the core was never reached");
        assert_eq!(registered["base_branch"], "main");
    }

    #[test]
    fn the_backlog_answers_with_the_tasks_that_are_waiting() {
        let core = Core::default();
        let data = data(answered(&core, "backlog", serde_json::json!({})));

        let items = data["backlog"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["title"], "refactor X");
    }

    #[test]
    fn showing_a_task_answers_with_every_field_section_two_one_lists() {
        let core = Core::default();
        let data = data(answered(
            &core,
            "task_show",
            serde_json::json!({ "task": "1" }),
        ));

        for field in [
            "id",
            "session",
            "state",
            "title",
            "instruction",
            "original",
            "base_branch",
            "after",
            "model",
            "repository",
            "branch",
            "reason",
            "worktree",
            "conversation",
            "disposition",
        ] {
            assert!(data.get(field).is_some(), "{field} is missing");
        }
    }

    #[test]
    fn removing_a_task_answers_with_what_was_taken() {
        let core = Core::default();
        let data = data(answered(
            &core,
            "task_rm",
            serde_json::json!({ "task": "task:1" }),
        ));
        assert_eq!(
            data,
            serde_json::json!({ "id": "task:1", "title": "refactor X" })
        );
    }

    #[test]
    fn a_task_add_without_a_title_never_reaches_the_core() {
        let core = Core::default();
        let refused = failure(answered(
            &core,
            "task_add",
            serde_json::json!({ "cwd": "/work/api", "instruction": "do it" }),
        ));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(!core.reached.get());
    }

    /// A missing cwd is refused the same as a missing typed argument.
    #[test]
    fn a_task_add_without_a_working_directory_never_reaches_the_core() {
        let core = Core::default();
        let refused = failure(answered(
            &core,
            "task_add",
            serde_json::json!({ "title": "refactor X", "instruction": "do it" }),
        ));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(!core.reached.get());
    }

    #[test]
    fn a_task_rm_without_a_task_never_reaches_the_core() {
        let core = Core::default();
        let refused = failure(answered(&core, "task_rm", serde_json::json!({})));
        assert_eq!(refused.code, USAGE_ERROR);
        assert!(!core.reached.get());
    }

    #[test]
    fn a_refusal_carries_the_code_and_the_sentence_it_becomes() {
        let core = Core::refusing(Refusal::NoSuchTask {
            id: "task:7".to_owned(),
        });
        let refused = failure(answered(
            &core,
            "task_show",
            serde_json::json!({ "task": "7" }),
        ));
        assert_eq!(refused.code, NOT_FOUND);
        assert!(refused.message.contains("task:7"), "{}", refused.message);
    }

    #[test]
    fn a_command_this_group_does_not_own_comes_back() {
        let core = Core::default();
        let handed_back = respond(&core, asked("config_get", Value::Null));
        assert!(matches!(handed_back, Err(request) if request.command == "config_get"));
    }
}
