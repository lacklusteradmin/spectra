//! The FFI boundary is enforced by attribute, not visibility: only
//! `#[uniffi::export]` and the `uniffi::` derives cross to Swift. `pub` alone
//! is a crate-public Rust API and stays invisible there.
//!
//! Exporting an `impl` block exports **every method in it**.

#![allow(clippy::too_many_arguments, clippy::type_complexity)]

uniffi::setup_scaffolding!();

pub mod amount;
pub mod api;
pub mod catalog;
pub mod derivation;
pub mod diagnostics;
pub mod error;
pub mod registry;
pub mod secrets;
pub mod send;
pub mod service;
pub mod staking;
pub mod store;
pub mod validation;
pub mod wallet_db;

#[cfg(test)]
#[path = "tests/app_boundary.rs"]
mod app_boundary_tests;
