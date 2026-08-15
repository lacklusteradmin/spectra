//! Refreshing balances, split by the two questions it answers.
//!
//! [`policy`] decides *whether* a refresh is due — pure functions over clocks,
//! intervals and last-success timestamps, with no I/O. [`engine`] does the
//! refresh: it holds the timer, fetches, writes the result into core's own
//! wallet state, and calls the observer back.
//!
//! They were `refresh.rs` and `refresh_engine.rs`, a pair of names that said
//! which was written first rather than what either one owns.

pub mod engine;
pub mod policy;
