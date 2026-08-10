//! The `run` command and the two session queries.
//!
//! What was typed goes to the core as it was given. Whether a budget can be
//! read and whether a session may open are the core's to decide, so this file
//! judges neither.

use std::process::ExitCode;

use cistern_contract::{Response, code::CORE_ERROR, exchange};
use serde_json::Value;

use crate::cli::SessionCommand;

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

pub fn query(command: SessionCommand) -> ExitCode {
    match command {
        SessionCommand::Ls { page, limit } => send(
            "session_ls",
            serde_json::json!({ "page": page, "limit": limit }),
            listed,
        ),
        SessionCommand::Show { session } => send(
            "session_show",
            serde_json::json!({ "session": session }),
            reported,
        ),
    }
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

/// What section 2.2 puts before a task's result branch.
const TOWARDS: &str = "\u{2192}";

/// What section 2.2 puts before a task, one mark per terminal state.
const FINISHED: &str = "\u{2713}";
const CUT_SHORT: &str = "\u{26a0}";
const FAILED: &str = "\u{2715}";
const STILL_GOING: &str = "\u{25cb}";

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

/// The width `session show` lays its two figures out in.
const REPORTED: usize = 10;

fn listed(data: &Value) {
    let Some(sessions) = data.get("sessions").and_then(Value::as_array) else {
        return;
    };
    for one in sessions {
        let field = |name: &str| one.get(name).and_then(Value::as_str).unwrap_or("(none)");
        let count = one.get("task_count").and_then(Value::as_u64).unwrap_or(0);
        println!(
            "{:<10} {:<10} usage {:<8} {:<8} {}",
            field("id"),
            field("state"),
            field("consumed"),
            tasks_as(count),
            since(field("updated_at"))
        );
    }
}

fn reported(data: &Value) {
    let Some(session) = data.get("session").and_then(Value::as_str) else {
        return;
    };
    let state = data
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("(none)");
    match data.get("stopped_reason").and_then(Value::as_str) {
        Some(why) => println!("{session}  {state} ({why})"),
        None => println!("{session}  {state}"),
    }

    for (label, held) in [("budget:", "budget"), ("consumed:", "consumed")] {
        let field = |name: &str| {
            data.get(held)
                .and_then(|held| held.get(name))
                .and_then(Value::as_str)
                .unwrap_or("(none)")
        };
        println!(
            "  {:<REPORTED$}usage {} {} time {}",
            label,
            field("usage"),
            BETWEEN,
            field("time")
        );
    }

    if let Some(at) = data.get("resets_at").and_then(Value::as_str) {
        println!("  {:<REPORTED$}{at}", "resets at:");
    }

    let Some(tasks) = data.get("tasks").and_then(Value::as_array) else {
        return;
    };
    if tasks.is_empty() {
        return;
    }
    println!("  tasks:");
    for one in tasks {
        let field = |name: &str| one.get(name).and_then(Value::as_str).unwrap_or("(none)");
        let reason = match one.get("reason").and_then(Value::as_str) {
            Some(why) => format!("  ({why})"),
            None => String::new(),
        };
        println!(
            "    {}  {:<8} {:<12} {:<16} {TOWARDS} {}{reason}",
            marked(field("state")),
            field("id"),
            field("state"),
            field("title"),
            field("branch")
        );
    }
}

/// The mark section 2.2 puts before a task, which says how it ended.
fn marked(state: &str) -> &'static str {
    match state {
        "Completed" => FINISHED,
        "Interrupted" => CUT_SHORT,
        "Error" => FAILED,
        _ => STILL_GOING,
    }
}

/// How long ago a moment was, in the words section 2.2 prints.
///
/// The surface reads its own clock. What the core answers with is the moment
/// itself, so nothing has to be recomputed when it is read again.
fn since(at: &str) -> String {
    let Ok(at) = at.parse::<u64>() else {
        return "(none)".to_owned();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let ago = now.saturating_sub(at);
    match ago {
        0..60 => "just now".to_owned(),
        60..3_600 => format!("{}m ago", ago / 60),
        3_600..86_400 => format!("{}h ago", ago / 3_600),
        _ => format!("{}d ago", ago / 86_400),
    }
}

/// Section 2.2 counts tasks in the line it prints, and one is not "1 tasks".
fn tasks_as(count: u64) -> String {
    match count {
        1 => "1 task".to_owned(),
        other => format!("{other} tasks"),
    }
}

/// Section 2.2 counts tasks in the line it prints, and one is not "1 tasks".
fn assigned_as(assigned: u64) -> String {
    match assigned {
        1 => "1 task assigned to start".to_owned(),
        other => format!("{other} tasks assigned to start"),
    }
}
