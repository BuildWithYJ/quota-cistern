//! What the core needs in order to run as a program, and nothing else.
//!
//! Nothing here touches a port, so none of it is an adapter.
//! It is gathered in one module rather than given a place per piece.
//! That way, code of this kind does not spread as more of it arrives.

pub mod serve;
pub mod signal;
pub mod work;
