//! Where a surface and the core reach each other, and the envelope between them.
//!
//! `docs/ipc.md` records this format, and becomes its authority once a surface is not Rust.

pub mod address;
pub mod code;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The release version this build carries.
/// A surface sends it on every request and the core compares it with its own.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A message from a surface to the core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// The release version of the surface that sent this.
    pub version: String,
    /// The command, in snake_case: `task add` is `task_add`.
    #[serde(rename = "type")]
    pub command: String,
    /// The command's arguments.
    /// What belongs here is the argument table in that command's section of `docs/cli.md`.
    pub params: Value,
}

/// A message from the core to a surface.
/// A response of more than one line carries one of these per line.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Response {
    Error(Failure),
    Data(Answer),
}

impl Response {
    /// Reads one response line.
    ///
    /// An untagged enum reports only that nothing matched.
    /// Deciding on `type` first lets the error name the field that was wrong.
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Tagged {
            #[serde(rename = "type")]
            kind: String,
        }

        let Tagged { kind } = serde_json::from_str(line)?;
        if kind == "error" {
            serde_json::from_str(line).map(Response::Error)
        } else {
            serde_json::from_str(line).map(Response::Data)
        }
    }
}

/// The core answered the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    /// The `command` of the request being answered.
    #[serde(rename = "type")]
    pub command: String,
    /// The command's output fields.
    /// What belongs here is the output table in that command's section of `docs/cli.md`.
    pub data: Value,
}

/// The core refused or failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Failure {
    /// Always `error`. It is what tells a failure from an answer.
    #[serde(rename = "type")]
    pub kind: FailureTag,
    /// An exit code from `docs/cli.md`. The surface exits with it.
    pub code: u8,
    /// What went wrong, in one sentence.
    pub message: String,
    /// Fields some failures carry, described where they arise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Failure {
    /// A refusal carrying nothing but a code and a reason.
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Failure {
            kind: FailureTag::Error,
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Adds the fields some refusals carry.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// The `type` of a [`Failure`].
/// Holding one value is what makes it refuse to deserialize from anything else.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FailureTag {
    #[serde(rename = "error")]
    Error,
}
