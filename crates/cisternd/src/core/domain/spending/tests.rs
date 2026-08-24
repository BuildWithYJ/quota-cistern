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

/// A count is declared in the unit it is measured in, and a share is declared in whole
/// percent and measured in hundredths of one.
#[test]
fn what_was_declared_reads_in_the_unit_it_is_measured_in() {
    assert_eq!(declaring(Usage::Tokens(1_000)).declared(), 1_000);
    assert_eq!(declaring(Usage::Share(50)).declared(), 5_000);
}

/// A figure read for a session is in the unit that session declared. The other pairing is a
/// figure about something else, and a session is not measured against one.
#[test]
fn a_figure_of_another_unit_is_no_figure_at_all() {
    assert_eq!(
        Spending::Share(2_000).against(Usage::Share(50)),
        Some(2_000)
    );
    assert_eq!(
        Spending::Tokens(400).against(Usage::Tokens(1_000)),
        Some(400)
    );
    assert_eq!(Spending::Tokens(1).against(Usage::Share(50)), None);
    assert_eq!(Spending::Share(1).against(Usage::Tokens(50)), None);
}
