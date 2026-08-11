//! The edges of the core.
//!
//! `inbound` is what the core offers, and `outbound` is what it requires of the outside.
//! A port is written in the core's own terms either way: no path, file format, or vendor name appears here.
//!
//! A port trait stays usable behind `dyn`, which rules out returning `impl Trait` from one.
//! Nothing needs that yet, but a service value holding several ports could only hold them as `dyn` references.

pub mod inbound;
pub mod outbound;
