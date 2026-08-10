//! What a session store hands over, and what goes back to it.
//!
//! The same job `service::backlog` does for the backlog. The two stay apart
//! because neither store knows what the other holds, and both are read by more
//! than one command group.

use crate::core::{
    domain::{Budget, Held, NotASessionSet, Sessions, Span, Usage},
    port::{
        inbound::Refusal,
        outbound::{SessionStore, StoredSession, StoredSessions},
    },
};

pub fn stored_count(field: &str, value: &str) -> Result<u64, Refusal> {
    value.parse().map_err(|_| unreadable(field, value))
}

/// Reads the sessions and holds them to the same standard as an argument.
///
/// Nobody is meant to write this file, so a set that does not add up is a store
/// this core cannot use rather than something the user typed wrong. This is
/// what `service::backlog` does for the backlog, and the two stay apart because
/// neither store knows what the other holds.
pub fn read_from(stored: StoredSessions) -> Result<Sessions, Refusal> {
    let next_id = stored
        .next_id
        .parse()
        .map_err(|_| unreadable("next_id", &stored.next_id))?;

    let mut held = Vec::with_capacity(stored.sessions.len());
    for one in stored.sessions {
        held.push(held_from(one)?);
    }

    Sessions::restore(next_id, held).map_err(|e| Refusal::Unavailable {
        reason: unusable(&e),
    })
}

/// Reads one session as a store handed it over.
fn held_from(one: StoredSession) -> Result<Held, Refusal> {
    use crate::core::domain::{SessionId, SessionState, StoppedReason};

    Ok(Held {
        started_at: stored_count("started_at", &one.started_at)?,
        limit_at_start: one
            .limit_at_start
            .as_deref()
            .map(|used| stored_count("limit_at_start", used))
            .transpose()?,
        id: SessionId::parse(&one.id).ok_or_else(|| unreadable("id", &one.id))?,
        state: SessionState::parse(&one.state).ok_or_else(|| unreadable("state", &one.state))?,
        stopped_reason: one
            .stopped_reason
            .as_deref()
            .map(|reason| {
                StoppedReason::parse(reason).ok_or_else(|| unreadable("stopped_reason", reason))
            })
            .transpose()?,
        budget: Budget {
            usage: Usage::parse(&one.usage).ok_or_else(|| unreadable("usage", &one.usage))?,
            time: Span::parse(&one.time).ok_or_else(|| unreadable("time", &one.time))?,
        },
        model: one.model,
    })
}

/// Hands the sessions to a store as the text a user would have typed.
fn written(sessions: &Sessions) -> StoredSessions {
    StoredSessions {
        next_id: sessions.next_id().to_string(),
        sessions: sessions
            .sessions()
            .iter()
            .map(|session| StoredSession {
                id: session.id().to_string(),
                state: session.state().to_string(),
                stopped_reason: session.stopped_reason().map(|why| why.to_string()),
                usage: session.budget().usage.to_string(),
                time: session.budget().time.to_string(),
                model: session.model().map(str::to_owned),
                started_at: session.started_at().to_string(),
                limit_at_start: session.limit_at_start().map(|used| used.to_string()),
            })
            .collect(),
    }
}

/// Reads the sessions, changes them, and writes them back as one step, for the
/// reason `service::backlog` gives.
pub fn change<T>(
    store: &dyn SessionStore,
    with: impl FnOnce(&mut Sessions) -> Result<T, Refusal>,
) -> Result<T, Refusal> {
    let mut with = Some(with);
    let mut answer = None;

    store.update(&mut |stored| {
        let Some(with) = with.take() else {
            return false;
        };

        let done = read_from(stored.clone()).and_then(|mut sessions| {
            let got = with(&mut sessions)?;
            Ok((got, sessions))
        });
        match done {
            Ok((got, sessions)) => {
                *stored = written(&sessions);
                answer = Some(Ok(got));
                true
            }
            Err(e) => {
                answer = Some(Err(e));
                false
            }
        }
    })?;

    answer.unwrap_or_else(|| {
        Err(Refusal::Unavailable {
            reason: "the store did not run the change it was given".to_owned(),
        })
    })
}

pub fn unreadable(field: &str, value: &str) -> Refusal {
    Refusal::Unavailable {
        reason: format!("the sessions hold {value} where {field} belongs"),
    }
}

fn unusable(e: &NotASessionSet) -> String {
    match e {
        NotASessionSet::RepeatedId { id } => format!("the sessions hold session:{id} twice"),
        NotASessionSet::TwoRunning { first, second } => {
            format!("session:{first} and session:{second} are both running")
        }
        NotASessionSet::ReasonDoesNotMatchState { id } => {
            format!("session:{id} does not say why it stopped, or says so while running")
        }
    }
}
