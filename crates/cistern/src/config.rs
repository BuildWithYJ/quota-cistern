//! The `config` command.
//!
//! What was typed goes to the core as it was given. Whether a key exists and
//! whether a value belongs to it are the core's to decide, so this file has no
//! list of either.

use std::process::ExitCode;

use cistern_contract::{Response, code::CORE_ERROR};
use serde_json::Value;

use crate::{cli::ConfigCommand, link};

pub fn run(command: ConfigCommand) -> ExitCode {
    match command {
        ConfigCommand::Set { key, value } => send(
            "config_set",
            serde_json::json!({ "key": key, "value": value }),
            said,
        ),
        ConfigCommand::Get { key } => send("config_get", serde_json::json!({ "key": key }), shown),
    }
}

/// Asks the core and prints what came back.
///
/// A refusal carries the code `docs/cli.md` gives it, so the surface exits
/// with the core's answer rather than deciding one of its own.
fn send(command: &str, params: Value, print: fn(&Value)) -> ExitCode {
    match link::ask(command, params) {
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

/// What `set` applied.
fn said(data: &Value) {
    if let (Some(key), Some(value)) = (text(data, "key"), text(data, "value")) {
        println!("{key} = {value}");
    }
}

/// One key, or every key that holds something.
///
/// A key that holds nothing prints nothing, which is also what an empty
/// configuration prints. Both are a successful read of nothing.
fn shown(data: &Value) {
    if let Some(value) = text(data, "value") {
        println!("{value}");
        return;
    }

    let Some(entries) = data.get("entries").and_then(Value::as_array) else {
        return;
    };
    let width = entries
        .iter()
        .filter_map(|entry| text(entry, "key"))
        .map(str::len)
        .max()
        .unwrap_or(0);

    for entry in entries {
        if let (Some(key), Some(value)) = (text(entry, "key"), text(entry, "value")) {
            // The colon belongs to the key, so the values line up under it.
            println!("{:width$} {value}", format!("{key}:"), width = width + 1);
        }
    }
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_holding_nothing_is_told_from_a_key_holding_a_value() {
        let held = serde_json::json!({ "key": "plan", "value": "pro" });
        let empty = serde_json::json!({ "key": "plan", "value": null });
        assert_eq!(text(&held, "value"), Some("pro"));
        assert_eq!(text(&empty, "value"), None);
    }
}
