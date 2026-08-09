//! A session and the budget it was declared with.
//!
//! Section 2.2 of `docs/cli.md` fixes the arguments and the output, and section
//! 1 fixes the identifiers and the states. This module is private, so a value
//! that reached here was parsed on the way in.

use std::fmt::{self, Display};

/// A session number.
///
/// The core issues these. Sessions count on a sequence of their own, so
/// `session:1` and `task:1` are different things and neither number follows
/// the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(u32);

/// The two states section 1 lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Stopped,
}

/// Why a session stopped.
///
/// Nothing stops a session yet. The whole list is declared now because the rule
/// that only one session runs at a time cannot be written without a state that
/// means a session no longer does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoppedReason {
    BudgetHardlock,
    VendorLimit,
    ObservationUnreadable,
    Interrupted,
    AllDone,
    Error,
}

/// What `--usage` declared.
///
/// A share is measured against the configured plan and an absolute count is
/// not, so which one was written has to survive as far as the report that says
/// what was consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    /// A percentage of the configured plan, 1 to 100.
    Share(u32),
    /// A number of tokens.
    Tokens(u64),
}

/// What `--time` declared, as a number of seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span(u64);

/// What a session was told it may consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub usage: Usage,
    pub time: Span,
}

/// A session, as the core holds one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    id: SessionId,
    state: SessionState,
    stopped_reason: Option<StoppedReason>,
    budget: Budget,
    /// The model tasks fall back to when they name none.
    model: Option<String>,
}

/// A session on its way back from a store, with every value already read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub id: SessionId,
    pub state: SessionState,
    pub stopped_reason: Option<StoppedReason>,
    pub budget: Budget,
    pub model: Option<String>,
}

/// Every session, and the number the next one will get.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sessions {
    next_id: u32,
    sessions: Vec<Session>,
}

/// A set of sessions that no store could hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotASessionSet {
    /// Two sessions carry the same number.
    RepeatedId { id: SessionId },
    /// Two sessions are running, which section 2.2 does not allow.
    TwoRunning { first: SessionId, second: SessionId },
    /// A session that stopped does not say why, or one that runs claims to.
    ReasonDoesNotMatchState { id: SessionId },
}

/// Why a session could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotOpened {
    /// One is already running, and section 2.2 allows one at a time.
    AlreadyRunning { id: SessionId },
}

impl SessionId {
    /// Reads an identifier as a user writes it.
    ///
    /// The `session:` prefix may be left off, the same way section 2.1 lets it
    /// be left off a task.
    pub fn parse(id: &str) -> Option<Self> {
        let digits = id.strip_prefix("session:").unwrap_or(id);
        digits.parse().ok().map(SessionId)
    }

    /// The identifier as section 1 writes it.
    pub fn labelled(&self) -> String {
        format!("session:{}", self.0)
    }
}

impl Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl SessionState {
    /// Reads a state name. One spelling is read and written, so what a store
    /// holds and what is printed cannot drift apart.
    pub fn parse(state: &str) -> Option<Self> {
        match state {
            "running" => Some(SessionState::Running),
            "stopped" => Some(SessionState::Stopped),
            _ => None,
        }
    }
}

impl Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SessionState::Running => "running",
            SessionState::Stopped => "stopped",
        })
    }
}

impl StoppedReason {
    pub fn parse(reason: &str) -> Option<Self> {
        match reason {
            "budget hardlock" => Some(StoppedReason::BudgetHardlock),
            "vendor limit" => Some(StoppedReason::VendorLimit),
            "observation unreadable" => Some(StoppedReason::ObservationUnreadable),
            "interrupted" => Some(StoppedReason::Interrupted),
            "all done" => Some(StoppedReason::AllDone),
            "error" => Some(StoppedReason::Error),
            _ => None,
        }
    }
}

impl Display for StoppedReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StoppedReason::BudgetHardlock => "budget hardlock",
            StoppedReason::VendorLimit => "vendor limit",
            StoppedReason::ObservationUnreadable => "observation unreadable",
            StoppedReason::Interrupted => "interrupted",
            StoppedReason::AllDone => "all done",
            StoppedReason::Error => "error",
        })
    }
}

impl Usage {
    /// Reads what `--usage` was given.
    ///
    /// A trailing `%` is a share of the plan and anything else is a count of
    /// tokens, where `K` is a thousand and `M` a million. A share outside 1 to
    /// 100 is refused here, since section 2.2 fixes that range.
    pub fn parse(usage: &str) -> Option<Self> {
        if let Some(digits) = usage.strip_suffix('%') {
            let share = digits.parse().ok()?;
            return (1..=100).contains(&share).then_some(Usage::Share(share));
        }

        let (digits, scale) = match usage.strip_suffix('K') {
            Some(digits) => (digits, 1_000),
            None => match usage.strip_suffix('M') {
                Some(digits) => (digits, 1_000_000),
                None => (usage, 1),
            },
        };
        let count: u64 = digits.parse().ok()?;
        count
            .checked_mul(scale)
            .filter(|&n| n > 0)
            .map(Usage::Tokens)
    }
}

impl Display for Usage {
    /// The declaration as it was written, which is the unit section 2.2 says
    /// consumption is reported in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Usage::Share(share) => write!(f, "{share}%"),
            Usage::Tokens(count) => write!(f, "{count}"),
        }
    }
}

impl Span {
    /// Reads what `--time` was given.
    ///
    /// Hours, then minutes, then seconds, each at most once and at least one of
    /// them. `8h` and `2h30m` are what section 2.2 shows.
    pub fn parse(time: &str) -> Option<Self> {
        let mut left = time;
        let mut seconds: u64 = 0;
        let mut given = false;

        for (unit, scale) in [('h', 3_600), ('m', 60), ('s', 1)] {
            let Some(at) = left.find(unit) else { continue };
            let count: u64 = left[..at].parse().ok()?;
            seconds = seconds.checked_add(count.checked_mul(scale)?)?;
            left = &left[at + 1..];
            given = true;
        }

        // Anything left over sat after a unit or between two of them, which is
        // neither of the two spellings section 2.2 shows.
        (given && left.is_empty() && seconds > 0).then_some(Span(seconds))
    }
}

impl Display for Span {
    /// The shortest spelling that reads back as the same length of time.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (hours, minutes, seconds) = (self.0 / 3_600, (self.0 % 3_600) / 60, self.0 % 60);
        if hours > 0 {
            write!(f, "{hours}h")?;
        }
        if minutes > 0 {
            write!(f, "{minutes}m")?;
        }
        if seconds > 0 || self.0 == 0 {
            write!(f, "{seconds}s")?;
        }
        Ok(())
    }
}

impl Session {
    pub fn id(&self) -> SessionId {
        self.id
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn stopped_reason(&self) -> Option<StoppedReason> {
        self.stopped_reason
    }

    pub fn budget(&self) -> Budget {
        self.budget
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

impl Sessions {
    /// Opens a session and hands back the number it was given.
    ///
    /// One runs at a time, so this refuses while another is running rather than
    /// leaving the caller to look first and open second.
    pub fn open(&mut self, budget: Budget, model: Option<String>) -> Result<SessionId, NotOpened> {
        if let Some(running) = self.running() {
            return Err(NotOpened::AlreadyRunning { id: running.id });
        }

        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.sessions.push(Session {
            id,
            state: SessionState::Running,
            stopped_reason: None,
            budget,
            model,
        });
        Ok(id)
    }

    /// Stops a session and records why.
    ///
    /// A session that already stopped keeps the reason it stopped for, since
    /// the first one is what happened and the second is what noticed.
    pub fn stop(&mut self, id: SessionId, reason: StoppedReason) {
        for session in &mut self.sessions {
            if session.id == id && session.state == SessionState::Running {
                session.state = SessionState::Stopped;
                session.stopped_reason = Some(reason);
            }
        }
    }

    /// The session that is running, if one is.
    pub fn running(&self) -> Option<&Session> {
        self.sessions
            .iter()
            .find(|session| session.state == SessionState::Running)
    }

    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Takes back what a store held, and refuses a set no store should hold.
    ///
    /// Nobody is meant to write this file, so what it holds is checked the way
    /// a backlog is: as a whole, before anything reads one session out of it.
    pub fn restore(next_id: u32, held: Vec<Held>) -> Result<Self, NotASessionSet> {
        let mut sessions: Vec<Session> = Vec::with_capacity(held.len());

        for one in held {
            if let Some(same) = sessions.iter().find(|session| session.id == one.id) {
                return Err(NotASessionSet::RepeatedId { id: same.id });
            }
            let says_why = one.stopped_reason.is_some();
            if says_why != (one.state == SessionState::Stopped) {
                return Err(NotASessionSet::ReasonDoesNotMatchState { id: one.id });
            }
            if one.state == SessionState::Running
                && let Some(first) = sessions
                    .iter()
                    .find(|session| session.state == SessionState::Running)
            {
                return Err(NotASessionSet::TwoRunning {
                    first: first.id,
                    second: one.id,
                });
            }

            sessions.push(Session {
                id: one.id,
                state: one.state,
                stopped_reason: one.stopped_reason,
                budget: one.budget,
                model: one.model,
            });
        }

        Ok(Sessions { next_id, sessions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_budget() -> Budget {
        Budget {
            usage: Usage::Share(50),
            time: Span(8 * 3_600),
        }
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
        let first = sessions.open(a_budget(), None).unwrap();
        assert_eq!(first.labelled(), "session:1");

        sessions.stop(first, StoppedReason::AllDone);
        let second = sessions.open(a_budget(), None).unwrap();
        assert_eq!(second.labelled(), "session:2");
    }

    #[test]
    fn a_second_session_is_refused_while_one_is_running() {
        let mut sessions = Sessions::default();
        let first = sessions.open(a_budget(), None).unwrap();

        assert_eq!(
            sessions.open(a_budget(), None),
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
            StoppedReason::Error,
        ] {
            assert_eq!(StoppedReason::parse(&reason.to_string()), Some(reason));
        }
    }

    #[test]
    fn a_stopped_session_says_why_and_keeps_the_first_answer() {
        let mut sessions = Sessions::default();
        let id = sessions.open(a_budget(), None).unwrap();

        sessions.stop(id, StoppedReason::ObservationUnreadable);
        sessions.stop(id, StoppedReason::AllDone);

        let stopped = sessions.sessions().first().unwrap();
        assert_eq!(stopped.state(), SessionState::Stopped);
        assert_eq!(
            stopped.stopped_reason(),
            Some(StoppedReason::ObservationUnreadable)
        );
    }
}
