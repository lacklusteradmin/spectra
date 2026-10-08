//! Durable, secret-free transaction artifacts. Protocol fields stay typed.

use crate::send::error::SendError;
use serde::{Deserialize, Serialize};

/// Wallet-owned account UTXO source, kept with each reviewed input so signing
/// derives the exact key and checks ownership again after a restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct UtxoSendSource {
    pub address: String,
    pub derivation_path: Option<String>,
    pub script_pubkey: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UtxoPreparedInput {
    pub source: UtxoSendSource,
    pub utxo: (String, u32, u64, Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PreparedAccountUtxoTransaction {
    pub inputs: Vec<UtxoPreparedInput>,
    pub amount: u64,
    pub fee: u64,
    pub recipient_script: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum PreparedPayload {
    Evm(super::evm::PreparedEvmTransaction),
    /// A transparent transfer from a wallet account's addresses, on every
    /// account UTXO network without a protocol stage of its own.
    AccountTransfer(super::account_utxo::PreparedAccountTransfer),
    Icp(super::icp_stages::PreparedIcpTransaction),
    IcpStaking(super::icp_staking::PreparedIcpStaking),
    Monero(super::monero_local::PreparedMoneroTransaction),
    Litecoin(PreparedAccountUtxoTransaction),
    Peercoin(PreparedAccountUtxoTransaction),
    Near {
        public_key: [u8; 32],
        nonce: u64,
        block_hash: [u8; 32],
        amount: u128,
        token_contract: Option<String>,
        fee_budget: String,
        /// The yoctoNEAR a token send deposits to register its recipient
        /// with the token first; `None` when the token has registered it.
        registration_deposit: Option<String>,
    },
    NearFunctionCall(super::near::PreparedNearFunctionCall),
    NearDeleteKey(super::near::PreparedNearDeleteKey),
    Ton {
        seqno: u32,
        #[serde(with = "units_u128")]
        amount: u128,
        valid_until: u32,
        jetton: Option<super::ton::PreparedJettonTransfer>,
    },
    Xrp {
        sequence: u32,
        fee_drops: u64,
        amount_drops: u64,
    },
    Stellar {
        sequence: u64,
        fee_stroops: u64,
        amount_stroops: i64,
    },
    /// Pays an XRP Ledger issued currency to the artifact's recipient.
    XrpIssuedPayment(super::xrp_issued::PreparedXrpIssuedPayment),
    /// Pays a Stellar credit asset to the artifact's recipient.
    StellarAssetPayment(super::stellar_issued::PreparedStellarAssetPayment),
    /// Opens or removes the sender's XRP Ledger trust line.
    XrpTrustSet(super::xrp_issued::PreparedXrpTrustSet),
    /// Opens or removes the sender's Stellar trustline.
    StellarChangeTrust(super::stellar_issued::PreparedStellarChangeTrust),
    /// Deletes the sender's XRP account into the artifact's recipient.
    XrpAccountDelete {
        sequence: u32,
        fee_drops: u64,
    },
    /// Merges the sender's Stellar account into the artifact's recipient.
    StellarAccountMerge {
        sequence: u64,
        fee_stroops: u64,
    },
    Substrate(super::polkadot::PreparedPolkadotTransaction),
    Cardano(super::cardano::PreparedCardanoTransaction),
    Solana(super::solana::PreparedSolanaTransaction),
    SolanaAccountClosure(super::solana::PreparedSolanaAccountClosure),
    Tron(super::tron::PreparedTronTransfer),
    Aptos(super::aptos::PreparedAptosTransfer),
    Sui(super::sui::PreparedSuiTransfer),
    SuiMerge(super::sui::PreparedSuiMerge),
    /// A Zcash transaction that touches the shielded pools, as librustzcash
    /// proposed it.
    ZcashShielded(super::zcash_shielded::PreparedZcashShielded),
    /// A payment out of a Litecoin wallet's MWEB funds, to an MWEB address
    /// or by a peg-out.
    LitecoinMweb(super::litecoin_mweb::prepared::PreparedMwebSpend),
    /// A Litecoin payment from transparent funds to an MWEB address.
    LitecoinPegIn(super::litecoin_mweb::prepared::PreparedPegIn),
}

/// Protocol amounts larger than JSON's integer range stay exact in artifacts
/// and every intermediate JSON reader. There are no legacy numeric variants.
mod units_u128 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum, PartialEq, Eq)]
pub enum SendStage {
    Prepared,
    Signed,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Enum, PartialEq, Eq)]
pub enum SubmissionOutcome {
    Accepted,
    Rejected,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct BroadcastAttempt {
    pub endpoint: String,
    pub attempted_at: f64,
    pub outcome: SubmissionOutcome,
    pub transaction_hash: Option<String>,
    pub detail: String,
}

/// Build-time advisories, bound to the immutable transaction and retained on resume.
#[derive(Debug, Clone, Default, Serialize, Deserialize, uniffi::Record)]
pub struct SendArtifactReview {
    pub warnings: Vec<crate::send::flow::HighRiskSendWarning>,
    pub recipient_warnings: Vec<crate::store::EvmRecipientPreflightWarning>,
    pub requires_self_send_confirmation: bool,
    pub staking: Option<crate::staking::StakingReview>,
    /// What the asset's own rules do to the transfer, when they do more than
    /// move the amount.
    pub transfer_terms: Option<AssetTransferTerms>,
}

/// What an asset's rules do to a transfer beyond moving the amount: a fee
/// its token program or issuer takes on the way, and a program it runs that
/// can refuse the transfer. Read from the prepared transaction, so the review
/// digest binds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct AssetTransferTerms {
    /// The most that leaves the sender, as an exact decimal of the asset.
    pub debited: String,
    /// What arrives at the recipient.
    pub received: String,
    /// What the asset's rules take between the two.
    pub fee: String,
    /// A program the asset runs on every transfer: a Solana transfer hook.
    pub hook_program: Option<String>,
    /// The network's own coin that travels with the asset and stays with
    /// the recipient, as an exact decimal: the minimum ADA a Cardano output
    /// holding the asset needs.
    pub carried_native: Option<String>,
    /// The network's own coin the sender deposits with the token to register
    /// a recipient it does not yet hold, as an exact decimal: a NEP-141
    /// token's NEP-145 storage deposit.
    #[uniffi(default = None)]
    pub recipient_registration: Option<String>,
}

impl PreparedPayload {
    /// The asset's transfer terms, read from what will be signed;
    /// `token_decimals` are the sent token's.
    pub(crate) fn transfer_terms(&self, token_decimals: Option<u32>) -> Option<AssetTransferTerms> {
        match self {
            Self::Solana(prepared) => prepared.token.as_ref()?.terms(),
            Self::XrpIssuedPayment(prepared) => prepared.terms(),
            Self::Cardano(prepared) => prepared.terms(token_decimals?),
            Self::Near {
                amount,
                token_contract: Some(_),
                registration_deposit: Some(deposit),
                ..
            } => {
                let sent = crate::decimal::from_units(*amount, token_decimals?);
                Some(AssetTransferTerms {
                    debited: sent.clone(),
                    received: sent,
                    fee: "0".into(),
                    hook_program: None,
                    carried_native: None,
                    recipient_registration: Some(crate::decimal::from_units(
                        deposit.parse().ok()?,
                        u32::from(crate::registry::Chain::Near.native_decimals()),
                    )),
                })
            }
            _ => None,
        }
    }
}

/// What a transaction built from a wallet's page does when it is not a
/// transfer. Each is bound into the review digest, and `StoredSend::validate`
/// checks the prepared payload is exactly that operation and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WalletOperation {
    /// Sets an ERC-20 allowance back to zero: `recipient` and `asset` are
    /// the token, and the prepared call is `approve(spender, 0)` on it.
    RevokeApproval {
        token: String,
        spender: String,
        /// The prepared call's fee ceiling, as an exact decimal of the
        /// native coin.
        network_fee: String,
    },
    /// Closes the account: it is removed and everything it holds, less the
    /// fee, goes to `destination`, the artifact's recipient; `amount` is
    /// what it held at the build, less the fee.
    CloseAccount {
        destination: String,
        /// The reserve the account held, which goes to the destination with
        /// the rest, as an exact decimal of the native coin.
        reserve: String,
        /// Objects the network deletes with the account: offers, tickets, a
        /// signer list. Always none on Stellar, which refuses to merge them.
        removed_objects: u64,
        network_fee: String,
    },
    /// Closes empty token accounts into their owner, returning their rent:
    /// the owner is the recipient and `amount` the rent.
    CloseTokenAccounts {
        /// The closed accounts.
        accounts: Vec<String>,
        /// The rent they return, as an exact decimal of SOL.
        rent: String,
        network_fee: String,
    },
    /// Merges a Sui coin type's objects into one: the account is the
    /// recipient and nothing leaves it.
    MergeCoins {
        coin_type: String,
        /// How many objects become one.
        objects: u64,
        /// The gas budget, the most the merge costs, as an exact decimal of
        /// SUI; a merge often refunds more storage than it spends.
        network_fee: String,
    },
    /// Opens a trust line (XRP Ledger) or trustline (Stellar) so the account
    /// can hold an issued asset: the account is the recipient and nothing
    /// leaves it but the fee.
    TrustAsset {
        /// `CODE.rIssuer` or `CODE:ISSUER`.
        asset: String,
        /// The reserve the line locks, as an exact decimal of the native coin.
        reserve: String,
        network_fee: String,
    },
    /// Removes an empty trust line, freeing its reserve.
    RemoveTrustLine {
        asset: String,
        /// The reserve the line held, as an exact decimal of the native coin.
        reserve: String,
        network_fee: String,
    },
    /// Sends an NFT: the artifact's recipient receives it and its `amount` is
    /// the quantity, a whole number; the prepared call is the collection's
    /// `safeTransferFrom` from the wallet, with no value.
    TransferNft {
        /// The collection's contract, which the transaction calls.
        contract: String,
        standard: crate::api::evm_nft::NftStandard,
        /// The token's id, a whole number in decimal.
        token_id: String,
        /// How many: one for an ERC-721 token.
        quantity: String,
        /// The collection's name as its contract gives it, or empty.
        collection: String,
        network_fee: String,
    },
    /// Moves a Zcash wallet's transparent funds into its own shielded pool:
    /// the recipient is the wallet's unified address and `amount` what
    /// arrives there.
    ShieldTransparent {
        /// What arrives in the shielded pool, as an exact decimal of ZEC.
        amount: String,
        network_fee: String,
    },
    /// Pays the recipient the amount from a Zcash wallet's shielded funds.
    ShieldedPayment {
        /// The text memo a shielded recipient reads, exactly as it is signed.
        memo: Option<String>,
        network_fee: String,
    },
    /// Deletes one of a NEAR account's function-call keys: the account is
    /// the recipient, and the dapp the key could call loses that access.
    DeleteAccessKey {
        /// The deleted key, as `ed25519:…`.
        public_key: String,
        /// The contract the key could call.
        receiver: String,
        network_fee: String,
    },
    /// Unregisters a NEAR account from a token contract it holds none of
    /// (NEP-145 `storage_unregister`, never with `force`), returning the
    /// storage deposit the contract held: the account is the recipient and
    /// `amount` the deposit.
    RefundTokenStorage {
        /// The token contract, which the transaction calls.
        contract: String,
        /// The deposit returned, as an exact decimal of NEAR.
        refund: String,
        network_fee: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct SendArtifact {
    pub id: String,
    pub revision: u64,
    pub stage: SendStage,
    pub wallet_id: String,
    pub chain_id: crate::registry::Chain,
    pub sender: String,
    pub recipient: String,
    /// The XRP destination tag or Stellar memo the transaction carries to
    /// the recipient: the request's, reviewed with it.
    pub memo: Option<super::payment_memo::PaymentMemo>,
    pub amount: String,
    /// Native symbol or exact token contract/mint identity.
    pub asset: String,
    /// What the asset is called on screen: the coin's symbol, the known
    /// token's symbol, or the contract when no known token claims it.
    pub symbol: String,
    pub staking: Option<crate::staking::StakingRequest>,
    /// What the transaction does when it is a wallet operation rather than
    /// a transfer.
    pub operation: Option<WalletOperation>,
    pub created_at: f64,
    /// Fingerprint of the complete immutable prepared content, confirmed by Sign.
    pub review_digest: String,
    pub review: SendArtifactReview,
    pub prepared_details: String,
    pub signing_payload_hex: String,
    pub signed_payload: Option<String>,
    pub transaction_hash: Option<String>,
    pub attempts: Vec<BroadcastAttempt>,
    pub selected_endpoints: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct StoredSend {
    pub view: SendArtifact,
    pub request: super::SendExecutionRequest,
    pub prepared: PreparedPayload,
    pub submission: Option<super::payload::PreparedSubmission>,
    pub signed_digest: Option<String>,
    pub substrate_verified_through: Option<u64>,
    pub icp_staking_receipts: Vec<super::icp_staking::IcpStakingReceipt>,
}

impl StoredSend {
    pub fn digest(&self) -> Result<String, SendError> {
        use sha2::Digest;
        let bytes = serde_json::to_vec(&(
            &self.request,
            &self.prepared,
            &self.view.sender,
            &self.view.id,
            self.view.created_at,
            &self.view.signing_payload_hex,
            &self.view.asset,
            &self.view.symbol,
            &self.view.staking,
            &self.view.operation,
            &self.view.review,
        ))?;
        Ok(hex::encode(sha2::Sha256::digest(bytes)))
    }
    pub fn submission_digest(&self) -> Result<Option<String>, SendError> {
        use sha2::Digest;
        self.submission
            .as_ref()
            .map(|s| serde_json::to_vec(s).map(|bytes| hex::encode(sha2::Sha256::digest(bytes))))
            .transpose()
            .map_err(SendError::invalid)
    }
    pub fn validate(&self) -> Result<(), SendError> {
        let mut receipt_ids = std::collections::HashSet::new();
        for receipt in &self.icp_staking_receipts {
            if !receipt_ids.insert(&receipt.request_id) {
                return Err(SendError::invalid("Duplicate ICP execution proof"));
            }
            receipt.validate()?;
            let PreparedPayload::IcpStaking(prepared) = &self.prepared else {
                return Err(SendError::invalid(
                    "ICP execution proof attached to another protocol",
                ));
            };
            let active: Vec<super::icp_staking::SignedIcpStakingCall> = self
                .submission
                .as_ref()
                .map(|s| serde_json::from_str(&s.payload))
                .transpose()?
                .unwrap_or_default();
            let controller = hex::decode(&prepared.controller_hex)?;
            for call in active
                .iter()
                .chain(&prepared.completed_calls)
                .chain(&prepared.prior_calls)
            {
                call.validated_argument(&controller)?;
            }
            if !active
                .iter()
                .chain(&prepared.completed_calls)
                .chain(&prepared.prior_calls)
                .any(|call| receipt.matches(call))
            {
                return Err(SendError::invalid(
                    "ICP execution proof is not bound to this artifact",
                ));
            }
        }
        if self.request.password.is_some() || self.digest()? != self.view.review_digest {
            return Err(SendError::Invalid(
                "Prepared transaction was altered; build and review again".into(),
            ));
        }
        if self.request.wallet_id != self.view.wallet_id
            || self.request.chain_id != self.view.chain_id
            || self.request.to_address != self.view.recipient
            || self.request.memo != self.view.memo
            || self.request.amount_str != self.view.amount
        {
            return Err(SendError::Invalid(
                "Transaction identity was altered".into(),
            ));
        }
        if self.view.staking.as_ref().is_some_and(|intent| {
            intent.wallet_id != self.view.wallet_id || intent.chain_id != self.view.chain_id
        }) {
            return Err(SendError::invalid("Staking wallet or network was altered"));
        }
        self.validate_operation()?;
        if self.view.prepared_details != serde_json::to_string_pretty(&self.prepared)?
            || self.submission_digest()? != self.signed_digest
        {
            return Err(SendError::Invalid(
                "Transaction artifact content was altered".into(),
            ));
        }
        match (&self.submission, &self.view.stage) {
            (None, SendStage::Prepared)
                if self.view.signed_payload.is_none() && self.view.transaction_hash.is_none() => {}
            (Some(signed), SendStage::Signed)
                if self.view.signed_payload.as_ref() == Some(&signed.payload)
                    && self.view.transaction_hash == signed.transaction_hash => {}
            _ => {
                return Err(SendError::Invalid(
                    "Transaction stage does not match its signed content".into(),
                ));
            }
        }
        Ok(())
    }
}

impl StoredSend {
    /// An MWEB payment or peg-in pays the reviewed recipient the reviewed
    /// amount, and is one Litecoin Core relays.
    fn validate_litecoin_mweb(&self) -> Result<(), SendError> {
        let chain = self.view.chain_id;
        let (recipient, amount) = match &self.prepared {
            PreparedPayload::LitecoinMweb(prepared) => {
                prepared.check(chain)?;
                (&prepared.recipient, prepared.amount)
            }
            PreparedPayload::LitecoinPegIn(prepared) => {
                prepared.check(chain)?;
                (&prepared.recipient, prepared.amount)
            }
            _ => return Ok(()),
        };
        if *recipient != self.view.recipient
            || crate::decimal::from_units(u128::from(amount), 8) != self.view.amount
            || self.view.asset != chain.coin_symbol()
        {
            return Err(SendError::invalid("Transaction identity was altered"));
        }
        Ok(())
    }

    /// The operation is exactly what the prepared payload does, and a
    /// payload only an operation builds carries one.
    fn validate_operation(&self) -> Result<(), SendError> {
        let fee_is = |fee: &str, units: u128| {
            fee == crate::decimal::from_units(
                units,
                u32::from(self.view.chain_id.native_decimals()),
            )
        };
        self.validate_litecoin_mweb()?;
        let Some(operation) = &self.view.operation else {
            return match self.prepared {
                PreparedPayload::ZcashShielded(_)
                | PreparedPayload::LitecoinMweb(_)
                | PreparedPayload::XrpAccountDelete { .. }
                | PreparedPayload::StellarAccountMerge { .. }
                | PreparedPayload::NearDeleteKey(_)
                | PreparedPayload::SuiMerge(_)
                | PreparedPayload::SolanaAccountClosure(_)
                | PreparedPayload::XrpTrustSet(_)
                | PreparedPayload::StellarChangeTrust(_) => {
                    Err(SendError::invalid("The wallet operation was altered"))
                }
                // Staking builds a NEAR call too; anything else is an
                // operation's.
                PreparedPayload::NearFunctionCall(_) if self.view.staking.is_none() => {
                    Err(SendError::invalid("The wallet operation was altered"))
                }
                _ => Ok(()),
            };
        };
        let exact = self.view.staking.is_none()
            && match (operation, &self.prepared) {
                // `approve(spender, 0)` on its token, with no value: anything
                // else signed under its name would be another call.
                (
                    WalletOperation::RevokeApproval {
                        token,
                        spender,
                        network_fee,
                    },
                    PreparedPayload::Evm(prepared),
                ) => {
                    prepared.to.eq_ignore_ascii_case(token)
                        && self.view.recipient.eq_ignore_ascii_case(token)
                        && prepared.value_wei == 0
                        && prepared.data == super::evm::encode_erc20_approve(spender, 0)?
                        && fee_is(network_fee, prepared.maximum_fee_wei()?)
                }
                // Every transparent output moves to the wallet's own shielded
                // pool: no payment to anyone, the change is the amount.
                (
                    WalletOperation::ShieldTransparent {
                        amount,
                        network_fee,
                    },
                    PreparedPayload::ZcashShielded(prepared),
                ) => {
                    prepared.payments.is_empty()
                        && prepared.transparent_in_zat > 0
                        && prepared.shielded_in_zat == 0
                        && *amount == self.view.amount
                        && *amount == crate::decimal::from_units(u128::from(prepared.change_zat), 8)
                        && fee_is(network_fee, u128::from(prepared.fee_zat))
                }
                // Transparent funds pegged into the wallet's own MWEB
                // address: the amount is what arrives there.
                (
                    WalletOperation::ShieldTransparent {
                        amount,
                        network_fee,
                    },
                    PreparedPayload::LitecoinPegIn(prepared),
                ) => {
                    *amount == self.view.amount
                        && prepared
                            .mweb_fee
                            .checked_add(prepared.canonical_fee)
                            .is_some_and(|fee| fee_is(network_fee, u128::from(fee)))
                }
                // A payment out of MWEB funds carries no memo.
                (
                    WalletOperation::ShieldedPayment { memo, network_fee },
                    PreparedPayload::LitecoinMweb(prepared),
                ) => memo.is_none() && fee_is(network_fee, u128::from(prepared.fee)),
                // The one payment the proposal makes, of the reviewed amount
                // and memo to the reviewed recipient, from shielded notes
                // alone.
                (
                    WalletOperation::ShieldedPayment { memo, network_fee },
                    PreparedPayload::ZcashShielded(prepared),
                ) => {
                    prepared.transparent_in_zat == 0
                        && matches!(prepared.payments.as_slice(), [payment]
                            if payment.address == self.view.recipient
                                && payment.memo == *memo
                                && crate::decimal::from_units(u128::from(payment.zatoshis), 8)
                                    == self.view.amount)
                        && fee_is(network_fee, u128::from(prepared.fee_zat))
                }
                // The collection's `safeTransferFrom` of exactly this token
                // and quantity, from the wallet to the reviewed recipient,
                // with no value.
                (
                    WalletOperation::TransferNft {
                        contract,
                        standard,
                        token_id,
                        quantity,
                        network_fee,
                        ..
                    },
                    PreparedPayload::Evm(prepared),
                ) => {
                    prepared.to.eq_ignore_ascii_case(contract)
                        && self.view.asset.eq_ignore_ascii_case(contract)
                        && self.view.amount == *quantity
                        && prepared.value_wei == 0
                        && prepared.data
                            == super::evm::encode_nft_transfer(
                                *standard,
                                &self.view.sender,
                                &self.view.recipient,
                                token_id,
                                quantity,
                            )?
                        && fee_is(network_fee, prepared.maximum_fee_wei()?)
                }
                (
                    WalletOperation::CloseAccount {
                        destination,
                        network_fee,
                        ..
                    },
                    PreparedPayload::XrpAccountDelete { fee_drops: fee, .. }
                    | PreparedPayload::StellarAccountMerge {
                        fee_stroops: fee, ..
                    },
                ) => self.view.recipient == *destination && fee_is(network_fee, u128::from(*fee)),
                (
                    WalletOperation::CloseTokenAccounts {
                        accounts,
                        network_fee,
                        ..
                    },
                    PreparedPayload::SolanaAccountClosure(prepared),
                ) => {
                    self.view.recipient == self.view.sender
                        && prepared
                            .accounts
                            .iter()
                            .map(|(account, _)| account)
                            .eq(accounts.iter())
                        && prepared.is_exact(&self.view.sender)
                        && prepared
                            .transaction
                            .network_fee
                            .is_some_and(|fee| fee_is(network_fee, u128::from(fee)))
                }
                (
                    WalletOperation::MergeCoins {
                        coin_type,
                        objects,
                        network_fee,
                    },
                    PreparedPayload::SuiMerge(prepared),
                ) => {
                    self.view.recipient == self.view.sender
                        && prepared.coin_type == *coin_type
                        && prepared.object_count() as u64 == *objects
                        && prepared.is_exact(&self.view.sender)
                        && fee_is(network_fee, u128::from(prepared.transaction.gas_budget))
                }
                // The line it names and nothing else, set to the largest
                // limit or removed: the account changes its own line.
                (
                    WalletOperation::TrustAsset {
                        asset, network_fee, ..
                    }
                    | WalletOperation::RemoveTrustLine {
                        asset, network_fee, ..
                    },
                    PreparedPayload::XrpTrustSet(prepared),
                ) => {
                    let removes = matches!(operation, WalletOperation::RemoveTrustLine { .. });
                    self.view.recipient == self.view.sender
                        && prepared.asset == *asset
                        && prepared.removes() == removes
                        && (removes
                            || prepared.limit
                                == crate::api::xrpl_amount::IouValue::parse(
                                    super::xrp_issued::MAX_TRUST_LIMIT,
                                )
                                .expect("the largest limit")
                                .to_decimal())
                        && fee_is(network_fee, u128::from(prepared.fee_drops))
                }
                (
                    WalletOperation::TrustAsset {
                        asset, network_fee, ..
                    }
                    | WalletOperation::RemoveTrustLine {
                        asset, network_fee, ..
                    },
                    PreparedPayload::StellarChangeTrust(prepared),
                ) => {
                    let removes = matches!(operation, WalletOperation::RemoveTrustLine { .. });
                    self.view.recipient == self.view.sender
                        && prepared.asset == *asset
                        && prepared.removes() == removes
                        && (removes || prepared.limit_stroops == super::stellar::MAX_TRUST_LIMIT)
                        && fee_is(network_fee, u128::from(prepared.fee_stroops))
                }
                (
                    WalletOperation::DeleteAccessKey {
                        public_key,
                        network_fee,
                        ..
                    },
                    PreparedPayload::NearDeleteKey(prepared),
                ) => {
                    self.view.recipient == self.view.sender
                        && prepared.signer == self.view.sender
                        && *public_key
                            == format!(
                                "ed25519:{}",
                                bs58::encode(prepared.deleted_key).into_string()
                            )
                        && fee_is(
                            network_fee,
                            prepared.fee_budget.parse().map_err(SendError::invalid)?,
                        )
                }
                // The account unregistering itself from the token it names,
                // with no `force` and the one yoctoNEAR; the deposit returns
                // to it.
                (
                    WalletOperation::RefundTokenStorage {
                        contract,
                        refund,
                        network_fee,
                    },
                    PreparedPayload::NearFunctionCall(prepared),
                ) => {
                    self.view.recipient == self.view.sender
                        && prepared.is_storage_unregister(&self.view.sender, contract)
                        && self.view.amount == *refund
                        && fee_is(
                            network_fee,
                            prepared.fee_budget.parse().map_err(SendError::invalid)?,
                        )
                }
                _ => false,
            };
        if !exact {
            return Err(SendError::invalid("The wallet operation was altered"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/nft.rs"]
mod nft_tests;

#[cfg(test)]
#[path = "tests/zcash_shielded_stages.rs"]
mod zcash_shielded_tests;

#[cfg(test)]
#[path = "tests/near_storage_refund.rs"]
mod near_storage_refund_tests;
