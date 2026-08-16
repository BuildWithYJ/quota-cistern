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
mod tests {
    use serde_json::json;

    use super::*;

    fn an_answer() -> Value {
        json!({
            "subtype": "success",
            "total_cost_usd": 0.0921703,
            "modelUsage": {
                "claude-haiku-4-5": { "inputTokens": 77, "outputTokens": 3377 },
                "claude-opus-5":    { "inputTokens": 1,  "outputTokens": 2 }
            },
            "usage": { "input_tokens": 74 }
        })
    }

    #[test]
    fn a_path_of_names_reads_what_it_names() {
        assert_eq!(text(&an_answer(), "subtype").as_deref(), Some("success"));
        assert_eq!(total(&an_answer(), "usage.input_tokens"), Some(74.0));
    }

    /// A vendor may split a figure by model while the core counts it once.
    #[test]
    fn a_star_adds_up_every_key_it_stands_for() {
        assert_eq!(total(&an_answer(), "modelUsage.*.inputTokens"), Some(78.0));
        assert_eq!(
            total(&an_answer(), "modelUsage.*.outputTokens"),
            Some(3379.0)
        );
    }

    #[test]
    fn a_path_that_leads_nowhere_is_not_a_total_of_zero() {
        assert_eq!(total(&an_answer(), "modelUsage.*.tokensIn"), None);
        assert_eq!(total(&an_answer(), "nothing.here"), None);
        assert_eq!(text(&an_answer(), "nothing"), None);
    }

    /// One model reporting under a name this does not know would otherwise drop out of the
    /// sum, and the run would read as cheaper than it was.
    #[test]
    fn one_key_that_lost_the_name_leaves_no_total() {
        let renamed = json!({
            "modelUsage": {
                "a": { "inputTokens": 10 },
                "b": { "tokensIn": 20 }
            }
        });
        assert_eq!(total(&renamed, "modelUsage.*.inputTokens"), None);
    }

    /// A figure that stopped being a number is not a figure of zero.
    #[test]
    fn a_value_that_is_not_a_number_leaves_no_total() {
        let odd = json!({ "modelUsage": { "a": { "inputTokens": "a lot" } } });
        assert_eq!(total(&odd, "modelUsage.*.inputTokens"), None);
    }

    #[test]
    fn a_star_over_nothing_is_not_a_total_of_zero() {
        let empty = json!({ "modelUsage": {} });
        assert_eq!(total(&empty, "modelUsage.*.inputTokens"), None);
    }
}
