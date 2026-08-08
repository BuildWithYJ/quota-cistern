//! Where a port meets the technology that answers it.
//!
//! An adapter has a port on one side and something outside on the other. Code
//! that touches no port is not an adapter however technical it is, and lives in
//! `platform` instead.

pub mod inbound;
pub mod outbound;
