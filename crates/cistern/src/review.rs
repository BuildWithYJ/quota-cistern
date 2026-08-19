//! The `diff` command and the three that dispose of a result.
//!
//! What was typed goes to the core as it was given.
//! Whether a task exists and whether its result may be applied are the core's to decide, so this file judges neither.

use std::process::ExitCode;

use cistern_contract::{
    Response,
    code::{CORE_ERROR, GENERAL_FAILURE},
};
use serde_json::Value;

use crate::{cli::ReviewCommand, daemon};

/// The marks section 2.2 puts before a task, one per terminal state.
const FINISHED: &str = "\u{2713}";
const CUT_SHORT: &str = "\u{26a0}";
const FAILED: &str = "\u{2715}";
const STILL_GOING: &str = "\u{25cb}";

/// What section 2.2 puts before a task's result branch.
const TOWARDS: &str = "\u{2192}";

/// The separator section 2.4 puts between a count and what follows it.
const BETWEEN: &str = "\u{b7}";

pub fn diff(task: &str, stat: bool) -> ExitCode {
    let answer = match ask("diff", serde_json::json!({ "task": task })) {
        Ok(answer) => answer,
        Err(code) => return code,
    };

    let files = array(&answer, "files");
    let patch = answer.get("patch").and_then(Value::as_str).unwrap_or("");

    // Section 2.3 makes an empty diff a failure and says what to print.
    // The core was not asked a question it could refuse, so both are here.
    if files.is_empty() && patch.trim().is_empty() {
        println!("(no changes)");
        return ExitCode::from(GENERAL_FAILURE);
    }

    match stat {
        true => summarised(&files),
        false => print!("{patch}"),
    }
    ExitCode::SUCCESS
}

pub fn run(command: ReviewCommand) -> ExitCode {
    match command {
        ReviewCommand::Ls => send("review_ls", serde_json::json!({}), waiting),
    }
}

pub fn apply(task: &str) -> ExitCode {
    send("apply", serde_json::json!({ "task": task }), taken)
}

pub fn discard(task: &str) -> ExitCode {
    send("discard", serde_json::json!({ "task": task }), dropped)
}

pub fn retry(task: &str) -> ExitCode {
    send("retry", serde_json::json!({ "task": task }), requeued)
}

pub fn tidy() -> ExitCode {
    send("tidy", serde_json::json!({}), tidied)
}

/// Asks the core and hands back what it answered, or the code to exit with.
fn ask(command: &str, params: Value) -> Result<Value, ExitCode> {
    match daemon::ask(command, params) {
        Ok(Response::Data(answer)) => Ok(answer.data),
        Ok(Response::Error(failure)) => {
            eprintln!("cistern: {}", failure.message);
            Err(ExitCode::from(failure.code))
        }
        Err(e) => {
            eprintln!("cistern: the core is not running: {e}");
            Err(ExitCode::from(CORE_ERROR))
        }
    }
}

fn send(command: &str, params: Value, print: fn(&Value)) -> ExitCode {
    match ask(command, params) {
        Ok(answer) => {
            print(&answer);
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn waiting(data: &Value) {
    let items = array(data, "review_queue");
    let width = |field: &str| {
        items
            .iter()
            .filter_map(|item| text(item, field))
            .map(str::len)
            .max()
            .unwrap_or(0)
    };
    let (id, title, session, branch, state) = (
        width("id"),
        width("title"),
        width("session"),
        width("branch"),
        width("state"),
    );

    for item in &items {
        let field = |name: &str| text(item, name).unwrap_or("(none)");
        println!(
            "{}  {:<id$}  {:<title$}  {:<session$}  {TOWARDS} {:<branch$}  {:<state$}  {}",
            marked(field("state")),
            field("id"),
            field("title"),
            field("session"),
            field("branch"),
            field("state"),
            standing(item)
        );
    }
}

/// What is on the branch, at the end of the line section 2.4 prints.
///
/// A branch the user deleted leaves no counts.
/// The task is still listed, since it was still registered.
fn standing(item: &Value) -> String {
    let Some(commits) = count(item, "commit_count") else {
        return "(branch not there)".to_owned();
    };
    let mut said = commits_as(commits);
    if let Some(ahead) = count(item, "base_ahead").filter(|ahead| *ahead > 0) {
        said.push_str(&format!(" {BETWEEN} base +{ahead}"));
    }
    said
}

fn taken(data: &Value) {
    let Some(task) = text(data, "task") else {
        return;
    };
    println!("{task} applied to working tree");

    let files = array(data, "files");
    let width = files
        .iter()
        .filter_map(|file| text(file, "path"))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let figures = |field: &str| {
        files
            .iter()
            .filter_map(|file| count(file, field))
            .map(|it| it.to_string().len())
            .max()
            .unwrap_or(1)
    };
    let (added, removed) = (figures("added"), figures("removed"));

    for file in &files {
        let Some(path) = text(file, "path") else {
            continue;
        };
        println!(
            "  {path:<width$}   {:>added$} {:>removed$}",
            plus(count(file, "added")),
            minus(count(file, "removed"))
        );
    }
    println!("  (nothing committed {BETWEEN} review and commit in your own environment)");
}

fn dropped(data: &Value) {
    let Some(task) = text(data, "task") else {
        return;
    };
    println!("{task} discarded");
    if let Some(branch) = text(data, "branch") {
        println!("  branch {branch} is kept");
    }
}

fn tidied(data: &Value) {
    let Some(items) = data.get("items").and_then(Value::as_array) else {
        return;
    };
    let gone: Vec<&Value> = items
        .iter()
        .filter(|one| one.get("kept").is_none_or(Value::is_null))
        .collect();
    if gone.is_empty() && items.is_empty() {
        println!("nothing to tidy up");
        return;
    }
    if !gone.is_empty() {
        let named: Vec<&str> = gone.iter().filter_map(|one| text(one, "task")).collect();
        println!("tidied up  {}", named.join("  "));
    }
    for one in items.iter().filter(|one| !gone.contains(one)) {
        let (Some(task), Some(kept)) = (text(one, "task"), text(one, "kept")) else {
            continue;
        };
        println!("left       {task}  {kept}");
    }
}

fn requeued(data: &Value) {
    let Some(task) = text(data, "task") else {
        return;
    };
    println!("{task} is waiting again");
    if let Some(branch) = text(data, "branch") {
        println!("  branch {branch} is kept, and the next run starts from it");
    }
    if let Some(attempts) = text(data, "attempts") {
        println!("  assigned {attempts} time(s) so far");
    }
}

/// How wide the bar beside a file may grow.
///
/// The longest file fills it and the rest are drawn to scale.
/// A file that changed twice as much is twice as long, whatever the numbers are.
const BAR: u64 = 40;

/// The per-file summary section 2.3 prints for `--stat`.
fn summarised(files: &[&Value]) {
    let width = files
        .iter()
        .filter_map(|file| text(file, "path"))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let touched: Vec<u64> = files.iter().map(|file| lines(file).unwrap_or(0)).collect();
    let most = touched.iter().copied().max().unwrap_or(0);
    let figures = touched
        .iter()
        .map(|it| it.to_string().len())
        .max()
        .unwrap_or(1);

    for file in files {
        let Some(path) = text(file, "path") else {
            continue;
        };
        match (count(file, "added"), count(file, "removed")) {
            // git counts no lines in a binary file, and neither does this.
            (None, None) => println!(" {path:<width$} | Bin"),
            (added, removed) => {
                let (added, removed) = (added.unwrap_or(0), removed.unwrap_or(0));
                println!(
                    " {path:<width$} | {:>figures$} {}",
                    added + removed,
                    bar(added, removed, most)
                );
            }
        }
    }

    let (added, removed) = files.iter().fold((0, 0), |(added, removed), file| {
        (
            added + count(file, "added").unwrap_or(0),
            removed + count(file, "removed").unwrap_or(0),
        )
    });
    println!("{}", changed_as(files.len(), added, removed));
}

/// The bar git draws beside a file, scaled so the widest one fits.
fn bar(added: u64, removed: u64, most: u64) -> String {
    let scaled = |count: u64| match most > BAR {
        // A file that changed anything keeps at least one mark.
        // A small change beside a large one does not disappear.
        true => match count {
            0 => 0,
            count => (count * BAR / most).max(1),
        },
        false => count,
    };
    let mut drawn = "+".repeat(scaled(added) as usize);
    drawn.push_str(&"-".repeat(scaled(removed) as usize));
    drawn
}

/// How many lines a file gained and lost together, when git counted any.
fn lines(file: &Value) -> Option<u64> {
    Some(count(file, "added")? + count(file, "removed")?)
}

/// The last line of `--stat`, in the words git writes it in.
fn changed_as(files: usize, added: u64, removed: u64) -> String {
    let mut said = match files {
        1 => "1 file changed".to_owned(),
        other => format!("{other} files changed"),
    };
    if added > 0 {
        said.push_str(&format!(", {added} insertion{}(+)", plural(added)));
    }
    if removed > 0 {
        said.push_str(&format!(", {removed} deletion{}(-)", plural(removed)));
    }
    said
}

fn commits_as(commits: u64) -> String {
    match commits {
        1 => "1 commit".to_owned(),
        other => format!("{other} commits"),
    }
}

fn plural(count: u64) -> &'static str {
    match count {
        1 => "",
        _ => "s",
    }
}

/// A count of lines gained, or the dash git writes for a file it did not count.
fn plus(count: Option<u64>) -> String {
    count.map_or_else(|| "-".to_owned(), |count| format!("+{count}"))
}

fn minus(count: Option<u64>) -> String {
    count.map_or_else(|| "-".to_owned(), |count| format!("-{count}"))
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

fn array<'a>(data: &'a Value, field: &str) -> Vec<&'a Value> {
    data.get(field)
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}

fn count(value: &Value, field: &str) -> Option<u64> {
    value.get(field)?.as_u64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_line_of_a_summary_reads_the_way_git_writes_it() {
        assert_eq!(
            changed_as(2, 34, 18),
            "2 files changed, 34 insertions(+), 18 deletions(-)"
        );
        assert_eq!(
            changed_as(1, 1, 1),
            "1 file changed, 1 insertion(+), 1 deletion(-)"
        );
    }

    /// A file that only gained lines has nothing to say about deletions.
    #[test]
    fn a_summary_leaves_out_what_did_not_happen() {
        assert_eq!(changed_as(1, 12, 0), "1 file changed, 12 insertions(+)");
        assert_eq!(changed_as(1, 0, 4), "1 file changed, 4 deletions(-)");
    }

    #[test]
    fn a_bar_shorter_than_the_width_is_drawn_as_it_stands() {
        assert_eq!(bar(3, 2, 5), "+++--");
    }

    /// The widest file fills the bar and the rest are drawn to scale.
    #[test]
    fn a_bar_wider_than_the_width_is_scaled_down() {
        assert_eq!(bar(80, 0, 80).len(), BAR as usize);
        assert_eq!(bar(40, 40, 80).len(), BAR as usize);
    }

    /// A file that changed anything keeps a mark, however small it is beside the largest one.
    #[test]
    fn the_smallest_change_does_not_disappear() {
        assert_eq!(bar(1, 0, 4_000), "+");
    }
}
