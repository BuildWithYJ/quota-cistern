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
