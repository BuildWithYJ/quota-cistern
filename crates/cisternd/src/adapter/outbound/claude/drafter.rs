//! Claude asked what a loose instruction is missing.
//!
//! A placeholder until the model is called: it proposes nothing, so a task is filled in only by
//! rule for now. Asking the model -- a cheaper one first, a stronger one when it is unsure --
//! lands next, and only this file changes when it does.

use crate::core::port::outbound::{Draft, Drafted, Drafter};

/// Proposes what a loose instruction is missing, by asking Claude.
pub struct ClaudeDrafter;

impl Drafter for ClaudeDrafter {
    fn draft(&self, ask: Draft<'_>) -> Option<Drafted> {
        // Not asking a model yet: take the ask in and propose nothing. Reading it to build a
        // prompt, a cheaper model first and a stronger one when unsure, is the next change here.
        let _ = (ask.instruction, ask.changed, ask.repository);
        None
    }
}
