//! The entities and the rules over them.
//!
//! One file per concept. The module is private, so what a file inside declares
//! public is still out of reach from outside `core`.

mod configuration;

pub use configuration::{Configuration, Key, Setting};
