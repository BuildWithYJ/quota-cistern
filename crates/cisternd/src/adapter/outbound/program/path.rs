//! Reading one figure out of an answer, by a path a definition holds.
//!
//! A path is names joined by dots. One name may be `*`, which stands for every key of the
//! object at that point; the numbers found under it are added together.
//!
//! That star is the only rule beyond following names, and it is here because a vendor may
//! report per model what the core counts once. Without it a definition could not say
//! "every model's input tokens" and the summing would go back into the code, which is what
//! this module exists to take out of it.

use serde_json::Value;

/// What a path leads to, read as text.
///
/// Nothing when the path leads nowhere or to something that is not text.
pub fn text(answer: &Value, path: &str) -> Option<String> {
    follow(answer, &segments(path))?.as_str().map(str::to_owned)
}

/// What a path leads to, added up.
///
/// A path with no star leads to one number. A path with a star leads to as many as the
/// object has keys, and this answers with their total.
///
/// Nothing when the path leads nowhere, or to something that is not a number, or to an
/// object with no keys. An object with no keys is not a total of zero: a vendor that
/// reported nothing under a name it usually fills has not said the run cost nothing.
pub fn total(answer: &Value, path: &str) -> Option<f64> {
    let found = gather(answer, &segments(path));
    match found.is_empty() {
        true => None,
        false => found.into_iter().sum::<Option<f64>>(),
    }
}

fn segments(path: &str) -> Vec<&str> {
    path.split('.').filter(|name| !name.is_empty()).collect()
}

/// The value at the end of a path with no star in it.
fn follow<'a>(at: &'a Value, left: &[&str]) -> Option<&'a Value> {
    match left.split_first() {
        None => Some(at),
        Some((name, rest)) => follow(at.get(name)?, rest),
    }
}

/// Every value the path leads to, as a number.
///
/// A name that is not there and a value that is not a number both come back as `None`, and
/// one `None` leaves the whole total unknown. That is what tells a renamed field from a
/// field holding zero: one model reporting under a name this does not know would otherwise
/// drop out of the sum and the run would read as cheaper than it was.
fn gather(at: &Value, left: &[&str]) -> Vec<Option<f64>> {
    let Some((name, rest)) = left.split_first() else {
        return vec![at.as_f64()];
    };
    if *name != "*" {
        return match at.get(name) {
            Some(next) => gather(next, rest),
            None => vec![None],
        };
    }
    match at.as_object() {
        Some(held) => held.values().flat_map(|next| gather(next, rest)).collect(),
        None => vec![None],
    }
}

#[cfg(test)]
mod tests;
