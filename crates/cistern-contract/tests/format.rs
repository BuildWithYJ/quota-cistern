//! The format `docs/ipc.md` records, checked the way a surface uses it.

// A test panics to signal failure.
#![allow(clippy::unwrap_used)]

use cistern_contract::{Answer, Failure, FailureTag, Request, Response, VERSION};

// Every line here is copied from docs/ipc.md.
// Breaking one is a change to the format, and the document moves with it.

#[test]
fn request_matches_the_document() {
    let r = Request {
        version: "0.1.0".to_owned(),
        command: "task_show".to_owned(),
        params: serde_json::json!({ "task": 1 }),
    };
    assert_eq!(
        serde_json::to_string(&r).unwrap(),
        r#"{"version":"0.1.0","type":"task_show","params":{"task":1}}"#
    );
}

#[test]
fn answer_matches_the_document() {
    let a = Answer {
        command: "task_show".to_owned(),
        data: serde_json::json!({ "id": 1, "state": "Completed" }),
    };
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        r#"{"type":"task_show","data":{"id":1,"state":"Completed"}}"#
    );
}

#[test]
fn failure_without_data_matches_the_document() {
    let f = Failure {
        kind: FailureTag::Error,
        code: 3,
        message: "no such task".to_owned(),
        data: None,
    };
    assert_eq!(
        serde_json::to_string(&f).unwrap(),
        r#"{"type":"error","code":3,"message":"no such task"}"#
    );
}

#[test]
fn version_refusal_matches_the_document() {
    let f = Failure {
        kind: FailureTag::Error,
        code: 5,
        message: "core is 0.2.0, surface is 0.1.0".to_owned(),
        data: Some(serde_json::json!({ "core": "0.2.0", "surface": "0.1.0" })),
    };
    assert_eq!(
        serde_json::to_string(&f).unwrap(),
        r#"{"type":"error","code":5,"message":"core is 0.2.0, surface is 0.1.0","data":{"core":"0.2.0","surface":"0.1.0"}}"#
    );
}

#[test]
fn a_response_tells_a_failure_from_an_answer() {
    let answer =
        Response::from_line(r#"{"type":"core_version","data":{"version":"0.1.0"}}"#).unwrap();
    assert!(matches!(answer, Response::Data(_)));

    let failure =
        Response::from_line(r#"{"type":"error","code":3,"message":"no such task"}"#).unwrap();
    assert!(matches!(failure, Response::Error(_)));
}

#[test]
fn a_bad_response_names_the_field_that_was_wrong() {
    let e = Response::from_line(r#"{"type":"error","code":3}"#).unwrap_err();
    assert!(e.to_string().contains("message"), "{e}");

    let e = Response::from_line(r#"{"type":"core_version"}"#).unwrap_err();
    assert!(e.to_string().contains("data"), "{e}");
}

#[test]
fn a_message_carries_no_newline() {
    let r = Request {
        version: VERSION.to_owned(),
        command: "task_add".to_owned(),
        params: serde_json::json!({ "instruction": "line one\nline two" }),
    };
    let line = serde_json::to_string(&r).unwrap();
    assert!(!line.contains('\n'));
}
