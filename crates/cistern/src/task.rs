//! The `task` commands and `backlog`.
//!
//! What was typed goes to the core as it was given.
//! Whether a value is allowed and whether a task exists are the core's to decide, so this file has no list of either.

use std::{
    env,
    ffi::OsString,
    io::{self, IsTerminal, Read, Write},
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

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
    let mut instruction = match read_instruction(instruction) {
        Ok(instruction) => instruction,
        Err(e) => {
            eprintln!("cistern: the instruction cannot be read: {e}");
            return ExitCode::from(USAGE_ERROR);
        }
    };
    let cwd = cwd.display().to_string();
    // What the author typed, kept beside the instruction once one of the fills replaces it. The
    // two asks are separate requests, so the core cannot remember this and is told it instead.
    let wrote = instruction.clone();
    let mut asked_already = false;

    // The core fills a loose instruction in from the repository, and where the repository allows
    // more than one fill it answers with them rather than picking. Asking is this side's to do:
    // the core is a daemon and the person is here. What they choose goes back as the instruction,
    // so the second ask is an ordinary one and the gate reads it like any other.
    loop {
        // Working out what a line meant means reading the repository and asking a model, which
        // takes as long as it takes. A command that says nothing for a minute is one a person
        // stops, so it says what it is doing and how long it has been at it.
        let waiting = (!asked_already).then(Waiting::shown);
        let asked = asked(
            "task_add",
            serde_json::json!({
                "cwd": cwd,
                "title": title,
                "instruction": instruction,
                "original": asked_already.then_some(wrote.as_str()),
                "branch": branch,
                "after": after,
                "model": model,
                "force": force,
            }),
            // The core registers a task or asks about one. Nothing it refuses task_add with is
            // a code this surface has more to say about than the core did.
            |_| None,
        );
        drop(waiting);
        let answer = match asked {
            Ok(answer) => answer,
            Err(code) => return code,
        };

        if text(&answer, "outcome") != Some(UNCONFIRMED) {
            added(&answer);
            return ExitCode::SUCCESS;
        }
        let Some(chosen) = chosen(&answer) else {
            return ExitCode::from(GENERAL_FAILURE);
        };
        instruction = chosen;
        asked_already = true;
    }
}

/// How long the core is given to answer before anything is said about waiting.
///
/// An instruction the author wrote out in full comes back at once, and a line that flashed up
/// and away would be read as something having gone wrong.
const BEFORE_SAYING_SO: Duration = Duration::from_millis(700);

/// A line saying the core is working, taken away when it answers.
///
/// Written over itself as the seconds pass rather than added to, so what is left on the screen
/// when the answer comes is what the answer wrote and nothing before it.
struct Waiting {
    going: Arc<AtomicBool>,
    saying: Option<thread::JoinHandle<()>>,
}

impl Waiting {
    fn shown() -> Self {
        let going = Arc::new(AtomicBool::new(true));
        let mine = Arc::clone(&going);
        let saying = thread::spawn(move || {
            let since = Instant::now();
            thread::sleep(BEFORE_SAYING_SO);
            while mine.load(Ordering::Relaxed) {
                eprint!(
                    "\r  working out what that means... {}s",
                    since.elapsed().as_secs()
                );
                io::stderr().flush().ok();
                thread::sleep(Duration::from_millis(500));
            }
        });
        Waiting {
            going,
            saying: Some(saying),
        }
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        self.going.store(false, Ordering::Relaxed);
        if let Some(saying) = self.saying.take() {
            saying.join().ok();
        }
        // Written over with blanks rather than left for the answer to print under.
        eprint!("\r{:60}\r", "");
        io::stderr().flush().ok();
    }
}

/// What `outcome` says when the core filled the instruction in and did not register anything.
const UNCONFIRMED: &str = "unconfirmed";

/// Shows the spec the core worked out, settles what is left, and answers with what to send back.
///
/// Nothing was registered, so there is nothing to undo and no state to hold between the two asks:
/// what goes back is the spec as text, and the core reads it as the author's own because they
/// have now seen it.
///
/// Nothing when there is nobody to ask, or when they said no. The spec is printed either way, so
/// a command in a script says what it would have asked about.
fn chosen(answer: &Value) -> Option<String> {
    let mut parts = parts(answer);
    let left: Vec<&Value> = answer.get("undecided")?.as_array()?.iter().collect();

    shown(&parts, &left);

    if !io::stdin().is_terminal() {
        eprintln!(
            "  nobody is here to settle {}. --force registers the instruction as written",
            match left.len() {
                1 => "it".to_owned(),
                many => format!("{many} of them"),
            }
        );
        return None;
    }

    // What nobody settled, one at a time. A part an agent would otherwise settle for itself is
    // the whole reason for asking, so there is no accepting past it.
    for one in &left {
        let named = text(one, "part")?;
        let said = settling(named, text(one, "decides").unwrap_or("it"))?;
        put(&mut parts, named, &said);
    }

    loop {
        eprint!("  register as it stands? [enter=yes / a number=change that line / n=no] ");
        io::stderr().flush().ok()?;
        let mut typed = String::new();
        if io::stdin().read_line(&mut typed).ok()? == 0 {
            eprintln!();
            return None;
        }
        let typed = typed.trim();
        if typed.is_empty() {
            return Some(written(&parts));
        }
        if typed.eq_ignore_ascii_case("n") {
            return None;
        }
        match typed.parse::<usize>() {
            Ok(at) if (1..=parts.len()).contains(&at) => {
                let named = parts[at - 1].named.clone();
                let said = typing(&format!("  {named}: "))?;
                put(&mut parts, &named, &said);
                shown(&parts, &[]);
            }
            _ => eprintln!("  there is no {typed} on the list"),
        }
    }
}

/// One part of a spec, as the core sent it.
struct Part {
    named: String,
    said: String,
    /// Who settled it: the author, the repository, or nobody yet.
    settled: String,
    /// What it was drawn from, for a reader deciding whether to take it.
    drawn_from: Option<String>,
    /// The others the repository allows, for a reader who does not take this one.
    others: Vec<String>,
}

impl Part {
    /// Writes what somebody settled into it, which makes it theirs.
    fn settle(&mut self, said: &str) {
        self.said = said.to_owned();
        self.settled = "given".to_owned();
        self.drawn_from = None;
        self.others.clear();
    }
}

/// The spec as the core sent it, part by part, in the order it was read in.
fn parts(answer: &Value) -> Vec<Part> {
    answer
        .get("parts")
        .and_then(Value::as_array)
        .map(|held| {
            held.iter()
                .map(|part| Part {
                    named: text(part, "part").unwrap_or_default().to_owned(),
                    said: text(part, "said").unwrap_or_default().to_owned(),
                    settled: text(part, "settled").unwrap_or_default().to_owned(),
                    drawn_from: text(part, "drawn_from").map(str::to_owned),
                    others: part
                        .get("others")
                        .and_then(Value::as_array)
                        .map(|others| {
                            others
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// How wide the column holding a part's name is, and how far a part's text is indented past it.
const NAMED: usize = 11;
const INDENT: usize = 6 + NAMED;

/// How wide the screen is taken to be where nothing says.
///
/// A model writes a paragraph where it has a paragraph's worth to say, and a paragraph printed
/// as one line is not read. `COLUMNS` is what a shell exports for this; where nothing exported
/// it, this is a width every terminal has.
const AS_WIDE_AS: usize = 80;

/// How wide the screen is.
fn across() -> usize {
    env::var("COLUMNS")
        .ok()
        .and_then(|held| held.parse().ok())
        .filter(|across| *across > INDENT + 20)
        .unwrap_or(AS_WIDE_AS)
}

/// The text broken to fit, its own line breaks kept.
///
/// Broken between words. A word longer than the width left is put on its own line rather than cut,
/// since a path cut in half is a path nobody can read back.
fn broken(text: &str, across: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for given in text.lines() {
        let mut held = String::new();
        for word in given.split_whitespace() {
            if !held.is_empty() && held.chars().count() + 1 + word.chars().count() > across {
                lines.push(std::mem::take(&mut held));
            }
            if !held.is_empty() {
                held.push(' ');
            }
            held.push_str(word);
        }
        lines.push(held);
    }
    lines
}

/// One part's text, under its label and indented to it.
fn under(first: &str, text: &str, across: usize) -> String {
    let mut written = String::new();
    for (at, line) in broken(text, across.saturating_sub(INDENT))
        .iter()
        .enumerate()
    {
        match at {
            0 => written.push_str(&format!("{first}{line}\n")),
            _ => written.push_str(&format!("{:INDENT$}{line}\n", "")),
        }
    }
    written
}

/// The whole spec on one screen, and what it still leaves.
///
/// Every part carries what it was drawn from, because a path on its own tells a reader nothing.
/// What tells them whether to take it is that it is the file they have open with the count
/// doubled on two lines.
fn shown(parts: &[Part], left: &[&Value]) {
    eprintln!();
    let across = across();
    for (at, part) in parts.iter().enumerate() {
        let said = match part.said.is_empty() {
            true => "-----",
            false => &part.said,
        };
        // Only what nobody settled is marked. That a part was worked out rather than typed is
        // what the line under it says, and saying it twice is a word in the way of the reading.
        let marked = match part.settled.as_str() {
            "open" => "   [open]",
            _ => "",
        };
        eprint!(
            "{}",
            under(
                &format!("  {:>2}  {:<NAMED$}", at + 1, part.named),
                &format!("{said}{marked}"),
                across,
            )
        );
        if let Some(drawn_from) = &part.drawn_from {
            eprint!(
                "{}",
                under(
                    &format!("      {:<NAMED$}", ""),
                    &format!("- {drawn_from}"),
                    across
                )
            );
        }
        if !part.others.is_empty() {
            eprint!(
                "{}",
                under(
                    &format!("      {:<NAMED$}", ""),
                    &format!("- also: {}", part.others.join(", ")),
                    across,
                )
            );
        }
        // A part with more than a line to it is given room, so the six do not run together.
        if part.said.chars().count() + INDENT > across || part.drawn_from.is_some() {
            eprintln!();
        }
    }
    eprintln!();
    if left.is_empty() {
        return;
    }
    eprintln!(
        "  {} left for the agent to decide by itself:",
        match left.len() {
            1 => "1 decision".to_owned(),
            many => format!("{many} decisions"),
        }
    );
    for one in left {
        eprintln!("    - {}", text(one, "decides").unwrap_or("something"));
    }
    eprintln!();
}

/// What a run does when it cannot get there, which no model may answer on an author's behalf.
///
/// The common unattended accident is an agent that could not pass a test and edited the test, so
/// this is the one decision a person makes. The recommended answer is first, and costs one key.
const WHEN_IT_FAILS: [&str; 2] = [
    "stop after three attempts and leave the branch as it is. do not edit the tests",
    "put back what was changed and say why it stopped",
];

/// Settles one part nobody settled, by offering answers or by taking one typed out.
fn settling(named: &str, decides: &str) -> Option<String> {
    eprintln!("  nothing says {decides}.");
    let offered: &[&str] = match named {
        "on failure" => &WHEN_IT_FAILS,
        _ => &[],
    };
    for (at, one) in offered.iter().enumerate() {
        let recommended = match at {
            0 => "   (recommended)",
            _ => "",
        };
        eprintln!("    {}) {one}{recommended}", at + 1);
    }
    if !offered.is_empty() {
        eprintln!("    {}) type your own", offered.len() + 1);
    }

    loop {
        let typed = typing(&format!("  {named}: "))?;
        match typed.parse::<usize>() {
            Ok(at) if (1..=offered.len()).contains(&at) => return Some(offered[at - 1].to_owned()),
            // The number past the offered ones is the one that asks for an answer of your own.
            Ok(at) if at == offered.len() + 1 && !offered.is_empty() => {
                return typing(&format!("  {named}: "));
            }
            Ok(at) => eprintln!("  there is no {at} on the list"),
            Err(_) => return Some(typed),
        }
    }
}

/// One line typed at a prompt, or nothing where there is no more input or it was left blank.
fn typing(prompt: &str) -> Option<String> {
    eprint!("{prompt}");
    // Printed without a newline, so it has to be pushed out before the read blocks on it.
    io::stderr().flush().ok()?;

    let mut typed = String::new();
    if io::stdin().read_line(&mut typed).ok()? == 0 {
        eprintln!();
        return None;
    }
    let typed = typed.trim();
    (!typed.is_empty()).then(|| typed.to_owned())
}

/// Writes what was settled into the part it settles.
fn put(parts: &mut [Part], named: &str, said: &str) {
    if let Some(part) = parts.iter_mut().find(|part| part.named == named) {
        part.settle(said);
    }
}

/// The spec as one text, which is what goes back as the instruction.
///
/// The same lines the core wrote it in, so that what is sent is what was shown.
fn written(parts: &[Part]) -> String {
    parts
        .iter()
        .filter(|part| !part.said.trim().is_empty())
        .map(|part| format!("{}: {}", part.named, part.said))
        .collect::<Vec<_>>()
        .join("\n")
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
    match asked(command, params, note) {
        Ok(answer) => {
            print(&answer);
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Asks the core and hands back what it answered, or the code to end on.
///
/// Apart from printing it, because `task add` may ask twice: what comes back the first time can
/// be a question rather than a task, and the second ask is made with the answer to it.
fn asked(
    command: &str,
    params: Value,
    note: fn(u8) -> Option<&'static str>,
) -> Result<Value, ExitCode> {
    match daemon::ask(command, params) {
        Ok(Response::Data(answer)) => Ok(answer.data),
        Ok(Response::Error(failure)) => {
            eprintln!("cistern: {}", failure.message);
            if let Some(note) = note(failure.code) {
                eprintln!("  {note}");
            }
            Err(ExitCode::from(failure.code))
        }
        Err(e) => {
            eprintln!("cistern: the core is not running: {e}");
            Err(ExitCode::from(CORE_ERROR))
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

    /// A model writes a paragraph where it has one, and a paragraph printed as one line is not
    /// read.
    #[test]
    fn a_part_too_wide_for_the_screen_is_broken_between_words() {
        assert_eq!(
            broken("the counter is incremented at both 41 and 43", 20),
            vec!["the counter is", "incremented at both", "41 and 43"]
        );
        // Its own line breaks are kept, since a model that broke a line meant to.
        assert_eq!(broken("one\ntwo", 20), vec!["one", "two"]);
    }

    /// A path cut in half is a path nobody can read back.
    #[test]
    fn a_word_longer_than_the_room_left_is_put_on_its_own_line() {
        assert_eq!(
            broken("in crates/cisternd/src/core/domain/decisions.rs now", 12),
            vec!["in", "crates/cisternd/src/core/domain/decisions.rs", "now"]
        );
    }

    /// What runs on is indented to where it started, so the column still reads as a column.
    #[test]
    fn what_runs_past_the_first_line_is_indented_under_it() {
        assert_eq!(
            under("  1  place      ", "one two three", INDENT + 7),
            "  1  place      one two\n                 three\n"
        );
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
}
