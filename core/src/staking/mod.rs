//! Staking validator directory queries.
//! Chain clients return shared metadata for the CLI and platform views.

mod types;
pub use types::*;

pub mod aptos;
pub mod icp;
pub mod near;
pub mod polkadot;
pub mod solana;
pub mod sui;

use crate::registry::Chain;

/// A validator's or pool's id cut to its first eight characters after any
/// `0x`, for the name of one that gives none.
fn short_id(id: &str) -> &str {
    let keep = if id.starts_with("0x") { 10 } else { 8 };
    id.get(..keep).unwrap_or(id)
}

/// The validators, pools or known neurons `chain` stakes with, read from
/// `endpoints`. A chain the registry does not stake on is refused here
/// rather than falling through an arm nobody wrote down.
pub(crate) async fn fetch_validators(
    chain: Chain,
    endpoints: Vec<String>,
) -> Result<Vec<StakingValidator>, StakingError> {
    match chain {
        Chain::Solana => {
            solana::SolanaStakingClient::new(endpoints)
                .fetch_validators()
                .await
        }
        Chain::Sui => {
            sui::SuiStakingClient::new(endpoints)
                .fetch_validators()
                .await
        }
        Chain::Aptos => {
            aptos::AptosStakingClient::new(endpoints)
                .fetch_validators()
                .await
        }
        Chain::Near => {
            near::NearStakingClient::new(endpoints)
                .fetch_validators()
                .await
        }
        Chain::Polkadot => {
            polkadot::PolkadotStakingClient::new(endpoints)
                .fetch_validators()
                .await
        }
        Chain::Icp => {
            icp::IcpStakingClient::new(endpoints)
                .fetch_validators()
                .await
        }
        _ => Err(StakingError::NotYetImplemented),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eight characters of every network's id, its `0x` aside.
    #[test]
    fn an_unnamed_validator_is_named_by_eight_characters_of_its_id() {
        assert_eq!(
            short_id("Vote111111111111111111111111111111111111111"),
            "Vote1111"
        );
        assert_eq!(short_id("0xabcdef0123456789"), "0xabcdef01");
        assert_eq!(short_id("0xabc"), "0xabc");
        assert_eq!(short_id("pool"), "pool");
    }

    /// The registry's staking chains and the validator dispatch are one
    /// answer. Offline: a client refuses missing endpoints, so a routed call
    /// and a refused one differ without a network.
    #[tokio::test]
    async fn the_registry_flag_and_the_dispatch_agree_on_every_chain() {
        for chain in Chain::all() {
            let routed = !matches!(
                fetch_validators(chain, Vec::new()).await,
                Err(StakingError::NotYetImplemented)
            );
            assert_eq!(routed, chain.supports_staking(), "{}", chain.str_id());
        }
    }

    /// A testnet never stakes, even where its mainnet does: the clients are
    /// built against mainnet programs and pools, so routing Solana Devnet to
    /// the Solana client would report mainnet validators for a devnet wallet.
    #[test]
    fn staking_is_a_mainnet_answer() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            assert!(!chain.supports_staking(), "{}", chain.str_id());
        }
        assert!(Chain::Solana.supports_staking());
        assert!(!Chain::SolanaDevnet.supports_staking());
    }
}
