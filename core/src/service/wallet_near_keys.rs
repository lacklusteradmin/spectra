//! A NEAR account's access keys, and deleting a function-call key.
//!
//! A NEAR account signs with any of its keys: full-access keys can do
//! anything, function-call keys only call one contract and spend gas from an
//! allowance. Dapps add function-call keys at sign-in and rarely remove them.
//! Spectra lists every key, marks the one it signs with, and deletes a
//! function-call key through the ordinary send stages under its own
//! [`WalletOperation::DeleteAccessKey`]. Full-access keys are only listed: a
//! wallet removing a key that may be its owner's only way in is not a page
//! action.

use super::*;
use crate::send::stages::{
    PreparedPayload, SendArtifact, SendArtifactReview, SendStage, StoredSend, WalletOperation,
};

/// One key on the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NearAccessKey {
    /// `ed25519:…`.
    pub public_key: String,
    pub full_access: bool,
    /// The contract a function-call key may call; `None` for full access.
    pub receiver: Option<String>,
    /// The methods it may call; empty for any method of the contract.
    pub method_names: Vec<String>,
    /// The NEAR it may still spend on gas, as an exact decimal; `None` for
    /// full access or an unlimited allowance.
    pub allowance: Option<String>,
    /// Whether this is the key Spectra signs with.
    pub signs: bool,
}

/// The access keys on a NEAR wallet's account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct NearAccessKeys {
    pub account: String,
    /// The key Spectra signs with first, then full-access keys, then
    /// function-call keys by contract.
    pub keys: Vec<NearAccessKey>,
}

/// The key a NEAR wallet signs with: an implicit account's address, or the
/// key a named account recorded at import.
pub(super) fn near_signing_key(
    wallet: &crate::store::state::WalletState,
    account: &str,
) -> Result<[u8; 32], SpectraBridgeError> {
    if account.len() == 64 && account.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return hex::decode(account)?
            .try_into()
            .map_err(|_| SpectraBridgeError::failure("Invalid NEAR public key"));
    }
    let key = wallet.near_account_key.as_deref().ok_or_else(|| {
        SpectraBridgeError::failure("Named NEAR wallet has no recorded signing key")
    })?;
    Ok(crate::derivation::solana::decode_b58_32(
        key.strip_prefix("ed25519:")
            .ok_or_else(|| SpectraBridgeError::failure("Unsupported NEAR key"))?,
    )?)
}

const NOT_NEAR: &str = "Only a NEAR account has access keys.";

fn ed25519_text(key: &[u8; 32]) -> String {
    format!("ed25519:{}", bs58::encode(key).into_string())
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The access keys on the wallet's NEAR account, read from a verified
    /// node, with the one Spectra signs with marked.
    pub async fn wallet_access_keys(
        &self,
        wallet_id: String,
    ) -> Result<NearAccessKeys, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, chain, account) = this.near_account(&wallet_id, NOT_NEAR).await?;
            let signing = near_signing_key(&wallet, &account)
                .ok()
                .map(|key| ed25519_text(&key));
            let decimals = u32::from(chain.native_decimals());
            let mut keys: Vec<NearAccessKey> = NearClient::new(
                this.endpoints_for(chain, &[EndpointCapability::Verification])
                    .await,
            )
            .fetch_access_keys(chain, &account)
            .await?
            .into_iter()
            .map(|entry| {
                let signs = signing.as_deref() == Some(entry.public_key.as_str());
                match entry.function_call {
                    None => NearAccessKey {
                        public_key: entry.public_key,
                        full_access: true,
                        receiver: None,
                        method_names: Vec::new(),
                        allowance: None,
                        signs,
                    },
                    Some(call) => NearAccessKey {
                        public_key: entry.public_key,
                        full_access: false,
                        receiver: Some(call.receiver_id),
                        method_names: call.method_names,
                        allowance: call
                            .allowance
                            .map(|allowance| crate::decimal::from_units(allowance, decimals)),
                        signs,
                    },
                }
            })
            .collect();
            keys.sort_by(|a, b| {
                (!a.signs, !a.full_access, &a.receiver, &a.public_key).cmp(&(
                    !b.signs,
                    !b.full_access,
                    &b.receiver,
                    &b.public_key,
                ))
            });
            Ok(NearAccessKeys { account, keys })
        })
        .await
    }

    /// Build the transaction that deletes one of the account's function-call
    /// keys, prepared and stored like any send, to be signed and broadcast
    /// through the same stages. Full-access keys, the key Spectra signs with
    /// and keys the account does not have are refused.
    pub async fn build_access_key_deletion(
        &self,
        wallet_id: String,
        public_key: String,
    ) -> Result<SendArtifact, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, chain, account) = this.near_account(&wallet_id, NOT_NEAR).await?;
            if wallet.is_watch_only() {
                return Err(SpectraBridgeError::invalid(
                    "a watch-only wallet cannot send",
                ));
            }
            let signer = near_signing_key(&wallet, &account)?;
            let public_key = public_key.trim().to_string();
            let client = NearClient::new(
                this.endpoints_for(chain, &[EndpointCapability::Verification])
                    .await,
            );
            let entry = client
                .fetch_access_keys(chain, &account)
                .await?
                .into_iter()
                .find(|entry| entry.public_key == public_key)
                .ok_or_else(|| {
                    SpectraBridgeError::refused("%@ is not a key of this account.", [&public_key])
                })?;
            let Some(call) = entry.function_call else {
                return Err(SpectraBridgeError::invalid(
                    "Spectra deletes only function-call keys; full-access keys are listed only.",
                ));
            };
            let deleted = public_key
                .strip_prefix("ed25519:")
                .map(crate::derivation::solana::decode_b58_32)
                .transpose()?
                .ok_or_else(|| SpectraBridgeError::invalid("Spectra deletes only Ed25519 keys."))?;
            if deleted == signer {
                return Err(SpectraBridgeError::invalid(
                    "This is the key Spectra signs with; it cannot delete it.",
                ));
            }
            let fee = client.delete_key_fee_budget().await?;
            if client.fetch_spendable_balance(&account).await? < fee {
                return Err(SpectraBridgeError::invalid(
                    "Insufficient spendable NEAR for the network fee",
                ));
            }
            let nonce = client
                .fetch_full_access_key_nonce(&account, &bs58::encode(signer).into_string())
                .await?
                .checked_add(1)
                .ok_or_else(|| SpectraBridgeError::failure("Nonce exhausted"))?;
            let block_hash =
                crate::derivation::solana::decode_b58_32(&client.fetch_latest_block_hash().await?)?;
            let prepared =
                PreparedPayload::NearDeleteKey(crate::send::near::PreparedNearDeleteKey::prepare(
                    &account, signer, nonce, deleted, block_hash, fee,
                )?);
            let network_fee = crate::decimal::from_units(fee, u32::from(chain.native_decimals()));
            let request = crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id: wallet_id.clone(),
                password: None,
                to_address: account.clone(),
                amount_str: "0".into(),
                contract_address: None,
                token_standard: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: Some(network_fee.clone()),
                evm_overrides: None,
                sign_only: false,
                memo: None,
            };
            let signing_payload_hex = match &prepared {
                PreparedPayload::NearDeleteKey(p) => hex::encode(&p.message),
                _ => unreachable!("the deletion was prepared as a NEAR transaction"),
            };
            let mut stored = StoredSend {
                view: SendArtifact {
                    id: crate::store::new_transaction_id(),
                    revision: 0,
                    stage: SendStage::Prepared,
                    wallet_id,
                    chain_id: chain,
                    sender: account.clone(),
                    recipient: account,
                    amount: "0".into(),
                    asset: chain.coin_symbol().into(),
                    symbol: chain.coin_symbol().into(),
                    staking: None,
                    operation: Some(WalletOperation::DeleteAccessKey {
                        public_key,
                        receiver: call.receiver_id,
                        network_fee,
                    }),
                    created_at: crate::store::now_unix().floor(),
                    review_digest: String::new(),
                    review: SendArtifactReview::default(),
                    prepared_details: serde_json::to_string_pretty(&prepared)?,
                    signing_payload_hex,
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
            this.save_send_artifact(&stored, Vec::new()).await?;
            Ok(stored.view)
        })
        .await
    }
}

impl WalletService {
    /// A NEAR wallet, its network and its account, or `refusal` for a wallet
    /// on another network.
    pub(super) async fn near_account(
        &self,
        wallet_id: &str,
        refusal: &'static str,
    ) -> Result<(crate::store::state::WalletState, Chain, String), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let chain = wallet.chain_id;
        if chain.mainnet_counterpart() != Chain::Near {
            return Err(SpectraBridgeError::invalid(refusal));
        }
        let account = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
            .to_string();
        Ok((wallet, chain, account))
    }
}
