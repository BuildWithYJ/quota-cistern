//! What answers the store ports, through files.
//!
//! One file each, read and written whole.
//! `kept` holds what all of them do with a file; each of the rest holds its own format and its own fields.
//! The two that keep JSON share how a value crosses, which is here.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::port::outbound::StoredConsumption;

pub mod backlog;
pub mod configuration;
mod kept;
pub mod run;
pub mod session;
pub mod trace;

/// What one run consumed, as both files that keep it hold it.
///
/// The backlog keeps the most recent run of a task and the ledger keeps every run there has
/// been, so the same five figures are written twice. One shape rather than two, since a figure
/// added to one of them and forgotten in the other leaves that file unreadable whole:
/// `deny_unknown_fields` refuses a line it did not expect.
///
/// An object of its own rather than five fields beside the others, so that a task or a run that
/// never counted anything carries no counts at all and a reader can see which of the three
/// states it is in without comparing five keys.
///
/// A value is held as whatever JSON found rather than as what the field is supposed to take,
/// which is what both stores do and for the same reason.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counted {
    input: Value,
    output: Value,
    cache_written: Value,
    cache_read: Value,
    cost: Value,
}

/// What the file holds, as the core takes it.
fn spending(counted: Counted) -> StoredConsumption {
    StoredConsumption {
        input: as_text(counted.input),
        output: as_text(counted.output),
        cache_written: as_text(counted.cache_written),
        cache_read: as_text(counted.cache_read),
        cost: as_text(counted.cost),
    }
}

/// What the core hands over, as the file holds it.
fn counted(spent: &StoredConsumption) -> Counted {
    Counted {
        input: as_number(&spent.input),
        output: as_number(&spent.output),
        cache_written: as_number(&spent.cache_written),
        cache_read: as_number(&spent.cache_read),
        cost: as_number(&spent.cost),
    }
}

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
