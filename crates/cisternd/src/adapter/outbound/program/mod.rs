//! What answers the vendor ports, by running a program a definition describes.
//!
//! The means is an external program. Which program, what to hand it, and where each figure
//! sits in what it answers are all in the definition, so a second vendor is a file rather
//! than a module. What stays here is the part that is the same whoever the vendor is:
//! starting the child, ending its process group, reading its pipes, and following a path
//! into its answer.

pub mod agent;
pub mod definition;
pub mod drafter;
pub mod limit;
pub mod path;

pub use definition::{Definition, Drafting};

/// The arguments a definition writes, with every place filled and every group that kept a
/// place nobody filled dropped whole.
///
/// A task that named no model has to lose `--model` along with the value, so a group is all
/// or nothing rather than an argument with an empty string after it.
pub(super) fn arguments(args: &[Vec<String>], filling: &[(&str, &str)]) -> Vec<String> {
    let mut given = Vec::with_capacity(args.len() * 2);

    for group in args {
        let filled: Vec<String> = group.iter().map(|token| fill(token, filling)).collect();
        if filled.iter().any(String::is_empty) {
            continue;
        }
        given.extend(filled);
    }
    given
}

/// One argument with `{name}` replaced by what was given for it.
///
/// One pass, so that what is written stands. Filling each name in turn over the whole string
/// would leave a value written for one name open to the names that follow, and a task's
/// instruction goes in among them: an instruction holding the text `{model}` would come out
/// carrying the model.
pub(super) fn fill(token: &str, filling: &[(&str, &str)]) -> String {
    let mut written = String::with_capacity(token.len());
    let mut left = token;

    while let Some(at) = left.find('{') {
        written.push_str(&left[..at]);
        let rest = &left[at..];
        let Some(end) = rest.find('}') else {
            // Nothing closes it, so the rest of the token is not a place and is written as
            // it stands. `left` has to move to it: what came before was written already, and
            // the tail below writes whatever `left` still holds.
            left = rest;
            break;
        };
        let name = &rest[1..end];
        match filling.iter().find(|(known, _)| *known == name) {
            Some((_, value)) => written.push_str(value),
            // A name nothing fills is not a place, so it stays as it was written.
            None => written.push_str(&rest[..=end]),
        }
        left = &rest[end + 1..];
    }

    written.push_str(left);
    written
}
