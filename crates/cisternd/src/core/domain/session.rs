//! A session and the budget it was declared with.
//!
//! Section 2.2 of `docs/cli.md` fixes the arguments and the output, and section 1 fixes the identifiers and the states.
//! This module is private, so a value that reached here was parsed on the way in.

use std::fmt::{self, Display};

use super::Spending;

/// A session number.
///
/// Sessions count on a sequence of their own, so `session:1` and `task:1` are different things.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(u32);

/// The two states section 1 lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Stopped,
}

/// Why a session stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoppedReason {
    BudgetHardlock,
    VendorLimit,
    ObservationUnreadable,
    Interrupted,
    AllDone,
    /// Every task left waits on one that did not complete.
    ///
    /// Apart from `AllDone`, which used to cover it and said the opposite of what happened.
    /// A ceiling makes this ordinary rather than rare: a task cut off at one ends
    /// `Interrupted`, and a task that waits on it may never start.
    Blocked,
    Error,
}

/// What `--usage` declared.
///
/// Which one was written survives as far as the report of what was consumed.
/// A share is measured against the vendor's limit and a count is not.
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

/// What a session is opened with.
///
/// The clock reading and the vendor's limit are taken outside and handed in, so that nothing here reaches for either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub budget: Budget,
    pub model: Option<String>,
    pub started_at: u64,
    pub limit_at_start: Option<u64>,
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
    /// Where the time limit is measured from.
    started_at: u64,
    /// Where a share is measured from, in hundredths of a percent.
    ///
    /// A share says what this session may add to what the limit already held.
    /// A session declared in tokens has none.
    limit_at_start: Option<u64>,
    /// What the limit read the last time this session looked, in hundredths of a percent.
    ///
    /// A share is added up look by look rather than taken as the distance from where it started.
    /// The limit is a window that begins again every few hours, and a session that outlives one
    /// reads a figure lower than the one it opened from.
    limit_last_seen: Option<u64>,
    /// What it has consumed, in the unit it was declared in.
    consumed: Spending,
    /// When it last changed.
    updated_at: u64,
    /// When the window the limit was last read in begins again.
    ///
    /// What tells one window from the next. Section 2.2 reports it for a session the vendor
    /// turned away, which is why the report asks why the session stopped rather than whether
    /// this is set.
    resets_at: Option<u64>,
}

/// A session on its way back from a store, with every value already read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub id: SessionId,
    pub state: SessionState,
    pub stopped_reason: Option<StoppedReason>,
    pub budget: Budget,
    pub model: Option<String>,
    pub started_at: u64,
    pub limit_at_start: Option<u64>,
    pub limit_last_seen: Option<u64>,
    pub consumed: Spending,
    pub updated_at: u64,
    pub resets_at: Option<u64>,
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
    /// The `session:` prefix may be left off, the same way section 2.1 lets it be left off a task.
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
    /// Reads a state name.
    ///
    /// One spelling is read and written, so what a store holds and what is printed cannot drift apart.
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
            "blocked" => Some(StoppedReason::Blocked),
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
            StoppedReason::Blocked => "blocked",
            StoppedReason::Error => "error",
        })
    }
}

impl Usage {
    /// Reads what `--usage` was given.
    ///
    /// A trailing `%` is a share and anything else is tokens, where `K` is a thousand and `M` a million.
    /// Section 2.2 fixes a share at 1 to 100.
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
    /// The declaration as it was written, which is the unit section 2.2 says consumption is reported in.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Usage::Share(share) => write!(f, "{share}%"),
            Usage::Tokens(count) => write!(f, "{count}"),
        }
    }
}

impl Span {
    /// A length of time from a number of seconds.
    pub fn of(seconds: u64) -> Self {
        Span(seconds)
    }

    /// The length of time, in seconds.
    pub fn seconds(&self) -> u64 {
        self.0
    }

    /// Reads what `--time` was given.
    ///
    /// Hours, then minutes, then seconds, each at most once and at least one of them.
    /// `8h` and `2h30m` are what section 2.2 shows.
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

        // Anything left over sat after a unit or between two of them.
        // Which is neither of the two spellings section 2.2 shows.
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

    pub fn started_at(&self) -> u64 {
        self.started_at
    }

    pub fn limit_at_start(&self) -> Option<u64> {
        self.limit_at_start
    }

    pub fn limit_last_seen(&self) -> Option<u64> {
        self.limit_last_seen
    }

    /// What it has consumed, in the unit it was declared in.
    pub fn consumed(&self) -> Spending {
        self.consumed
    }

    pub fn updated_at(&self) -> u64 {
        self.updated_at
    }

    pub fn resets_at(&self) -> Option<u64> {
        self.resets_at
    }

    /// Whether the session has run for as long as it declared.
    pub fn out_of_time(&self, now: u64) -> bool {
        now.saturating_sub(self.started_at) >= self.budget.time.seconds()
    }

    /// How much of the time it declared is left, in seconds.
    ///
    /// Nought once it has run that long, which `out_of_time` says the same way. Something has
    /// to wake at that moment, since a session with one long run going is a session nothing
    /// else would ask about until that run ended.
    pub fn time_left(&self, now: u64) -> u64 {
        self.budget
            .time
            .seconds()
            .saturating_sub(now.saturating_sub(self.started_at))
    }
}

impl Sessions {
    /// Opens a session and hands back the number it was given.
    ///
    /// One runs at a time.
    /// This refuses while another is running rather than leaving the caller to look first and open second.
    pub fn open(&mut self, opening: Opening) -> Result<SessionId, NotOpened> {
        if let Some(running) = self.running() {
            return Err(NotOpened::AlreadyRunning { id: running.id });
        }

        let id = SessionId(self.next_id);
        self.next_id += 1;
        self.sessions.push(Session {
            id,
            state: SessionState::Running,
            stopped_reason: None,
            budget: opening.budget,
            model: opening.model,
            started_at: opening.started_at,
            limit_at_start: opening.limit_at_start,
            // The first look is the one taken to open with.
            limit_last_seen: opening.limit_at_start,
            consumed: match opening.budget.usage {
                Usage::Share(_) => Spending::Share(0),
                Usage::Tokens(_) => Spending::Tokens(0),
            },
            updated_at: opening.started_at,
            resets_at: None,
        });
        Ok(id)
    }

    /// Stops a session and records why.
    ///
    /// A session that already stopped keeps the reason it stopped for.
    /// The first one is what happened, and the second is what noticed.
    pub fn stop(&mut self, id: SessionId, reason: StoppedReason, now: u64) {
        for session in &mut self.sessions {
            if session.id == id && session.state == SessionState::Running {
                session.state = SessionState::Stopped;
                session.stopped_reason = Some(reason);
                session.updated_at = now;
            }
        }
    }

    /// Records what a session has consumed, and when the reading was taken.
    ///
    /// A share cannot be worked out later.
    /// What was read while the session ran is what is reported for it afterwards.
    ///
    /// Two things are turned away. A session that has stopped keeps what it consumed when it
    /// did: a record arriving afterwards would report a figure from a moment the session was
    /// no longer running, and `updated_at` is what `elapsed` reads as the moment it stopped.
    /// And a figure below the one stored is one that was read earlier, since a session only
    /// ever spends more; the readings are taken outside the hold this is called under, so two
    /// of them can arrive in the other order from the one they were read in.
    pub fn record(&mut self, id: SessionId, consumed: Spending, now: u64) {
        for session in &mut self.sessions {
            if session.id != id {
                continue;
            }
            if session.state != SessionState::Running {
                return;
            }
            if consumed.behind(&session.consumed) {
                return;
            }
            session.consumed = consumed;
            session.updated_at = now;
        }
    }

    /// Adds what the vendor's limit has moved since this session last looked at it, and says
    /// what the session has consumed once it has.
    ///
    /// `resets_at` is when the window this reading was taken in begins again, and a different
    /// one from the last look is a window that has begun again, so the whole of the reading was
    /// spent since it did. A reading lower than the last one says the same thing and stands in
    /// where the vendor named no window, since a limit only climbs while one window lasts.
    ///
    /// Whatever was spent between the last look and the window turning over is in no reading at
    /// all, so the figure falls short by that much rather than by a whole window.
    ///
    /// A reading is taken outside the hold this is called under, since asking the vendor takes
    /// as long as ninety seconds and nothing else could be answered while it did. Two threads
    /// can therefore arrive here in the other order from the one they read in, and the later
    /// arrival carries the older figure. Where the vendor named the window and named the same
    /// one, that is what a lower reading means, and it is left out rather than counted as a
    /// window that began again.
    pub fn measured(
        &mut self,
        id: SessionId,
        used: u64,
        resets_at: Option<u64>,
        now: u64,
    ) -> Option<Spending> {
        let session = self.sessions.iter_mut().find(|session| session.id == id)?;
        let Spending::Share(consumed) = session.consumed else {
            return Some(session.consumed);
        };
        let began_again = match (session.resets_at, resets_at) {
            (Some(seen), Some(now_at)) => now_at != seen,
            // Nothing to compare against says nothing about the window.
            _ => false,
        };

        // Within one window a limit only climbs, so a lower reading of the window already
        // being measured was taken before the last look. Counting it would add a whole
        // window to a session that had spent none of it. Nothing is written down: the look
        // this session is measured from stays the later one.
        let named_the_same_window =
            !began_again && session.resets_at.is_some() && resets_at.is_some();
        if named_the_same_window && session.limit_last_seen.is_some_and(|seen| used < seen) {
            return Some(session.consumed);
        }

        let since = match session.limit_last_seen {
            Some(_) if began_again => used,
            Some(seen) if used >= seen => used - seen,
            // A window nobody named, found lower than the last look.
            Some(_) => used,
            // Nothing to measure from: a share opened without a first look, or one a store
            // held before this was written down. Adding what it has already counted a second
            // time would be worse than adding nothing, and this look becomes the one the next
            // is measured from.
            None => 0,
        };
        session.consumed = Spending::Share(consumed.saturating_add(since));
        session.limit_last_seen = Some(used);
        if resets_at.is_some() {
            session.resets_at = resets_at;
        }
        session.updated_at = now;
        Some(session.consumed)
    }

    /// Records when the vendor's limit starts over.
    pub fn resets_at(&mut self, id: SessionId, at: u64) {
        for session in &mut self.sessions {
            if session.id == id {
                session.resets_at = Some(at);
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
    /// Nobody is meant to write this file.
    /// What it holds is checked the way a backlog is: as a whole, before anything reads one session out of it.
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
                started_at: one.started_at,
                limit_at_start: one.limit_at_start,
                limit_last_seen: one.limit_last_seen,
                consumed: one.consumed,
                updated_at: one.updated_at,
                resets_at: one.resets_at,
            });
        }

        Ok(Sessions { next_id, sessions })
    }
}

#[cfg(test)]
mod tests;
