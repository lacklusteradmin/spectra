//! Cross-chain staking surface.
//!
//! Each chain has its own protocol-native staking model (Solana stake
//! accounts, Cardano stake-address certs, Cosmos-style nominate/bond on
//! Polkadot, Move calls on Sui/Aptos, function calls on NEAR, neuron
//! lock-ups on ICP). Rather than try to flatten them into a single
//! generic "stake" RPC, each chain owns a `<Chain>StakingClient` whose
//! method names mirror that chain's vocabulary.
//!
//! Shared types in this module describe what the UI cares about —
//! validators, positions, action previews — at a level chain-agnostic
//! enough that Swift can render them uniformly.

mod types;
pub use types::*;

pub mod chains {
    pub mod aptos;
    pub mod cardano;
    pub mod icp;
    pub mod near;
    pub mod polkadot;
    pub mod solana;
    pub mod sui;
}
