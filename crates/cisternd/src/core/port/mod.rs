//! The edges of the core.
//!
//! `outbound` is what the core requires of the outside. A port is written in
//! the core's own terms: no path, file format, or vendor name appears here.
//!
//! A port trait has to stay usable behind `dyn`, which rules out returning
//! `impl Trait` from one. Nothing needs it while a service takes the ports it
//! uses as arguments, but the ports will be gathered into a service value once
//! there are enough of them, and that value can only hold them as `dyn`
//! references without a type parameter per port. Honouring the constraint from
//! the start costs nothing; adopting it later would mean designing a trait
//! twice.

pub mod outbound;
