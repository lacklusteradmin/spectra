//! Unified staking service — the single UniFFI-exported object Swift holds.
//! Dispatches every call to the appropriate per-chain client so the Swift
//! layer never imports chain-specific Rust types directly.

use std::sync::Arc;

use crate::registry::Chain;
use crate::service::ChainEndpoints;
use crate::staking::{StakingAction, StakingActionRequest,
    chains::{
        aptos::AptosStakingClient, cardano::CardanoStakingClient, icp::IcpStakingClient,
        near::NearStakingClient, polkadot::PolkadotStakingClient, solana::SolanaStakingClient,
        sui::SuiStakingClient,
    },
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};


#[derive(uniffi::Object)]
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

// `async_runtime = "tokio"` is not optional on a block with `async fn`s: without
// it UniFFI polls the future with no reactor installed and every call fails with
// "there is no reactor running, must be called from the context of a Tokio 1.x
// runtime". This block was the only async-exporting one in the crate without it,
// so the staking tab could never load a validator or a position on iOS. The CLI
// could not show it — it drives `StakingService` from inside its own runtime via
// `ctx.rt.block_on` — and no Swift test reaches the network. Found by opening the
// app.
#[uniffi::export(async_runtime = "tokio")]
impl StakingService {
    #[uniffi::constructor]
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

    // ── Polkadot-specific ────────────────────────────────────────────────────

    pub async fn polkadot_fetch_nomination_pools(
        &self,
    ) -> Result<Vec<StakingValidator>, StakingError> {
        self.polkadot.fetch_nomination_pools().await
    }

    // ── Cardano-specific ─────────────────────────────────────────────────────

    pub async fn cardano_is_stake_address_registered(
        &self,
        stake_address: String,
    ) -> Result<bool, StakingError> {
        self.cardano
            .is_stake_address_registered(&stake_address)
            .await
    }

    // ── Action previews: Solana ──────────────────────────────────────────────

    /// Build the transaction for one staking action on one chain.
    ///
    /// Twenty-three exports stood here, one per (chain, action) pair, and the
    /// Swift bridge and view model each repeated the same twenty-three names.
    /// The pair is data; this matches on it.
    pub async fn build_staking_tx(
        &self,
        request: StakingActionRequest,
    ) -> Result<StakingActionPreview, StakingError> {
        use crate::registry::Chain;
        use StakingAction as A;

        let chain = Chain::from_str_id(&request.chain_id)
            .ok_or_else(|| request.malformed("chain_id", "no chain has that id"))?;
        let unsupported = || StakingError::UnsupportedAction {
            chain: chain.chain_display_name().to_string(),
            action: request.action.label().to_string(),
        };
        let who = request.wallet_address.as_str();

        match (chain.mainnet_counterpart(), request.action) {
            (Chain::Solana, A::Stake) => {
                self.solana
                    .build_create_and_delegate_tx(who, request.amount_u64()?, request.target()?)
                    .await
            }
            (Chain::Solana, A::Unstake) => {
                self.solana.build_deactivate_tx(who, request.target()?).await
            }
            (Chain::Solana, A::Withdraw) => {
                self.solana
                    .build_withdraw_tx(who, request.target()?, request.amount_u64()?)
                    .await
            }

            (Chain::Cardano, A::Stake) => {
                self.cardano
                    .build_register_and_delegate_tx(who, request.target()?)
                    .await
            }
            (Chain::Cardano, A::ClaimRewards) => {
                self.cardano
                    .build_claim_rewards_tx(who, request.amount_u64()?)
                    .await
            }
            (Chain::Cardano, A::Deregister) => self.cardano.build_deregister_tx(who).await,

            (Chain::Sui, A::Stake) => {
                self.sui
                    .build_request_add_stake_tx(who, request.amount_u64()?, request.target()?)
                    .await
            }
            (Chain::Sui, A::Withdraw) => {
                self.sui
                    .build_request_withdraw_stake_tx(who, request.target()?)
                    .await
            }

            (Chain::Aptos, A::Stake) => {
                self.aptos
                    .build_add_stake_tx(who, request.target()?, request.amount_u64()?)
                    .await
            }
            (Chain::Aptos, A::Unstake) => {
                self.aptos
                    .build_unlock_tx(who, request.target()?, request.amount_u64()?)
                    .await
            }
            (Chain::Aptos, A::Withdraw) => {
                self.aptos
                    .build_withdraw_tx(who, request.target()?, request.amount_u64()?)
                    .await
            }

            (Chain::Near, A::Stake) => {
                self.near
                    .build_deposit_and_stake_tx(who, request.target()?, &request.amount)
                    .await
            }
            (Chain::Near, A::Unstake) => {
                self.near
                    .build_unstake_tx(who, request.target()?, &request.amount)
                    .await
            }
            (Chain::Near, A::Withdraw) => {
                self.near
                    .build_withdraw_tx(who, request.target()?, &request.amount)
                    .await
            }

            (Chain::Polkadot, A::Stake) => {
                self.polkadot
                    .build_bond_and_nominate_tx(who, request.amount_u128()?, &request.targets)
                    .await
            }
            (Chain::Polkadot, A::JoinPool) => {
                self.polkadot
                    .build_join_pool_tx(who, request.amount_u128()?, request.target_u32()?)
                    .await
            }
            (Chain::Polkadot, A::Unstake) => {
                self.polkadot
                    .build_unbond_tx(who, request.amount_u128()?)
                    .await
            }
            (Chain::Polkadot, A::Withdraw) => {
                self.polkadot.build_withdraw_unbonded_tx(who).await
            }

            (Chain::Icp, A::Stake) => {
                self.icp
                    .build_create_neuron_tx(who, request.amount_u64()?, request.months()?)
                    .await
            }
            (Chain::Icp, A::ExtendLockup) => {
                self.icp
                    .build_increase_dissolve_delay_tx(who, request.target_u64()?, request.months()?)
                    .await
            }
            (Chain::Icp, A::Unstake) => {
                self.icp
                    .build_start_dissolving_tx(who, request.target_u64()?)
                    .await
            }
            (Chain::Icp, A::Withdraw) => {
                self.icp
                    .build_disburse_tx(who, request.target_u64()?, request.amount_u64()?)
                    .await
            }
            (Chain::Icp, A::ClaimRewards) => {
                self.icp
                    .build_claim_maturity_tx(who, request.target_u64()?)
                    .await
            }

            _ => Err(unsupported()),
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
        for chain in [Chain::Bitcoin, Chain::Ethereum, Chain::Tron, Chain::Xrp, Chain::Monero] {
            assert!(
                !chain.supports_staking(),
                "{} now stakes — that is a new client, so say so in PLAN.md and \
                 take it off this list",
                chain.chain_display_name()
            );
        }
    }
}


#[cfg(test)]
mod one_entry_point {
    use super::*;
    use crate::registry::Chain;

    fn request(chain: Chain, action: StakingAction) -> StakingActionRequest {
        StakingActionRequest {
            chain_id: chain.str_id().to_string(),
            action,
            wallet_address: "whoever".into(),
            amount: "1".into(),
            targets: vec!["1".into()],
            months: Some(6),
        }
    }

    /// Every chain the registry says can stake accepts a stake.
    ///
    /// Offline some build locally and some fail on the network; either way the
    /// pair was *recognised*, which is what this asserts.
    #[tokio::test]
    async fn every_staking_chain_accepts_a_stake() {
        let service = StakingService::new(Vec::new());
        for chain in Chain::all().filter(|c| c.supports_staking()) {
            if let Err(err) = service
                .build_staking_tx(request(chain, StakingAction::Stake))
                .await
            {
                assert!(
                    !matches!(err, StakingError::UnsupportedAction { .. }),
                    "{} cannot stake: {err}",
                    chain.chain_display_name()
                );
            }
        }
    }

    /// The whole matrix, stated once.
    ///
    /// Cardano is why this is a table rather than "every chain does every
    /// action": its model is delegate / claim / deregister, with no separate
    /// unstake or withdraw — deregistering is how you stop, and it reclaims the
    /// key deposit at the same time. Twenty-three exports said this by which
    /// names existed, which is not somewhere a reader can look.
    #[tokio::test]
    async fn the_supported_matrix_is_exactly_this() {
        use StakingAction as A;
        const MATRIX: &[(Chain, &[StakingAction])] = &[
            (Chain::Solana, &[A::Stake, A::Unstake, A::Withdraw]),
            (Chain::Cardano, &[A::Stake, A::ClaimRewards, A::Deregister]),
            (Chain::Sui, &[A::Stake, A::Withdraw]),
            (Chain::Aptos, &[A::Stake, A::Unstake, A::Withdraw]),
            (Chain::Near, &[A::Stake, A::Unstake, A::Withdraw]),
            (
                Chain::Polkadot,
                &[A::Stake, A::JoinPool, A::Unstake, A::Withdraw],
            ),
            (
                Chain::Icp,
                &[
                    A::Stake,
                    A::Unstake,
                    A::Withdraw,
                    A::ClaimRewards,
                    A::ExtendLockup,
                ],
            ),
        ];
        const EVERY_ACTION: &[StakingAction] = &[
            A::Stake,
            A::JoinPool,
            A::Unstake,
            A::Withdraw,
            A::ClaimRewards,
            A::Deregister,
            A::ExtendLockup,
        ];

        let service = StakingService::new(Vec::new());
        for (chain, supported) in MATRIX {
            for action in EVERY_ACTION {
                let refused = matches!(
                    service.build_staking_tx(request(*chain, *action)).await,
                    Err(StakingError::UnsupportedAction { .. })
                );
                assert_eq!(
                    refused,
                    !supported.contains(action),
                    "{} / {}: supported says {}, the dispatch says {}",
                    chain.chain_display_name(),
                    action.label(),
                    supported.contains(action),
                    !refused
                );
            }
        }

        // And the table covers every chain the registry says stakes.
        let tabled: Vec<Chain> = MATRIX.iter().map(|(c, _)| *c).collect();
        for chain in Chain::all().filter(|c| c.supports_staking() && !c.is_testnet()) {
            assert!(
                tabled.contains(&chain),
                "{} stakes and is not in the matrix",
                chain.chain_display_name()
            );
        }
    }

    /// A pair no chain has says so, rather than being a name that does not
    /// exist.
    #[tokio::test]
    async fn an_action_a_chain_does_not_have_is_refused_by_name() {
        let service = StakingService::new(Vec::new());
        let err = service
            .build_staking_tx(request(Chain::Solana, StakingAction::JoinPool))
            .await
            .expect_err("Solana has no nomination pools");
        match err {
            StakingError::UnsupportedAction { chain, action } => {
                assert_eq!(chain, "Solana");
                assert_eq!(action, "join pool");
            }
            other => panic!("expected UnsupportedAction, got {other}"),
        }
    }

    /// A chain id nothing knows is a malformed request, not a silent no-op.
    #[tokio::test]
    async fn an_unknown_chain_is_a_malformed_request() {
        let service = StakingService::new(Vec::new());
        let mut req = request(Chain::Solana, StakingAction::Stake);
        req.chain_id = "not-a-chain".into();
        assert!(matches!(
            service.build_staking_tx(req).await,
            Err(StakingError::MalformedRequest { .. })
        ));
    }
}
