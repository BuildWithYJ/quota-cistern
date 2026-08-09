//! The `run` command.
//!
//! What was typed goes to the core as it was given. Whether a budget can be
//! read and whether a session may open are the core's to decide, so this file
//! judges neither.

use std::process::ExitCode;

use cistern_contract::{Response, code::CORE_ERROR, exchange};
use serde_json::Value;

pub fn run(usage: &str, time: &str, model: Option<String>) -> ExitCode {
    send(
        "run",
        serde_json::json!({
            "usage": usage,
            "time": time,
            "model": model,
        }),
        started,
    )
}

/// Asks the core and prints what came back.
fn send(command: &str, params: Value, print: fn(&Value)) -> ExitCode {
    match exchange::ask(command, params) {
        Ok(Response::Data(answer)) => {
            print(&answer.data);
            ExitCode::SUCCESS
        }
        Ok(Response::Error(failure)) => {
            eprintln!("cistern: {}", failure.message);
            ExitCode::from(failure.code)
        }
        Err(e) => {
            eprintln!("cistern: the core is not running: {e}");
            ExitCode::from(CORE_ERROR)
        }
    }
}

/// The layout section 2.2 shows.
const STARTED: usize = 9;

/// The separator section 2.2 puts between the two halves of the budget.
///
/// Written as an escape because a source file here holds ASCII only.
const BETWEEN: &str = "\u{b7}";

fn started(data: &Value) {
    let Some(session) = data.get("session").and_then(Value::as_str) else {
        return;
    };
    let assigned = data.get("assigned").and_then(Value::as_u64).unwrap_or(0);
    let state = data
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("(none)");
    println!("{session} {state} ({})", assigned_as(assigned));

    let budget = data.get("budget");
    let field = |name: &str| {
        budget
            .and_then(|budget| budget.get(name))
            .and_then(Value::as_str)
            .unwrap_or("(none)")
    };
    println!(
        "  {:<STARTED$}usage {} {} time {}",
        "budget:",
        field("usage"),
        BETWEEN,
        field("time")
    );
    println!("  {:<STARTED$}cistern trace <task> --follow", "observe:");
    println!("  {:<STARTED$}cistern interrupt", "stop:");
}

/// Section 2.2 counts tasks in the line it prints, and one is not "1 tasks".
fn assigned_as(assigned: u64) -> String {
    match assigned {
        1 => "1 task assigned to start".to_owned(),
        other => format!("{other} tasks assigned to start"),
    }
}
