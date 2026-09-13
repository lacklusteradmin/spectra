//! Internal staking query dispatcher, configured by WalletService.

use std::sync::Arc;

use crate::registry::Chain;
use crate::service::ChainEndpoints;
use crate::staking::{
    chains::{
        aptos::AptosStakingClient, cardano::CardanoStakingClient, icp::IcpStakingClient,
        near::NearStakingClient, polkadot::PolkadotStakingClient, solana::SolanaStakingClient,
        sui::SuiStakingClient,
    },
    StakingError, StakingPosition, StakingValidator,
};

pub struct StakingService {
    solana: SolanaStakingClient,
    cardano: CardanoStakingClient,
    sui: SuiStakingClient,
    aptos: AptosStakingClient,
    near: NearStakingClient,
    polkadot: PolkadotStakingClient,
    icp: IcpStakingClient,
}

impl StakingService {
    /// The chain a staking call is for, refused before it reaches a client.
    ///
    /// `Chain::supports_staking` is what the picker is built from, so gating
    /// here is what makes the two the same answer: a chain the app can select
    /// is a chain this routes, and one it cannot is `NotYetImplemented` rather
    /// than a fall-through nobody stated.
    fn staking_chain(&self, chain_id: &str) -> Result<Chain, StakingError> {
        Chain::from_str_id(chain_id)
            .filter(|chain| chain.supports_staking())
            .ok_or(StakingError::NotYetImplemented)
    }
}

impl StakingService {
    pub fn new(endpoints: Vec<ChainEndpoints>) -> Arc<Self> {
        // Seven `const CHAIN_*: &str` spellings used to sit above this, and
        // `"internet-computer"` among them was one typo away from a staking tab
        // that silently listed no validators. The registry is the one speller.
        let eps = |chain: Chain| -> Vec<String> {
            endpoints
                .iter()
                .find(|e| e.chain_id == chain.str_id())
                .map(|e| e.endpoints.clone())
                .unwrap_or_default()
        };
        let cardano_api_key = endpoints
            .iter()
            .find(|e| e.chain_id == Chain::Cardano.str_id())
            .and_then(|e| e.api_key.clone());
        Arc::new(Self {
            solana: SolanaStakingClient::new(eps(Chain::Solana)),
            cardano: CardanoStakingClient::new(eps(Chain::Cardano), cardano_api_key),
            sui: SuiStakingClient::new(eps(Chain::Sui)),
            aptos: AptosStakingClient::new(eps(Chain::Aptos)),
            near: NearStakingClient::new(eps(Chain::Near)),
            polkadot: PolkadotStakingClient::new(eps(Chain::Polkadot)),
            icp: IcpStakingClient::new(eps(Chain::Icp)),
        })
    }

    // ── Common ───────────────────────────────────────────────────────────────

    pub async fn fetch_validators(
        &self,
        chain_id: String,
    ) -> Result<Vec<StakingValidator>, StakingError> {
        match self.staking_chain(&chain_id)? {
            Chain::Solana => self.solana.fetch_validators().await,
            Chain::Cardano => self.cardano.fetch_validators().await,
            Chain::Sui => self.sui.fetch_validators().await,
            Chain::Aptos => self.aptos.fetch_validators().await,
            Chain::Near => self.near.fetch_validators().await,
            Chain::Polkadot => self.polkadot.fetch_validators().await,
            Chain::Icp => self.icp.fetch_validators().await,
            _ => Err(StakingError::NotYetImplemented),
        }
    }

    pub async fn fetch_positions(
        &self,
        chain_id: String,
        wallet_address: String,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        match self.staking_chain(&chain_id)? {
            Chain::Solana => self.solana.fetch_positions(&wallet_address).await,
            Chain::Cardano => self.cardano.fetch_positions(&wallet_address).await,
            Chain::Sui => self.sui.fetch_positions(&wallet_address).await,
            Chain::Aptos => self.aptos.fetch_positions(&wallet_address).await,
            Chain::Near => self.near.fetch_positions(&wallet_address).await,
            Chain::Polkadot => self.polkadot.fetch_positions(&wallet_address).await,
            Chain::Icp => self.icp.fetch_positions(&wallet_address).await,
            _ => Err(StakingError::NotYetImplemented),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picker's list and the dispatch's arms are one answer.
    ///
    /// Offline: every client returns an empty list when it was given no
    /// endpoints, so a routed call and a refused one are distinguishable
    /// without a network. That is the whole property — a chain the registry
    /// says stakes reaches a client, and one it does not is refused rather
    /// than falling through an arm nobody wrote down.
    #[tokio::test]
    async fn the_registry_flag_and_the_dispatch_agree_on_every_chain() {
        let service = StakingService::new(vec![]);
        for chain in Chain::all() {
            let routed = !matches!(
                service.fetch_validators(chain.str_id().to_string()).await,
                Err(StakingError::NotYetImplemented)
            );
            assert_eq!(
                routed,
                chain.supports_staking(),
                "{}: supports_staking = {} but the dispatch routed it = {routed}",
                chain.chain_display_name(),
                chain.supports_staking()
            );
        }
    }

    /// A testnet never stakes, even where its mainnet does.
    ///
    /// The clients are built against mainnet endpoints and mainnet contract
    /// addresses, so routing Solana Devnet to the Solana client would report
    /// mainnet validators for a devnet wallet. `supports_staking` is exact for
    /// that reason and this is what says so.
    #[test]
    fn staking_is_a_mainnet_answer() {
        for chain in Chain::all().filter(|c| c.is_testnet()) {
            assert!(
                !chain.supports_staking(),
                "{} is a testnet and claims staking",
                chain.chain_display_name()
            );
        }
        assert!(Chain::Solana.supports_staking());
        assert!(!Chain::SolanaDevnet.supports_staking());
    }

    /// What staking does *not* cover, named rather than implied.
    #[test]
    fn these_chains_do_not_stake() {
        for chain in [
            Chain::Bitcoin,
            Chain::Ethereum,
            Chain::Tron,
            Chain::Xrp,
            Chain::Monero,
        ] {
            assert!(
                !chain.supports_staking(),
                "{} now stakes — that is a new client, so say so in PLAN.md and \
                 take it off this list",
                chain.chain_display_name()
            );
        }
    }
}
