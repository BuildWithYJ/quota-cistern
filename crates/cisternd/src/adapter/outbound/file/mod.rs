//! What answers the store ports, through files.
//!
//! One file each, read and written whole.
//! `kept` holds what all of them do with a file; each of the rest holds its own format and its own fields.
//! The two that keep JSON share how a value crosses, which is here.

use serde_json::Value;

pub mod backlog;
pub mod configuration;
mod kept;
pub mod session;
pub mod trace;

/// The text a user would have typed for what JSON holds.
///
/// A string keeps its contents; everything else is rendered as the file writes it.
/// A number, a boolean, and an object all reach the core as something it can read and refuse.
fn as_text(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

/// The same, for a field the file may leave out.
fn as_optional(value: Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(as_text(other)),
    }
}

/// A number where the text is one, so that the file reads as JSON rather than as strings holding digits.
fn as_number(text: &str) -> Value {
    match text.parse::<u64>() {
        Ok(number) => Value::from(number),
        Err(_) => Value::String(text.to_owned()),
    }
}

fn as_value(text: Option<String>) -> Value {
    text.map_or(Value::Null, Value::String)
}
