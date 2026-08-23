use super::*;

fn a_budget() -> Budget {
    Budget {
        usage: Usage::Share(50),
        time: Span(8 * 3_600),
    }
}

fn opening() -> Opening {
    Opening {
        budget: a_budget(),
        model: None,
        started_at: 1_000,
        limit_at_start: Some(1_100),
    }
}

/// A session that has stopped keeps what it consumed when it did.
///
/// A record arriving afterwards reports a figure from a moment the session was no longer
/// running, and it moves `updated_at`, which `elapsed` reads as the moment it stopped.
#[test]
fn a_session_that_stopped_takes_no_more_records() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();

    sessions.record(id, Spending::Share(600), 2_000);
    sessions.stop(id, StoppedReason::AllDone, 3_000);
    sessions.record(id, Spending::Share(900), 4_000);

    let stopped = sessions.sessions().first().unwrap();
    assert_eq!(stopped.consumed(), Spending::Share(600));
    assert_eq!(stopped.updated_at(), 3_000);
}

/// A session only ever spends more, so the lower of two figures was read earlier.
/// The readings are taken outside the hold this is written under, so two of them can
/// arrive in the other order from the one they were read in.
#[test]
fn a_record_that_arrived_late_is_left_out() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();

    sessions.record(id, Spending::Share(900), 2_000);
    sessions.record(id, Spending::Share(600), 3_000);

    assert_eq!(
        sessions.sessions().first().unwrap().consumed(),
        Spending::Share(900)
    );
}

#[test]
fn a_share_and_a_count_are_told_apart_by_the_sign() {
    assert_eq!(Usage::parse("50%"), Some(Usage::Share(50)));
    assert_eq!(Usage::parse("2M"), Some(Usage::Tokens(2_000_000)));
    assert_eq!(Usage::parse("500K"), Some(Usage::Tokens(500_000)));
    assert_eq!(Usage::parse("2000"), Some(Usage::Tokens(2_000)));
}

#[test]
fn a_share_outside_the_range_section_2_2_fixes_is_refused() {
    assert_eq!(Usage::parse("0%"), None);
    assert_eq!(Usage::parse("101%"), None);
}

#[test]
fn a_count_that_is_not_a_whole_number_is_refused() {
    assert_eq!(Usage::parse("1.5M"), None);
    assert_eq!(Usage::parse("many"), None);
    assert_eq!(Usage::parse("0"), None);
}

#[test]
fn a_declaration_reads_back_as_it_was_written() {
    assert_eq!(Usage::Share(50).to_string(), "50%");
    assert_eq!(Usage::Tokens(2_000_000).to_string(), "2000000");
}

#[test]
fn the_two_spellings_section_2_2_shows_are_read() {
    assert_eq!(Span::parse("8h"), Some(Span(8 * 3_600)));
    assert_eq!(Span::parse("2h30m"), Some(Span(2 * 3_600 + 30 * 60)));
}

#[test]
fn a_length_of_time_that_is_not_one_of_the_units_is_refused() {
    assert_eq!(Span::parse("8x"), None);
    assert_eq!(Span::parse("8"), None);
    assert_eq!(Span::parse(""), None);
    assert_eq!(Span::parse("0h"), None);
    // Minutes before hours is not a spelling anyone is shown.
    assert_eq!(Span::parse("30m2h"), None);
}

#[test]
fn a_length_of_time_reads_back_as_the_same_length() {
    for written in ["8h", "2h30m", "45s", "1h1m1s"] {
        let read = Span::parse(written).unwrap();
        assert_eq!(read.to_string(), written);
    }
}

#[test]
fn a_number_is_given_out_once_and_the_next_one_follows_it() {
    let mut sessions = Sessions::restore(1, Vec::new()).unwrap();
    let first = sessions.open(opening()).unwrap();
    assert_eq!(first.labelled(), "session:1");

    sessions.stop(first, StoppedReason::AllDone, 2_000);
    let second = sessions.open(opening()).unwrap();
    assert_eq!(second.labelled(), "session:2");
}

#[test]
fn a_second_session_is_refused_while_one_is_running() {
    let mut sessions = Sessions::default();
    let first = sessions.open(opening()).unwrap();

    assert_eq!(
        sessions.open(opening()),
        Err(NotOpened::AlreadyRunning { id: first })
    );
}

#[test]
fn a_store_holding_two_running_sessions_is_refused() {
    let running = |id| Held {
        id: SessionId(id),
        state: SessionState::Running,
        stopped_reason: None,
        budget: a_budget(),
        model: None,
        started_at: 1_000,
        limit_at_start: Some(1_100),
        limit_last_seen: Some(1_100),
        consumed: Spending::Share(0),
        updated_at: 1_000,
        resets_at: None,
    };

    assert_eq!(
        Sessions::restore(2, vec![running(0), running(1)]),
        Err(NotASessionSet::TwoRunning {
            first: SessionId(0),
            second: SessionId(1),
        })
    );
}

#[test]
fn a_stopped_session_that_does_not_say_why_is_refused() {
    let held = Held {
        id: SessionId(0),
        state: SessionState::Stopped,
        stopped_reason: None,
        budget: a_budget(),
        model: None,
        started_at: 1_000,
        limit_at_start: Some(1_100),
        limit_last_seen: Some(1_100),
        consumed: Spending::Share(0),
        updated_at: 1_000,
        resets_at: None,
    };

    assert_eq!(
        Sessions::restore(1, vec![held]),
        Err(NotASessionSet::ReasonDoesNotMatchState { id: SessionId(0) })
    );
}

#[test]
fn a_state_name_reads_back_as_it_was_written() {
    for state in [SessionState::Running, SessionState::Stopped] {
        assert_eq!(SessionState::parse(&state.to_string()), Some(state));
    }
}

#[test]
fn a_reason_reads_back_as_it_was_written() {
    for reason in [
        StoppedReason::BudgetHardlock,
        StoppedReason::VendorLimit,
        StoppedReason::ObservationUnreadable,
        StoppedReason::Interrupted,
        StoppedReason::AllDone,
        StoppedReason::Blocked,
        StoppedReason::Error,
    ] {
        assert_eq!(StoppedReason::parse(&reason.to_string()), Some(reason));
    }
}

#[test]
fn a_session_has_no_time_left_once_it_has_run_as_long_as_it_declared() {
    let mut sessions = Sessions::default();
    sessions.open(opening()).unwrap();
    let held = sessions.running().unwrap();

    assert_eq!(held.time_left(1_000 + 8 * 3_600 - 1), 1);
    assert_eq!(held.time_left(1_000 + 8 * 3_600), 0);
}

#[test]
fn a_stopped_session_says_why_and_keeps_the_first_answer() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();

    sessions.stop(id, StoppedReason::ObservationUnreadable, 2_000);
    sessions.stop(id, StoppedReason::AllDone, 3_000);

    let stopped = sessions.sessions().first().unwrap();
    assert_eq!(stopped.state(), SessionState::Stopped);
    assert_eq!(
        stopped.stopped_reason(),
        Some(StoppedReason::ObservationUnreadable)
    );
}

#[test]
fn a_share_is_what_the_limit_moved_between_one_look_and_the_next() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();

    assert_eq!(
        sessions.measured(id, 1_400, None, 2_000),
        Some(Spending::Share(300))
    );
    assert_eq!(
        sessions.measured(id, 1_900, None, 3_000),
        Some(Spending::Share(800))
    );
}

/// A reading is taken outside the hold it is applied under, so two threads can arrive in
/// the other order from the one they read in.
///
/// Where the vendor named the window and named the same one, the later arrival carrying the
/// lower figure read first. Counting it would add a whole window to a session that had
/// spent none of it.
#[test]
fn a_reading_that_arrived_late_is_left_out() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();

    // The thread that read 3400 records first: 3400 - 1100 since the session opened.
    assert_eq!(
        sessions.measured(id, 3_400, Some(100), 2_000),
        Some(Spending::Share(2_300))
    );
    // The thread that read 3000 records second, from the window the vendor just named.
    assert_eq!(
        sessions.measured(id, 3_000, Some(100), 3_000),
        Some(Spending::Share(2_300))
    );
    // And the look the next is measured from is still the later one.
    assert_eq!(
        sessions.measured(id, 3_500, Some(100), 4_000),
        Some(Spending::Share(2_400))
    );
}

/// A window the vendor named as a new one is counted whole, however it reads.
/// That is what tells a window turning over from a look that arrived late.
#[test]
fn a_named_window_that_began_again_is_still_counted_whole() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();

    sessions.measured(id, 3_400, Some(100), 2_000);
    assert_eq!(
        sessions.measured(id, 200, Some(200), 3_000),
        Some(Spending::Share(2_500))
    );
}

/// A limit only climbs while one window lasts.
#[test]
fn a_reading_below_the_last_is_a_window_that_has_begun_again() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();
    sessions.measured(id, 1_900, None, 2_000);

    // The whole of the new window was spent since it began.
    assert_eq!(
        sessions.measured(id, 200, None, 3_000),
        Some(Spending::Share(1_000))
    );
    // And the one after it is measured from there, not from where the session opened.
    assert_eq!(
        sessions.measured(id, 500, None, 4_000),
        Some(Spending::Share(1_300))
    );
}

/// A window that has already climbed past the last reading before anyone looks.
///
/// Nothing about the figure says a window turned over here, so the figure alone would take
/// the whole of the new window for a few points of the old one.
#[test]
fn a_window_named_as_a_new_one_is_counted_whole_however_high_it_reads() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();
    sessions.measured(id, 1_900, Some(100), 2_000);

    // 2000 is above 1900, and only the window it is counted in tells the two apart.
    // 800 in the window the session opened in, and the whole of the 2000 in the next.
    assert_eq!(
        sessions.measured(id, 2_000, Some(200), 3_000),
        Some(Spending::Share(2_800))
    );
}

/// What a vendor that keeps naming the same window looks like.
#[test]
fn a_window_named_as_the_one_before_is_the_distance_from_the_last_look() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();
    sessions.measured(id, 1_900, Some(100), 2_000);

    assert_eq!(
        sessions.measured(id, 2_000, Some(100), 3_000),
        Some(Spending::Share(900))
    );
}

/// A vendor that names no window leaves the figure to say what it can.
#[test]
fn a_reading_below_the_last_stands_in_where_no_window_is_named() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();
    sessions.measured(id, 1_900, None, 2_000);

    assert_eq!(
        sessions.measured(id, 200, None, 3_000),
        Some(Spending::Share(1_000))
    );
}

/// The defect this rule was written for.
#[test]
fn a_session_that_crosses_a_reset_still_runs_out_of_what_it_declared() {
    let mut sessions = Sessions::default();
    let id = sessions.open(opening()).unwrap();
    // Declared 50%, so 5000 hundredths.
    sessions.measured(id, 5_100, None, 2_000);
    let spent = sessions.measured(id, 1_000, None, 3_000).unwrap();

    assert_eq!(spent, Spending::Share(5_000));
    assert_eq!(a_budget().left(spent), 0);
}

#[test]
fn a_session_declared_in_tokens_is_not_measured_against_the_limit() {
    let mut sessions = Sessions::default();
    let id = sessions
        .open(Opening {
            budget: Budget {
                usage: Usage::Tokens(2_000_000),
                time: Span(8 * 3_600),
            },
            limit_at_start: None,
            ..opening()
        })
        .unwrap();

    assert_eq!(
        sessions.measured(id, 1_400, None, 2_000),
        Some(Spending::Tokens(0))
    );
}

#[test]
fn nothing_is_measured_for_a_session_that_is_not_there() {
    let mut sessions = Sessions::default();

    assert_eq!(sessions.measured(SessionId(7), 1_400, None, 2_000), None);
}
