//! What answers the vendor ports, by running a program a definition describes.
//!
//! The means is an external program. Which program, what to hand it, and where each figure
//! sits in what it answers are all in the definition, so a second vendor is a file rather
//! than a module. What stays here is the part that is the same whoever the vendor is:
//! starting the child, ending its process group, reading its pipes, and following a path
//! into its answer.

pub mod agent;
pub mod definition;
pub mod path;

pub use definition::Definition;
