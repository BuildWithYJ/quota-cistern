//! The envelope for `run` and the two session queries.

use cistern_contract::{Request, Response};
use serde_json::Value;

use crate::core::port::inbound::{Declaration, ExecutionUseCase, Page, Report, Started};

use super::{answer, missing, text};

/// Answers the commands this group owns, for the reason
/// `inbound::configuration` gives.
pub fn respond(execution: &impl ExecutionUseCase, request: Request) -> Result<Response, Request> {
    match request.command.as_str() {
        "run" => Ok(run(execution, request)),
        "session_ls" => Ok(list(execution, request)),
        "session_show" => Ok(show(execution, request)),
        _ => Err(request),
    }
}

fn run(execution: &impl ExecutionUseCase, request: Request) -> Response {
    // A model is optional. Null is how a surface says it named none, so only a
    // model that is present and is neither is malformed.
    if request
        .params
        .get("model")
        .is_some_and(|model| !model.is_string() && !model.is_null())
    {
        return missing("the model run was given is not a string");
    }

    let outcome = match (text(&request, "usage"), text(&request, "time")) {
        (Some(usage), Some(time)) => execution
            .run(Declaration {
                usage,
                time,
                model: text(&request, "model"),
            })
            .map(started),
        _ => return missing("run takes a usage and a time, both strings"),
    };
    answer(request.command, outcome)
}

fn list(execution: &impl ExecutionUseCase, request: Request) -> Response {
    let outcome = execution
        .sessions(text(&request, "page"), text(&request, "limit"))
        .map(page);
    answer(request.command, outcome)
}

fn show(execution: &impl ExecutionUseCase, request: Request) -> Response {
    let Some(session) = text(&request, "session") else {
        return missing("session show takes a session, a string");
    };
    let outcome = execution.session(session).map(report);
    answer(request.command, outcome)
}

fn page(page: Page) -> Value {
    serde_json::json!({
        "page": page.page,
        "limit": page.limit,
        "sessions": page.sessions.into_iter().map(|one| serde_json::json!({
            "id": one.id,
            "state": one.state,
            "consumed": one.consumed,
            "task_count": one.task_count,
            "updated_at": one.updated_at,
        })).collect::<Vec<_>>(),
    })
}

fn report(report: Report) -> Value {
    serde_json::json!({
        "session": report.session,
        "state": report.state,
        "budget": { "usage": report.budget.usage, "time": report.budget.time },
        "consumed": { "usage": report.consumed.usage, "time": report.consumed.time },
        "stopped_reason": report.stopped_reason,
        "resets_at": report.resets_at,
        "updated_at": report.updated_at,
        "tasks": report.tasks.into_iter().map(|one| serde_json::json!({
            "id": one.id,
            "state": one.state,
            "title": one.title,
            "branch": one.branch,
            "reason": one.reason,
        })).collect::<Vec<_>>(),
    })
}

fn started(started: Started) -> Value {
    serde_json::json!({
        "session": started.session,
        "state": started.state,
        "assigned": started.assigned.len(),
        "budget": {
            "usage": started.budget.usage,
            "time": started.budget.time,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cistern_contract::code::{STATE_CONFLICT, USAGE_ERROR};

    use crate::core::port::inbound::{Declared, Refusal};

    use super::super::tests::{asked, data, failure};
    use super::*;

    /// A core that answers whatever it was told to, and remembers what it was
    /// asked. The adapter is what is under test, so nothing behind it runs.
    struct Answering {
        outcome: Result<Started, Refusal>,
        asked: RefCell<Option<(String, String, Option<String>)>>,
    }

    impl Answering {
        fn with(outcome: Result<Started, Refusal>) -> Self {
            Answering {
                outcome,
                asked: RefCell::new(None),
            }
        }
    }

    impl ExecutionUseCase for Answering {
        fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal> {
            *self.asked.borrow_mut() = Some((
                declared.usage.to_owned(),
                declared.time.to_owned(),
                declared.model.map(str::to_owned),
            ));
            self.outcome.clone()
        }

        /// Nothing here calls this: the adapter answers `run` and the queue is
        /// what reaches this, which is the composition root's.
        fn carry_on(&self, _task: &str) -> Result<Vec<String>, Refusal> {
            Ok(Vec::new())
        }

        fn sessions(&self, _page: Option<&str>, _limit: Option<&str>) -> Result<Page, Refusal> {
            Ok(Page {
                page: 1,
                limit: 20,
                sessions: Vec::new(),
            })
        }

        fn session(&self, id: &str) -> Result<Report, Refusal> {
            Err(Refusal::NoSuchSession { id: id.to_owned() })
        }
    }

    fn a_start() -> Started {
        Started {
            session: "session:1".to_owned(),
            state: "running".to_owned(),
            assigned: Vec::new(),
            budget: Declared {
                usage: "50%".to_owned(),
                time: "8h".to_owned(),
            },
        }
    }

    #[test]
    fn a_command_this_group_does_not_own_comes_back_untouched() {
        let execution = Answering::with(Ok(a_start()));
        let request = asked("config_get", Value::Null);

        assert!(respond(&execution, request).is_err());
    }

    #[test]
    fn what_the_core_answered_is_the_shape_section_2_2_gives() {
        let execution = Answering::with(Ok(a_start()));
        let answered = data(
            respond(
                &execution,
                asked("run", serde_json::json!({"usage": "50%", "time": "8h"})),
            )
            .unwrap(),
        );

        assert_eq!(answered["session"], "session:1");
        assert_eq!(answered["state"], "running");
        assert_eq!(answered["assigned"], 0);
        assert_eq!(answered["budget"]["usage"], "50%");
        assert_eq!(answered["budget"]["time"], "8h");
    }

    #[test]
    fn the_model_reaches_the_core_when_one_was_named() {
        let execution = Answering::with(Ok(a_start()));
        respond(
            &execution,
            asked(
                "run",
                serde_json::json!({"usage": "50%", "time": "8h", "model": "haiku"}),
            ),
        )
        .unwrap();

        let asked = execution.asked.borrow().clone().unwrap();
        assert_eq!(asked.2, Some("haiku".to_owned()));
    }

    #[test]
    fn an_envelope_without_a_budget_never_reaches_the_core() {
        let execution = Answering::with(Ok(a_start()));
        let refused = failure(
            respond(
                &execution,
                asked("run", serde_json::json!({"usage": "50%"})),
            )
            .unwrap(),
        );

        assert_eq!(refused.code, USAGE_ERROR);
        assert!(execution.asked.borrow().is_none());
    }

    #[test]
    fn a_model_that_is_not_a_string_never_reaches_the_core() {
        let execution = Answering::with(Ok(a_start()));
        let refused = failure(
            respond(
                &execution,
                asked(
                    "run",
                    serde_json::json!({"usage": "50%", "time": "8h", "model": 7}),
                ),
            )
            .unwrap(),
        );

        assert_eq!(refused.code, USAGE_ERROR);
        assert!(execution.asked.borrow().is_none());
    }

    #[test]
    fn a_session_already_running_is_the_code_section_2_2_gives_it() {
        let execution = Answering::with(Err(Refusal::AlreadyRunning {
            id: "session:1".to_owned(),
        }));
        let refused = failure(
            respond(
                &execution,
                asked("run", serde_json::json!({"usage": "50%", "time": "8h"})),
            )
            .unwrap(),
        );

        assert_eq!(refused.code, STATE_CONFLICT);
    }
}
