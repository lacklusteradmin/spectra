//! Staking validator directory queries.
//! Chain clients return shared metadata for the CLI and platform views.

mod types;
pub use types::*;

pub mod service;
pub use service::StakingService;

pub mod aptos;
pub mod icp;
pub mod near;
pub mod polkadot;
pub mod solana;
pub mod sui;
