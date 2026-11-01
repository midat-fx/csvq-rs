//! csvq: a byte-for-byte Rust port of the Bash + awk `csvq` toolkit.
//! The binary in main.rs reproduces the csvq.sh wrapper; each module here
//! reproduces one legacy awk program.

pub mod audit;
pub mod awk;
pub mod csv;
pub mod dedupe;
pub mod diff;
pub mod stats;
pub mod validate;
