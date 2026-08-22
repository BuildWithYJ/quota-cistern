use crate::core::domain::{Budget, Span, Usage};

use super::*;

fn declaring(usage: Usage) -> Budget {
    Budget {
        usage,
        time: Span::parse("8h").unwrap(),
    }
}

#[test]
fn a_share_is_shown_as_the_percentage_it_was_declared_in() {
    assert_eq!(Spending::Share(400).to_string(), "4%");
    assert_eq!(Spending::Share(350).to_string(), "3.5%");
    assert_eq!(Spending::Share(405).to_string(), "4.05%");
    assert_eq!(Spending::Share(0).to_string(), "0%");
}

#[test]
fn a_count_is_shown_as_the_count() {
    assert_eq!(Spending::Tokens(2_000_000).to_string(), "2000000");
}

#[test]
fn what_is_left_is_what_was_declared_less_what_was_spent() {
    let budget = declaring(Usage::Tokens(1_000));
    assert_eq!(budget.left(Spending::Tokens(400)), 600);
}

/// A share is declared in whole percent and measured in hundredths.
#[test]
fn a_share_is_left_over_in_hundredths_of_a_percent() {
    let budget = declaring(Usage::Share(50));
    assert_eq!(budget.left(Spending::Share(2_000)), 3_000);
}

/// A session can pass its budget between two decisions.
/// A decision is made when a task ends and not while one runs.
#[test]
fn spending_more_than_was_declared_leaves_nothing() {
    let budget = declaring(Usage::Tokens(1_000));
    assert_eq!(budget.left(Spending::Tokens(4_000)), 0);
}
