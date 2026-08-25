//! The `task` commands and `backlog`.
//!
//! What was typed goes to the core as it was given.
//! Whether a value is allowed and whether a task exists are the core's to decide, so this file has no list of either.

use std::{env, ffi::OsString, io::Read, process::ExitCode};

use cistern_contract::{Response, code::CORE_ERROR, code::GENERAL_FAILURE, code::USAGE_ERROR};
use serde_json::Value;

use crate::{cli::TaskCommand, daemon};

/// The mark `docs/cli.md` puts beside a task waiting to be assigned.
///
/// Written as an escape because a source file here holds ASCII only.
const WAITING: &str = "\u{25cb}";

pub fn run(command: TaskCommand) -> ExitCode {
    match command {
        TaskCommand::Add {
            title,
            instruction,
            branch,
            after,
            model,
            force,
        } => add(&title, &instruction, branch, after, model, force),
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
    force: bool,
) -> ExitCode {
    // The core runs as a daemon, so where it was started is not where this command was.
    // It cannot learn that from anywhere but here.
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

    send_noting(
        "task_add",
        serde_json::json!({
            "cwd": cwd.display().to_string(),
            "title": title,
            "instruction": instruction,
            "branch": branch,
            "after": after,
            "model": model,
            "force": force,
        }),
        added,
        way_past,
    )
}

/// What to say after a refusal `task add` gets, beyond what the refusal says itself.
///
/// The exit codes in section 2.1 of `docs/cli.md` give `task add` one refusal at code 1: the
/// instruction carries too little to run unattended and `--force` was not given. It names what it
/// did not find, which leaves the author who meant the instruction as they wrote it with nowhere
/// to go. The way past is `--force`, spelled on this surface and nowhere else, so the line is
/// added here rather than by the core.
fn way_past(code: u8) -> Option<&'static str> {
    (code == GENERAL_FAILURE).then_some("--force registers it as written")
}

/// `docs/cli.md` gives `-` the meaning of standard input.
/// An instruction longer than a line does not have to fit in an argument.
fn read_instruction(given: &str) -> std::io::Result<String> {
    if given != "-" {
        return Ok(given.to_owned());
    }
    let mut read = String::new();
    std::io::stdin().read_to_string(&mut read)?;
    Ok(read)
}

/// How long to leave the core alone between asks while following.
///
/// The core answers one connection at a time and a task that is working has nothing new to say most of the time.
/// Asking oftener would take the core away from the run being followed.
const BETWEEN_ASKS: std::time::Duration = std::time::Duration::from_secs(2);

pub fn trace(task: &str, follow: bool, since: Option<String>) -> ExitCode {
    let mut since = since.unwrap_or_default();
    loop {
        let asked = daemon::ask("trace", serde_json::json!({ "task": task, "since": since }));
        let answer = match asked {
            Ok(Response::Data(answer)) => answer.data,
            Ok(Response::Error(failure)) => {
                eprintln!("cistern: {}", failure.message);
                return ExitCode::from(failure.code);
            }
            Err(e) => {
                eprintln!("cistern: the core is not running: {e}");
                return ExitCode::from(CORE_ERROR);
            }
        };

        happened(&answer);
        let done = answer.get("done").and_then(Value::as_bool).unwrap_or(true);
        let carried_on = match text(&answer, "cursor") {
            Some(cursor) if cursor != since => {
                since = cursor.to_owned();
                true
            }
            _ => false,
        };

        // The core answers with as much as it holds at once, so each piece is printed as it comes.
        // A long run costs no more to read.
        if carried_on {
            continue;
        }
        if !follow || done {
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(BETWEEN_ASKS);
    }
}

/// How far this machine's clock stands from the count, in seconds.
///
/// Asked of `date`, which is the one place here that knows what time zone this machine keeps, and asked once.
fn offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let Ok(asked) = std::process::Command::new("date").arg("+%z").output() else {
            return 0;
        };
        let said = String::from_utf8_lossy(&asked.stdout);
        let said = said.trim();
        if said.len() != 5 {
            return 0;
        }
        let (sign, digits) = said.split_at(1);
        let hours: i64 = digits[..2].parse().unwrap_or(0);
        let minutes: i64 = digits[2..].parse().unwrap_or(0);
        let away = hours * 3_600 + minutes * 60;
        if sign == "-" { -away } else { away }
    })
}

/// One event per line, the time it happened and what happened.
fn happened(data: &Value) {
    let Some(events) = data.get("events").and_then(Value::as_array) else {
        return;
    };
    for one in events {
        let at = one.get("at").and_then(Value::as_str).unwrap_or("");
        let said = one.get("said").and_then(Value::as_str).unwrap_or("");
        println!("[{}] {said}", clock_of(at));
    }
}

/// A moment as the clock on this machine reads it.
///
/// Section 2.3 prints the time of day rather than a count of seconds.
/// The count is the same everywhere and the time of day is not, and whoever is reading is looking at their own clock.
fn clock_of(at: &str) -> String {
    let Ok(at) = at.parse::<i64>() else {
        return "--:--:--".to_owned();
    };
    let day = (at + offset()).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        day / 3_600,
        (day % 3_600) / 60,
        day % 60
    )
}

/// Asks the core and prints what came back.
fn send(command: &str, params: Value, print: fn(&Value)) -> ExitCode {
    send_noting(command, params, print, |_| None)
}

/// The same, with a second line for a refusal this command has more to say about.
fn send_noting(
    command: &str,
    params: Value,
    print: fn(&Value),
    note: fn(u8) -> Option<&'static str>,
) -> ExitCode {
    match daemon::ask(command, params) {
        Ok(Response::Data(answer)) => {
            print(&answer.data);
            ExitCode::SUCCESS
        }
        Ok(Response::Error(failure)) => {
            eprintln!("cistern: {}", failure.message);
            if let Some(note) = note(failure.code) {
                eprintln!("  {note}");
            }
            ExitCode::from(failure.code)
        }
        Err(e) => {
            eprintln!("cistern: the core is not running: {e}");
            ExitCode::from(CORE_ERROR)
        }
    }
}

/// The layout section 2.1 shows, which is narrower than the one `task show` uses.
/// It has no label as long as `disposition`.
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
    line(DETAILED, "base", based(data).as_deref());
    line(DETAILED, "after", text(data, "after"));
    line(
        DETAILED,
        "repo",
        home_relative(text(data, "repository")).as_deref(),
    );
    line(DETAILED, "branch", text(data, "branch"));
    line(DETAILED, "reason", text(data, "reason"));
    line(DETAILED, "worktree", text(data, "worktree"));
    line(DETAILED, "carries on", text(data, "conversation"));
    made(data);
    line(DETAILED, "disposition", text(data, "disposition"));
    if let Some(instruction) = text(data, "instruction") {
        print!("{}", block("instruction", instruction));
    }
    if let Some(original) = text(data, "original") {
        print!("{}", block("original", original));
    }
}

/// A field that will not sit in a column, printed under its label instead.
///
/// An instruction read from standard input runs to paragraphs, and one filled in from the
/// repository is longer than what the author wrote, so neither keeps the width the fields above
/// align to. Its lines are indented under the label rather than wrapped, so what is printed is
/// what was given.
fn block(label: &str, value: &str) -> String {
    let mut written = format!("  {label}:\n");
    for line in value.lines() {
        written.push_str("    ");
        written.push_str(line);
        written.push('\n');
    }
    written
}

/// The base branch, and how far it has moved since the task left it.
fn based(data: &Value) -> Option<String> {
    let base = text(data, "base_branch")?.to_owned();
    match data.get("base_ahead").and_then(Value::as_u64) {
        Some(0) | None => Some(base),
        Some(1) => Some(format!("{base} (1 commit ahead)")),
        Some(ahead) => Some(format!("{base} ({ahead} commits ahead)")),
    }
}

/// The commits on the result branch, one per line, as section 2.1 shows them.
fn made(data: &Value) {
    let Some(made) = data.get("commits").and_then(Value::as_array) else {
        return;
    };
    if made.is_empty() {
        return;
    }
    let width = made
        .iter()
        .filter_map(|one| text(one, "subject"))
        .map(str::len)
        .max()
        .unwrap_or(0);

    println!("  commits:");
    for one in made {
        let (Some(sha), Some(subject)) = (text(one, "sha"), text(one, "subject")) else {
            continue;
        };
        println!(
            "    {sha}  {subject:<width$}  {} {}",
            counted('+', one.get("added")),
            counted('-', one.get("removed"))
        );
    }
}

/// A count of lines a commit gained or lost, or a dash where git counted none.
fn counted(sign: char, held: Option<&Value>) -> String {
    match held.and_then(Value::as_u64) {
        Some(count) => format!("{sign}{count}"),
        None => "-".to_owned(),
    }
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
/// `docs/cli.md` shows a value that is absent in parentheses, so a field that holds nothing is still printed.
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
/// The home directory is an argument rather than a read.
/// Both outcomes can be checked without setting a variable the whole process sees.
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
    use cistern_contract::code::{NOT_FOUND, STATE_CONFLICT};

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

    /// An instruction is printed as it was given, however many lines that is.
    #[test]
    fn a_field_that_will_not_sit_in_a_column_is_printed_under_its_label() {
        assert_eq!(
            block("instruction", "fix parse() in src/util.rs"),
            "  instruction:\n    fix parse() in src/util.rs\n"
        );
        assert_eq!(
            block("original", "fix the parser\nit panics on an empty line"),
            "  original:\n    fix the parser\n    it panics on an empty line\n"
        );
    }

    #[test]
    fn no_home_leaves_every_path_alone() {
        assert_eq!(under_home("/home/a/work/api", None), "/home/a/work/api");
    }

    /// Section 2.1 of `docs/cli.md` gives `task add` one refusal at code 1, and it is the
    /// turned-back instruction.
    #[test]
    fn a_turned_back_instruction_is_told_how_to_be_registered() {
        assert_eq!(
            way_past(GENERAL_FAILURE),
            Some("--force registers it as written")
        );
    }

    /// Every other refusal has nothing this surface can add.
    #[test]
    fn another_refusal_is_left_as_the_core_put_it() {
        for code in [USAGE_ERROR, NOT_FOUND, STATE_CONFLICT, CORE_ERROR] {
            assert_eq!(way_past(code), None, "code {code}");
        }
    }
}
