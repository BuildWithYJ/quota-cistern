//! What `run` does.

use crate::core::{
    domain::{Budget, Held, Key, NotASessionSet, NotOpened, Sessions, Setting, Span, Usage},
    port::{
        inbound::{Declaration, Declared, ExecutionUseCase, Refusal, Started},
        outbound::{ConfigurationStore, SessionStore, StoredSession, StoredSessions},
    },
};

/// The commands over sessions, and what they need from outside.
///
/// The configuration is here because a share of a plan cannot be declared
/// without one, which is the only thing this reads it for.
pub struct ExecutionService<'a> {
    sessions: &'a dyn SessionStore,
    configuration: &'a dyn ConfigurationStore,
}

impl<'a> ExecutionService<'a> {
    pub fn new(sessions: &'a dyn SessionStore, configuration: &'a dyn ConfigurationStore) -> Self {
        ExecutionService {
            sessions,
            configuration,
        }
    }

    /// Whether a plan is configured, which is what a share is measured against.
    ///
    /// A plan that is there and cannot be read is a store this core cannot use,
    /// the same as any other value a file holds wrongly.
    fn plan_is_configured(&self) -> Result<bool, Refusal> {
        let Some(plan) = self.configuration.load()?.plan else {
            return Ok(false);
        };
        match Setting::parse(Key::Plan, &plan) {
            Some(_) => Ok(true),
            None => Err(Refusal::Unavailable {
                reason: format!("the configuration holds {plan} where plan belongs"),
            }),
        }
    }
}

impl ExecutionUseCase for ExecutionService<'_> {
    fn run(&self, declared: Declaration<'_>) -> Result<Started, Refusal> {
        let usage = Usage::parse(declared.usage).ok_or_else(|| Refusal::BadValue {
            key: "usage".to_owned(),
            value: declared.usage.to_owned(),
        })?;
        let time = Span::parse(declared.time).ok_or_else(|| Refusal::BadValue {
            key: "time".to_owned(),
            value: declared.time.to_owned(),
        })?;

        // Asked before the sessions are read, since a share with no plan is
        // refused whatever else is running.
        if matches!(usage, Usage::Share(_)) && !self.plan_is_configured()? {
            return Err(Refusal::NoPlanConfigured);
        }

        let budget = Budget { usage, time };
        let model = declared.model.map(str::to_owned);

        change(self.sessions, |sessions| {
            let id = sessions
                .open(budget, model)
                .map_err(|NotOpened::AlreadyRunning { id }| Refusal::AlreadyRunning {
                    id: id.labelled(),
                })?;

            Ok(Started {
                session: id.labelled(),
                state: sessions
                    .running()
                    .map_or_else(String::new, |session| session.state().to_string()),
                // Nothing assigns a task yet.
                assigned: 0,
                budget: Declared {
                    usage: usage.to_string(),
                    time: time.to_string(),
                },
            })
        })
    }
}

/// Reads the sessions and holds them to the same standard as an argument.
///
/// Nobody is meant to write this file, so a set that does not add up is a store
/// this core cannot use rather than something the user typed wrong. This is
/// what `service::backlog` does for the backlog, and the two stay apart because
/// neither store knows what the other holds.
fn read_from(stored: StoredSessions) -> Result<Sessions, Refusal> {
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
            })
            .collect(),
    }
}

/// Reads the sessions, changes them, and writes them back as one step, for the
/// reason `service::backlog` gives.
fn change<T>(
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

fn unreadable(field: &str, value: &str) -> Refusal {
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::core::port::outbound::{StoredConfiguration, Unavailable};

    use super::*;

    /// Sessions held in memory, so the steps can be checked without a file.
    #[derive(Default)]
    struct Remembered {
        stored: RefCell<StoredSessions>,
    }

    impl Remembered {
        fn empty() -> Self {
            Remembered {
                stored: RefCell::new(StoredSessions {
                    next_id: "1".to_owned(),
                    sessions: Vec::new(),
                }),
            }
        }
    }

    impl Remembered {
        fn load(&self) -> StoredSessions {
            self.stored.borrow().clone()
        }
    }

    impl SessionStore for Remembered {
        fn update(
            &self,
            change: &mut dyn FnMut(&mut StoredSessions) -> bool,
        ) -> Result<(), Unavailable> {
            let mut sessions = self.load();
            if change(&mut sessions) {
                *self.stored.borrow_mut() = sessions;
            }
            Ok(())
        }
    }

    /// A configuration held in memory.
    struct Configured {
        plan: Option<&'static str>,
    }

    impl ConfigurationStore for Configured {
        fn load(&self) -> Result<StoredConfiguration, Unavailable> {
            Ok(StoredConfiguration {
                vendor: None,
                plan: self.plan.map(str::to_owned),
                usage_limit: None,
            })
        }

        fn store(&self, _stored: &StoredConfiguration) -> Result<(), Unavailable> {
            Ok(())
        }
    }

    static ON_A_PLAN: Configured = Configured {
        plan: Some("max-20x"),
    };
    static ON_NO_PLAN: Configured = Configured { plan: None };

    fn declaring<'a>(usage: &'a str, time: &'a str) -> Declaration<'a> {
        Declaration {
            usage,
            time,
            model: None,
        }
    }

    #[test]
    fn a_session_opens_and_answers_what_it_was_declared_with() {
        let sessions = Remembered::empty();
        let execution = ExecutionService::new(&sessions, &ON_A_PLAN);

        let started = execution.run(declaring("50%", "8h")).unwrap();
        assert_eq!(started.session, "session:1");
        assert_eq!(started.state, "running");
        assert_eq!(started.assigned, 0);
        assert_eq!(started.budget.usage, "50%");
        assert_eq!(started.budget.time, "8h");
    }

    #[test]
    fn what_was_opened_is_there_for_the_next_command_to_read() {
        let sessions = Remembered::empty();
        let execution = ExecutionService::new(&sessions, &ON_A_PLAN);
        execution.run(declaring("2M", "30m")).unwrap();

        let held = sessions.load();
        assert_eq!(held.next_id, "2");
        assert_eq!(held.sessions.len(), 1);
        assert_eq!(held.sessions[0].state, "running");
        assert_eq!(held.sessions[0].usage, "2000000");
    }

    #[test]
    fn a_second_session_is_refused_while_one_is_running() {
        let sessions = Remembered::empty();
        let execution = ExecutionService::new(&sessions, &ON_A_PLAN);
        execution.run(declaring("50%", "8h")).unwrap();

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::AlreadyRunning {
                id: "session:1".to_owned()
            })
        );
        // The refused command left the store as it was.
        assert_eq!(sessions.load().sessions.len(), 1);
    }

    #[test]
    fn a_share_of_a_plan_nobody_configured_is_refused() {
        let sessions = Remembered::empty();
        let execution = ExecutionService::new(&sessions, &ON_NO_PLAN);

        assert_eq!(
            execution.run(declaring("50%", "8h")),
            Err(Refusal::NoPlanConfigured)
        );
    }

    /// An absolute count is measured against nothing, so it needs no plan.
    #[test]
    fn a_count_of_tokens_does_not_need_a_plan() {
        let sessions = Remembered::empty();
        let execution = ExecutionService::new(&sessions, &ON_NO_PLAN);

        assert!(execution.run(declaring("2M", "8h")).is_ok());
    }

    #[test]
    fn a_declaration_that_cannot_be_read_is_refused_as_a_bad_argument() {
        let sessions = Remembered::empty();
        let execution = ExecutionService::new(&sessions, &ON_A_PLAN);

        assert_eq!(
            execution.run(declaring("50%", "8x")),
            Err(Refusal::BadValue {
                key: "time".to_owned(),
                value: "8x".to_owned()
            })
        );
        assert_eq!(
            execution.run(declaring("half", "8h")),
            Err(Refusal::BadValue {
                key: "usage".to_owned(),
                value: "half".to_owned()
            })
        );
    }

    #[test]
    fn a_stored_session_that_cannot_be_read_fails_as_a_store() {
        let sessions = Remembered {
            stored: RefCell::new(StoredSessions {
                next_id: "2".to_owned(),
                sessions: vec![StoredSession {
                    id: "1".to_owned(),
                    state: "sprinting".to_owned(),
                    stopped_reason: None,
                    usage: "50%".to_owned(),
                    time: "8h".to_owned(),
                    model: None,
                }],
            }),
        };
        let execution = ExecutionService::new(&sessions, &ON_A_PLAN);

        let refused = execution.run(declaring("50%", "8h")).unwrap_err();
        assert!(matches!(refused, Refusal::Unavailable { reason } if reason.contains("sprinting")),);
    }
}
