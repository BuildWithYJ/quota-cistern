//! What decides, and what it needs from outside.
//!
//! ADR 0002 records this arrangement.
//! `domain` is private, so nothing outside `core` names an entity.
//! What crosses either edge is declared in `port`, and `service` is what answers the inbound side of it.

mod domain;
pub mod port;
pub mod service;
