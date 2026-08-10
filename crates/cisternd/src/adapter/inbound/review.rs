//! The envelope for `diff`, `review ls`, `apply`, and `discard`.

use cistern_contract::{Request, Response};
use serde_json::Value;

use crate::core::port::inbound::{
    Changed, Difference, Dropped, Queue, Refusal, ReviewUseCase, Taken,
};

use super::{answer, missing, text};

/// Answers the commands this group owns.
pub fn respond(review: &impl ReviewUseCase, request: Request) -> Result<Response, Request> {
    match request.command.as_str() {
        "diff" => Ok(about(request, |id| review.diff(id).map(differed))),
        "review_ls" => Ok(list(review, request)),
        "apply" => Ok(about(request, |id| review.apply(id).map(applied))),
        "discard" => Ok(about(request, |id| review.discard(id).map(discarded))),
        _ => Err(request),
    }
}

fn list(review: &impl ReviewUseCase, request: Request) -> Response {
    let outcome = review.queue().map(queued);
    answer(request.command, outcome)
}

/// The three commands that take a task and nothing else.
fn about(request: Request, ask: impl FnOnce(&str) -> Result<Value, Refusal>) -> Response {
    let Some(id) = text(&request, "task") else {
        return missing(&format!("{} takes a task, as a string", request.command));
    };
    let outcome = ask(id);
    answer(request.command, outcome)
}

fn differed(difference: Difference) -> Value {
    serde_json::json!({
        "base": difference.base,
        "branch": difference.branch,
        "files": files(difference.files),
        "patch": difference.patch,
    })
}

fn queued(queue: Queue) -> Value {
    let items: Vec<Value> = queue
        .items
        .into_iter()
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "title": item.title,
                "session": item.session,
                "branch": item.branch,
                "state": item.state,
                "commit_count": item.commit_count,
                "base_ahead": item.base_ahead,
            })
        })
        .collect();
    serde_json::json!({ "review_queue": items })
}

fn applied(taken: Taken) -> Value {
    serde_json::json!({
        "task": taken.task,
        "branch": taken.branch,
        "files": files(taken.files),
    })
}

fn discarded(dropped: Dropped) -> Value {
    serde_json::json!({ "task": dropped.task, "branch": dropped.branch })
}

fn files(changed: Vec<Changed>) -> Vec<Value> {
    changed
        .into_iter()
        .map(|file| {
            serde_json::json!({
                "path": file.path,
                "added": file.added,
                "removed": file.removed,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use cistern_contract::code::{GENERAL_FAILURE, NOT_FOUND, STATE_CONFLICT, USAGE_ERROR};

    use super::super::tests::{asked, data, failure};
    use crate::core::port::inbound::{Awaiting, Refusal};

    use super::*;

    /// Stands in for the core. It answers rather than deciding, so what is
    /// checked here is the envelope and nothing else.
    #[derive(Default)]
    struct Core {
        refuses: Option<Refusal>,
        reached: Cell<bool>,
    }

    impl Core {
        fn refusing(refusal: Refusal) -> Self {
            Core {
                refuses: Some(refusal),
                reached: Cell::new(false),
            }
        }

        fn refused<T>(&self) -> Option<Result<T, Refusal>> {
            self.reached.set(true);
            self.refuses.clone().map(Err)
        }
    }

    impl ReviewUseCase for Core {
        fn diff(&self, _id: &str) -> Result<Difference, Refusal> {
            self.refused().unwrap_or(Ok(Difference {
                base: "main".to_owned(),
                branch: Some("cistern/5".to_owned()),
                files: vec![Changed {
                    path: "src/webhook/verify.ts".to_owned(),
                    added: Some(64),
                    removed: Some(3),
                }],
                patch: "diff --git a b".to_owned(),
            }))
        }

        fn queue(&self) -> Result<Queue, Refusal> {
            self.refused().unwrap_or(Ok(Queue {
                items: vec![Awaiting {
                    id: "task:5".to_owned(),
                    title: "verify webhook signature".to_owned(),
                    session: Some("session:1".to_owned()),
                    branch: Some("cistern/5".to_owned()),
                    state: "Completed".to_owned(),
                    commit_count: Some(3),
                    base_ahead: Some(0),
                }],
            }))
        }

        fn apply(&self, id: &str) -> Result<Taken, Refusal> {
            self.refused().unwrap_or(Ok(Taken {
                task: id.to_owned(),
                branch: "cistern/5".to_owned(),
                files: Vec::new(),
            }))
        }

        fn discard(&self, id: &str) -> Result<Dropped, Refusal> {
            self.refused().unwrap_or(Ok(Dropped {
                task: id.to_owned(),
                branch: "cistern/5".to_owned(),
            }))
        }
    }

    fn answered(core: &Core, command: &str, params: Value) -> Response {
        match respond(core, asked(command, params)) {
            Ok(response) => response,
            Err(request) => panic!("this group does not own {}", request.command),
        }
    }

    fn asking(task: &str) -> Value {
        serde_json::json!({ "task": task })
    }

    #[test]
    fn a_difference_answers_with_every_field_section_two_three_lists() {
        let core = Core::default();
        let data = data(answered(&core, "diff", asking("5")));

        assert_eq!(data["base"], "main");
        assert_eq!(data["branch"], "cistern/5");
        assert_eq!(data["patch"], "diff --git a b");
        let files = data["files"].as_array().unwrap();
        assert_eq!(files[0]["path"], "src/webhook/verify.ts");
        assert_eq!(files[0]["added"], 64);
        assert_eq!(files[0]["removed"], 3);
    }

    #[test]
    fn the_queue_answers_with_every_field_section_two_four_lists() {
        let core = Core::default();
        let data = data(answered(&core, "review_ls", serde_json::json!({})));

        let items = data["review_queue"].as_array().unwrap();
        for field in [
            "id",
            "title",
            "session",
            "branch",
            "state",
            "commit_count",
            "base_ahead",
        ] {
            assert!(items[0].get(field).is_some(), "{field} is missing");
        }
    }

    #[test]
    fn applying_answers_with_what_was_disposed_of() {
        let core = Core::default();
        let data = data(answered(&core, "apply", asking("task:5")));
        assert_eq!(data["task"], "task:5");
        assert_eq!(data["branch"], "cistern/5");
    }

    #[test]
    fn discarding_answers_with_the_branch_that_stays() {
        let core = Core::default();
        let data = data(answered(&core, "discard", asking("task:6")));
        assert_eq!(
            data,
            serde_json::json!({ "task": "task:6", "branch": "cistern/5" })
        );
    }

    #[test]
    fn a_command_without_a_task_never_reaches_the_core() {
        for command in ["diff", "apply", "discard"] {
            let core = Core::default();
            let refused = failure(answered(&core, command, serde_json::json!({})));
            assert_eq!(refused.code, USAGE_ERROR, "{command}");
            assert!(!core.reached.get(), "{command}");
        }
    }

    /// Section 2.4 gives each of these its own code, and the sentence has to
    /// say which of them happened.
    #[test]
    fn each_way_a_disposal_is_refused_carries_its_own_code() {
        let refusals = [
            (
                Refusal::NotEnded {
                    id: "task:5".to_owned(),
                },
                STATE_CONFLICT,
            ),
            (
                Refusal::Uncommitted {
                    at: "/work/api".to_owned(),
                },
                STATE_CONFLICT,
            ),
            (
                Refusal::NoChange {
                    id: "task:5".to_owned(),
                },
                GENERAL_FAILURE,
            ),
            (
                Refusal::AlreadyApplied {
                    id: "task:5".to_owned(),
                },
                GENERAL_FAILURE,
            ),
            (
                Refusal::Conflicts {
                    why: "patch failed".to_owned(),
                },
                GENERAL_FAILURE,
            ),
            (
                Refusal::NoResult {
                    branch: "cistern/5".to_owned(),
                    at: "/work/api".to_owned(),
                },
                GENERAL_FAILURE,
            ),
            (
                Refusal::NoRepository {
                    at: "/work/api".to_owned(),
                },
                GENERAL_FAILURE,
            ),
            (
                Refusal::NoSuchTask {
                    id: "task:7".to_owned(),
                },
                NOT_FOUND,
            ),
        ];

        for (refusal, code) in refusals {
            let core = Core::refusing(refusal.clone());
            let refused = failure(answered(&core, "apply", asking("5")));
            assert_eq!(refused.code, code, "{refusal:?}");
            assert!(!refused.message.is_empty(), "{refusal:?}");
        }
    }

    #[test]
    fn a_command_this_group_does_not_own_comes_back() {
        let core = Core::default();
        let handed_back = respond(&core, asked("backlog", Value::Null));
        assert!(matches!(handed_back, Err(request) if request.command == "backlog"));
    }
}
