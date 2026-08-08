//! The `task` commands and `backlog`.
//!
//! What was typed goes to the core as it was given. Whether a value is allowed
//! and whether a task exists are the core's to decide, so this file has no list
//! of either.

use std::{env, ffi::OsString, io::Read, process::ExitCode};

use cistern_contract::{Response, code::CORE_ERROR, code::USAGE_ERROR};
use serde_json::Value;

use crate::{cli::TaskCommand, link};

/// The mark `docs/cli.md` puts beside a task waiting to be assigned.
///
/// Written as an escape because a source file here holds ASCII only. What
/// reaches the terminal is the character the specification shows.
const WAITING: &str = "\u{25cb}";

pub fn run(command: TaskCommand) -> ExitCode {
    match command {
        TaskCommand::Add {
            title,
            instruction,
            branch,
            after,
            model,
        } => add(&title, &instruction, branch, after, model),
        TaskCommand::Rm { task } => send("task_rm", serde_json::json!({ "task": task }), removed),
        TaskCommand::Show { task } => {
            send("task_show", serde_json::json!({ "task": task }), detailed)
        }
    }
}

pub fn backlog() -> ExitCode {
    send("backlog", serde_json::json!({}), waiting)
}

fn add(
    title: &str,
    instruction: &str,
    branch: Option<String>,
    after: Option<String>,
    model: Option<String>,
) -> ExitCode {
    // The core runs as a daemon, so where it was started is not where this
    // command was. It cannot learn that from anywhere but here.
    let Ok(cwd) = env::current_dir() else {
        eprintln!("cistern: the current directory cannot be read");
        return ExitCode::from(USAGE_ERROR);
    };
    let instruction = match read_instruction(instruction) {
        Ok(instruction) => instruction,
        Err(e) => {
            eprintln!("cistern: the instruction cannot be read: {e}");
            return ExitCode::from(USAGE_ERROR);
        }
    };

    send(
        "task_add",
        serde_json::json!({
            "cwd": cwd.display().to_string(),
            "title": title,
            "instruction": instruction,
            "branch": branch,
            "after": after,
            "model": model,
        }),
        added,
    )
}

/// `docs/cli.md` gives `-` the meaning of standard input, so that an
/// instruction longer than a line does not have to fit in an argument.
fn read_instruction(given: &str) -> std::io::Result<String> {
    if given != "-" {
        return Ok(given.to_owned());
    }
    let mut read = String::new();
    std::io::stdin().read_to_string(&mut read)?;
    Ok(read)
}

/// Asks the core and prints what came back.
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

/// The layout section 2.1 shows, which is narrower than the one `task show`
/// uses because it has no label as long as `disposition`.
const REGISTERED: usize = 8;
const DETAILED: usize = 13;

fn added(data: &Value) {
    let Some(id) = text(data, "id") else { return };
    println!("{id} added to backlog");
    line(REGISTERED, "title", text(data, "title"));
    println!(
        "  {:<REGISTERED$}{} (base)",
        "branch:",
        text(data, "base_branch").unwrap_or("(none)")
    );
    line(
        REGISTERED,
        "repo",
        home_relative(text(data, "repository")).as_deref(),
    );
}

fn removed(data: &Value) {
    if let Some(id) = text(data, "id") {
        println!("{id} removed from backlog");
    }
}

fn detailed(data: &Value) {
    let Some(id) = text(data, "id") else { return };
    println!("{id}  {}", text(data, "state").unwrap_or("(none)"));
    line(DETAILED, "session", text(data, "session"));
    line(DETAILED, "title", text(data, "title"));
    line(DETAILED, "base", text(data, "base_branch"));
    line(DETAILED, "after", text(data, "after"));
    line(
        DETAILED,
        "repo",
        home_relative(text(data, "repository")).as_deref(),
    );
    line(DETAILED, "branch", text(data, "branch"));
    line(DETAILED, "reason", text(data, "reason"));
    line(DETAILED, "worktree", text(data, "worktree"));
    line(DETAILED, "disposition", text(data, "disposition"));
}

fn waiting(data: &Value) {
    let Some(items) = data.get("backlog").and_then(Value::as_array) else {
        return;
    };
    let width = items
        .iter()
        .filter_map(|item| text(item, "id"))
        .map(str::len)
        .max()
        .unwrap_or(0);

    for item in items {
        let (Some(id), Some(title)) = (text(item, "id"), text(item, "title")) else {
            continue;
        };
        let base = text(item, "base_branch").unwrap_or("(none)");
        println!("{WAITING} {id:<width$}  {title:<20}  base {base}");
    }
}

/// One line of a labelled layout.
///
/// `docs/cli.md` shows a value that is absent in parentheses, so a field that
/// holds nothing is still printed.
fn line(width: usize, label: &str, value: Option<&str>) {
    println!(
        "  {:<width$}{}",
        format!("{label}:"),
        value.unwrap_or("(none)")
    );
}

fn home_relative(path: Option<&str>) -> Option<String> {
    Some(under_home(path?, env::var_os("HOME")))
}

/// A path under the home directory, written the way a user would.
///
/// Only what is printed changes. The core keeps the path it was given, and
/// `-o json` will carry that one.
///
/// The home directory is an argument rather than a read, so that both outcomes
/// can be checked without setting a variable the whole process sees.
fn under_home(path: &str, home: Option<OsString>) -> String {
    let Some(home) = home else {
        return path.to_owned();
    };
    match path.strip_prefix(&*home.to_string_lossy()) {
        Some(rest) => format!("~{rest}"),
        None => path.to_owned(),
    }
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(path: &str) -> Option<OsString> {
        Some(OsString::from(path))
    }

    #[test]
    fn a_path_under_the_home_directory_is_written_with_a_tilde() {
        assert_eq!(
            under_home("/home/a/work/api", home("/home/a")),
            "~/work/api"
        );
    }

    #[test]
    fn a_path_elsewhere_is_left_alone() {
        assert_eq!(under_home("/srv/api", home("/home/a")), "/srv/api");
    }

    #[test]
    fn no_home_leaves_every_path_alone() {
        assert_eq!(under_home("/home/a/work/api", None), "/home/a/work/api");
    }
}
