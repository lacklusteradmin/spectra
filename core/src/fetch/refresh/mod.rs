//! Refreshing balances, split by the two questions it answers.
//!
//! [`policy`] decides *whether* a refresh is due — pure functions over clocks,
//! intervals and last-success timestamps, with no I/O. [`engine`] does the
//! refresh: it holds the timer, fetches, writes the result into core's own
//! wallet state, and calls the observer back.


pub mod engine;
pub mod policy;
