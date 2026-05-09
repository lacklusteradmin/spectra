//! Unified staking service — the single UniFFI-exported object Swift holds.
//! Dispatches every call to the appropriate per-chain client so the Swift
//! layer never imports chain-specific Rust types directly.

use std::sync::Arc;

use crate::service::ChainEndpoints;
use crate::staking::{
    chains::{
        aptos::AptosStakingClient, cardano::CardanoStakingClient, icp::IcpStakingClient,
        near::NearStakingClient, polkadot::PolkadotStakingClient, solana::SolanaStakingClient,
        sui::SuiStakingClient,
    },
    StakingActionPreview, StakingError, StakingPosition, StakingValidator,
};

const CHAIN_SOLANA: &str = "solana";
const CHAIN_CARDANO: &str = "cardano";
const CHAIN_POLKADOT: &str = "polkadot";
const CHAIN_SUI: &str = "sui";
const CHAIN_APTOS: &str = "aptos";
const CHAIN_NEAR: &str = "near";
const CHAIN_ICP: &str = "internet-computer";

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

#[uniffi::export]
impl StakingService {
    #[uniffi::constructor]
    pub fn new(endpoints: Vec<ChainEndpoints>) -> Arc<Self> {
        let eps = |chain_id: &str| -> Vec<String> {
            endpoints
                .iter()
                .find(|e| e.chain_id == chain_id)
                .map(|e| e.endpoints.clone())
                .unwrap_or_default()
        };
        Arc::new(Self {
            solana: SolanaStakingClient::new(eps(CHAIN_SOLANA)),
            cardano: CardanoStakingClient::new(eps(CHAIN_CARDANO)),
            sui: SuiStakingClient::new(eps(CHAIN_SUI)),
            aptos: AptosStakingClient::new(eps(CHAIN_APTOS)),
            near: NearStakingClient::new(eps(CHAIN_NEAR)),
            polkadot: PolkadotStakingClient::new(eps(CHAIN_POLKADOT)),
            icp: IcpStakingClient::new(eps(CHAIN_ICP)),
        })
    }

    // ── Common ───────────────────────────────────────────────────────────────

    pub async fn fetch_validators(
        &self,
        chain_id: String,
    ) -> Result<Vec<StakingValidator>, StakingError> {
        match chain_id.as_str() {
            CHAIN_SOLANA => self.solana.fetch_validators().await,
            CHAIN_CARDANO => self.cardano.fetch_validators().await,
            CHAIN_SUI => self.sui.fetch_validators().await,
            CHAIN_APTOS => self.aptos.fetch_validators().await,
            CHAIN_NEAR => self.near.fetch_validators().await,
            CHAIN_POLKADOT => self.polkadot.fetch_validators().await,
            CHAIN_ICP => self.icp.fetch_validators().await,
            _ => Err(StakingError::InvalidValidator(format!(
                "unsupported chain {chain_id}"
            ))),
        }
    }

    pub async fn fetch_positions(
        &self,
        chain_id: String,
        wallet_address: String,
    ) -> Result<Vec<StakingPosition>, StakingError> {
        match chain_id.as_str() {
            CHAIN_SOLANA => self.solana.fetch_positions(&wallet_address).await,
            CHAIN_CARDANO => self.cardano.fetch_positions(&wallet_address).await,
            CHAIN_SUI => self.sui.fetch_positions(&wallet_address).await,
            CHAIN_APTOS => self.aptos.fetch_positions(&wallet_address).await,
            CHAIN_NEAR => self.near.fetch_positions(&wallet_address).await,
            CHAIN_POLKADOT => self.polkadot.fetch_positions(&wallet_address).await,
            CHAIN_ICP => self.icp.fetch_positions(&wallet_address).await,
            _ => Err(StakingError::InvalidValidator(format!(
                "unsupported chain {chain_id}"
            ))),
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

    pub async fn solana_build_stake_tx(
        &self,
        wallet_address: String,
        amount_lamports: u64,
        vote_account: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.solana
            .build_create_and_delegate_tx(&wallet_address, amount_lamports, &vote_account)
            .await
    }

    pub async fn solana_build_deactivate_tx(
        &self,
        wallet_address: String,
        stake_account: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.solana
            .build_deactivate_tx(&wallet_address, &stake_account)
            .await
    }

    pub async fn solana_build_withdraw_tx(
        &self,
        wallet_address: String,
        stake_account: String,
        amount_lamports: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.solana
            .build_withdraw_tx(&wallet_address, &stake_account, amount_lamports)
            .await
    }

    // ── Action previews: Cardano ─────────────────────────────────────────────

    pub async fn cardano_build_delegate_tx(
        &self,
        wallet_address: String,
        pool_id: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.cardano
            .build_register_and_delegate_tx(&wallet_address, &pool_id)
            .await
    }

    pub async fn cardano_build_claim_rewards_tx(
        &self,
        wallet_address: String,
        amount_lovelace: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.cardano
            .build_claim_rewards_tx(&wallet_address, amount_lovelace)
            .await
    }

    pub async fn cardano_build_deregister_tx(
        &self,
        wallet_address: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.cardano.build_deregister_tx(&wallet_address).await
    }

    // ── Action previews: Sui ─────────────────────────────────────────────────

    pub async fn sui_build_add_stake_tx(
        &self,
        wallet_address: String,
        amount_mist: u64,
        validator_address: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.sui
            .build_request_add_stake_tx(&wallet_address, amount_mist, &validator_address)
            .await
    }

    pub async fn sui_build_withdraw_stake_tx(
        &self,
        wallet_address: String,
        staked_sui_object_id: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.sui
            .build_request_withdraw_stake_tx(&wallet_address, &staked_sui_object_id)
            .await
    }

    // ── Action previews: Aptos ───────────────────────────────────────────────

    pub async fn aptos_build_add_stake_tx(
        &self,
        wallet_address: String,
        pool_address: String,
        amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.aptos
            .build_add_stake_tx(&wallet_address, &pool_address, amount_octas)
            .await
    }

    pub async fn aptos_build_unlock_tx(
        &self,
        wallet_address: String,
        pool_address: String,
        amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.aptos
            .build_unlock_tx(&wallet_address, &pool_address, amount_octas)
            .await
    }

    pub async fn aptos_build_withdraw_tx(
        &self,
        wallet_address: String,
        pool_address: String,
        amount_octas: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.aptos
            .build_withdraw_tx(&wallet_address, &pool_address, amount_octas)
            .await
    }

    // ── Action previews: NEAR ────────────────────────────────────────────────

    pub async fn near_build_deposit_and_stake_tx(
        &self,
        wallet_address: String,
        pool_account_id: String,
        amount_yocto_near: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.near
            .build_deposit_and_stake_tx(&wallet_address, &pool_account_id, &amount_yocto_near)
            .await
    }

    pub async fn near_build_unstake_tx(
        &self,
        wallet_address: String,
        pool_account_id: String,
        amount_yocto_near: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.near
            .build_unstake_tx(&wallet_address, &pool_account_id, &amount_yocto_near)
            .await
    }

    pub async fn near_build_withdraw_tx(
        &self,
        wallet_address: String,
        pool_account_id: String,
        amount_yocto_near: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.near
            .build_withdraw_tx(&wallet_address, &pool_account_id, &amount_yocto_near)
            .await
    }

    // ── Action previews: Polkadot ────────────────────────────────────────────

    pub async fn polkadot_build_bond_and_nominate_tx(
        &self,
        wallet_address: String,
        amount_planck: String,
        validator_addresses: Vec<String>,
    ) -> Result<StakingActionPreview, StakingError> {
        let planck = amount_planck
            .parse::<u128>()
            .map_err(|_| StakingError::AmountBelowMinimum(amount_planck))?;
        self.polkadot
            .build_bond_and_nominate_tx(&wallet_address, planck, &validator_addresses)
            .await
    }

    pub async fn polkadot_build_join_pool_tx(
        &self,
        wallet_address: String,
        amount_planck: String,
        pool_id: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        let planck = amount_planck
            .parse::<u128>()
            .map_err(|_| StakingError::AmountBelowMinimum(amount_planck))?;
        self.polkadot
            .build_join_pool_tx(&wallet_address, planck, pool_id)
            .await
    }

    pub async fn polkadot_build_unbond_tx(
        &self,
        wallet_address: String,
        amount_planck: String,
    ) -> Result<StakingActionPreview, StakingError> {
        let planck = amount_planck
            .parse::<u128>()
            .map_err(|_| StakingError::AmountBelowMinimum(amount_planck))?;
        self.polkadot
            .build_unbond_tx(&wallet_address, planck)
            .await
    }

    pub async fn polkadot_build_withdraw_unbonded_tx(
        &self,
        wallet_address: String,
    ) -> Result<StakingActionPreview, StakingError> {
        self.polkadot
            .build_withdraw_unbonded_tx(&wallet_address)
            .await
    }

    // ── Action previews: ICP ─────────────────────────────────────────────────

    pub async fn icp_build_create_neuron_tx(
        &self,
        wallet_address: String,
        amount_e8s: u64,
        dissolve_delay_months: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        self.icp
            .build_create_neuron_tx(&wallet_address, amount_e8s, dissolve_delay_months)
            .await
    }

    pub async fn icp_build_increase_dissolve_delay_tx(
        &self,
        wallet_address: String,
        neuron_id: u64,
        additional_months: u32,
    ) -> Result<StakingActionPreview, StakingError> {
        self.icp
            .build_increase_dissolve_delay_tx(&wallet_address, neuron_id, additional_months)
            .await
    }

    pub async fn icp_build_start_dissolving_tx(
        &self,
        wallet_address: String,
        neuron_id: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.icp
            .build_start_dissolving_tx(&wallet_address, neuron_id)
            .await
    }

    pub async fn icp_build_disburse_tx(
        &self,
        wallet_address: String,
        neuron_id: u64,
        amount_e8s: u64,
    ) -> Result<StakingActionPreview, StakingError> {
        self.icp
            .build_disburse_tx(&wallet_address, neuron_id, amount_e8s)
            .await
    }
}
