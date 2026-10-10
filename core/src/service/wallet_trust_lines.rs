//! An XRP Ledger or Stellar wallet's trust lines, and changing them.
//!
//! Neither network lets an account hold an issued asset it has not trusted:
//! a trust line (a trustline on Stellar) names the asset and the most of it
//! the account accepts, and locks a reserve of the native coin while it
//! exists. Core lists the wallet's lines, opens one so the wallet can
//! receive an asset, and removes an empty one to free its reserve, each a
//! transaction built, signed and broadcast through the ordinary send stages
//! under [`WalletOperation::TrustAsset`] or
//! [`WalletOperation::RemoveTrustLine`].

use super::*;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};

/// One trust line of a wallet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletTrustLine {
    /// `CODE.rIssuer` on the XRP Ledger, `CODE:ISSUER` on Stellar.
    pub asset: String,
    /// The asset's code as a person reads it.
    pub code: String,
    pub issuer: String,
    /// What the wallet holds, as an exact decimal.
    pub balance: String,
    /// The most the line accepts, as an exact decimal.
    pub limit: String,
    /// Whether the issuer lets the wallet hold the asset.
    pub authorized: bool,
    /// Whether the issuer has frozen the line.
    pub frozen: bool,
    /// Why the line cannot be removed now; `None` when it can.
    pub removal_blocked: Option<String>,
}

/// A wallet's trust lines, and the reserve each one locks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletTrustLines {
    pub lines: Vec<WalletTrustLine>,
    /// The reserve one line locks, as an exact decimal of the native coin.
    pub reserve_per_line: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The wallet's trust lines from a verified node, each saying whether it
    /// can be removed.
    pub async fn wallet_trust_lines(
        &self,
        wallet_id: String,
    ) -> Result<WalletTrustLines, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (chain, owner) = this.trust_line_owner(&wallet_id).await?;
            let endpoints = this
                .endpoints_for(chain, &[EndpointCapability::Verification])
                .await;
            let decimals = u32::from(chain.native_decimals());
            if chain.mainnet_counterpart() == Chain::Xrp {
                let client = XrplClient::new(endpoints);
                let reserve = client.fetch_reserve_state(chain, &owner).await?;
                let lines = client
                    .fetch_trust_lines(&owner, None)
                    .await?
                    .into_iter()
                    .filter(|line| !line.limit.is_zero() || !line.balance.is_zero())
                    .map(|line| WalletTrustLine {
                        asset: line.issue.identifier(),
                        code: line.issue.currency.display(),
                        issuer: line.issue.issuer.clone(),
                        balance: line.balance.to_decimal(),
                        limit: line.limit.to_decimal(),
                        authorized: line.peer_authorized,
                        frozen: line.frozen || line.deep_frozen,
                        removal_blocked: if !line.balance.is_zero() {
                            Some("The trust line still holds the token".into())
                        } else if !line.limit_peer.is_zero() {
                            Some(
                                "The issuer extends its own trust on this line, so the line stays"
                                    .into(),
                            )
                        } else {
                            None
                        },
                    })
                    .collect();
                return Ok(WalletTrustLines {
                    lines,
                    reserve_per_line: crate::decimal::from_units(
                        u128::from(reserve.reserve_increment),
                        decimals,
                    ),
                });
            }
            let client = HorizonClient::new(endpoints);
            client.verify_network(chain).await?;
            let base_reserve = client.fetch_base_reserve().await?;
            let lines = client
                .fetch_account_state(&owner)
                .await?
                .map(|state| state.trustlines)
                .unwrap_or_default()
                .into_iter()
                .map(|line| WalletTrustLine {
                    asset: line.asset.identifier(),
                    code: line.asset.code.clone(),
                    issuer: line.asset.issuer.clone(),
                    balance: crate::decimal::from_units(u128::from(line.balance.unsigned_abs()), 7),
                    limit: crate::decimal::from_units(u128::from(line.limit.unsigned_abs()), 7),
                    authorized: line.authorized,
                    frozen: false,
                    removal_blocked: if line.balance != 0 {
                        Some("The trust line still holds the token".into())
                    } else if line.buying_liabilities != 0 {
                        Some("Open offers still use this trustline".into())
                    } else {
                        None
                    },
                })
                .collect();
            Ok(WalletTrustLines {
                lines,
                reserve_per_line: crate::decimal::from_units(u128::from(base_reserve), decimals),
            })
        })
        .await
    }

    /// Build the transaction that opens a trust line to `asset`, so the
    /// wallet can hold it. Stored like any send, to be signed and broadcast
    /// through the same stages.
    pub async fn build_trust_asset(
        &self,
        wallet_id: String,
        asset: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(
            async move { this.build_trust_line_change(wallet_id, asset, false).await },
        )
        .await
    }

    /// Build the transaction that removes the wallet's empty trust line to
    /// `asset`, freeing its reserve.
    pub async fn build_remove_trust_line(
        &self,
        wallet_id: String,
        asset: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(
            async move { this.build_trust_line_change(wallet_id, asset, true).await },
        )
        .await
    }
}

impl WalletService {
    /// An XRP Ledger or Stellar wallet's network and address, or a refusal.
    async fn trust_line_owner(
        &self,
        wallet_id: &str,
    ) -> Result<(Chain, String), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        if !matches!(chain.mainnet_counterpart(), Chain::Xrp | Chain::Stellar) {
            return Err(SpectraBridgeError::invalid(
                "Only an XRP Ledger or Stellar wallet keeps trust lines.",
            ));
        }
        let owner = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
            .to_string();
        Ok((chain, owner))
    }

    async fn build_trust_line_change(
        &self,
        wallet_id: String,
        asset: String,
        remove: bool,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let (chain, owner) = self.trust_line_owner(&wallet_id).await?;
        if self.stored_wallet(&wallet_id).await?.is_watch_only() {
            return Err(SpectraBridgeError::invalid(
                "a watch-only wallet cannot send",
            ));
        }
        let standard = chain.token_standard_for_identifier(&asset);
        let asset = crate::tokens::validate_protocol_identifier(chain, standard, &asset)?;
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let fees = self.endpoints_for(chain, &[EndpointCapability::Fee]).await;
        let (prepared, fee, reserve) = if chain.mainnet_counterpart() == Chain::Xrp {
            let fee = XrplClient::new(fees).fetch_fee().await?;
            let (plan, reserve) = crate::send::xrp_issued::PreparedXrpTrustSet::plan(
                &XrplClient::new(endpoints),
                chain,
                &owner,
                &asset,
                remove,
                fee,
            )
            .await?;
            (PreparedPayload::XrpTrustSet(plan), fee, reserve)
        } else {
            let fee = HorizonClient::new(fees).fetch_base_fee().await?;
            let (plan, reserve) = crate::send::stellar_issued::PreparedStellarChangeTrust::plan(
                &HorizonClient::new(endpoints),
                chain,
                &owner,
                &asset,
                remove,
                fee,
            )
            .await?;
            (PreparedPayload::StellarChangeTrust(plan), fee, reserve)
        };
        let decimals = u32::from(chain.native_decimals());
        let network_fee = crate::decimal::from_units(u128::from(fee), decimals);
        let reserve = crate::decimal::from_units(u128::from(reserve), decimals);
        let operation = if remove {
            WalletOperation::RemoveTrustLine {
                asset: asset.clone(),
                reserve,
                network_fee: network_fee.clone(),
            }
        } else {
            WalletOperation::TrustAsset {
                asset: asset.clone(),
                reserve,
                network_fee: network_fee.clone(),
            }
        };
        let request = crate::send::SendExecutionRequest {
            chain_id: chain,
            wallet_id: wallet_id.clone(),
            password: None,
            to_address: owner.clone(),
            amount_str: "0".into(),
            contract_address: None,
            token_standard: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: Some(network_fee),
            evm_overrides: None,
            sign_only: false,
            memo: None,
        };
        let state = self.app_state().await;
        let mut stored = StoredSend {
            view: SendArtifact {
                id: crate::store::new_transaction_id(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id,
                chain_id: chain,
                sender: owner.clone(),
                recipient: owner,
                amount: "0".into(),
                symbol: super::send_records::send_asset_names(&state, chain, Some(&asset)).0,
                asset,
                staking: None,
                operation: Some(operation),
                created_at: crate::store::now_unix().floor(),
                review_digest: String::new(),
                review: SendArtifactReview::default(),
                prepared_details: serde_json::to_string_pretty(&prepared)?,
                signing_payload_hex: String::new(),
                signed_payload: None,
                transaction_hash: None,
                attempts: Vec::new(),
                selected_endpoints: Vec::new(),
                memo: None,
            },
            request,
            prepared,
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: vec![],
        };
        stored.view.review_digest = stored.digest()?;
        self.save_send_artifact(&stored, Vec::new()).await?;
        Ok(stored.view)
    }
}

#[cfg(test)]
#[path = "tests/issued_assets.rs"]
mod issued_assets_tests;
