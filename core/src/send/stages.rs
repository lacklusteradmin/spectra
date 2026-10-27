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
    pub utxo: crate::api::utxo::UtxoInput,
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
    /// What the review says before signing, in the order it says it.
    pub warnings: Vec<ReviewWarning>,
    pub staking: Option<crate::staking::StakingReview>,
    /// What the asset's own rules do to the transfer, when they do more than
    /// move the amount.
    pub transfer_terms: Option<AssetTransferTerms>,
    /// What the transaction pays the network, when it is known at review.
    #[uniffi(default = None)]
    pub network_fee: Option<NetworkFee>,
    /// The amount and that fee together, when the fee is paid in the sent
    /// asset: what leaves the wallet.
    #[uniffi(default = None)]
    pub total: Option<String>,
}

impl SendArtifactReview {
    /// A transfer's review: the reasons it looks risky, what its EVM
    /// contract checks found, and a destination that is the user's own.
    pub(crate) fn of_transfer(
        risks: Vec<crate::send::flow::HighRiskSendWarning>,
        contracts: Vec<crate::send::flow::EvmRecipientPreflightWarning>,
        own_address: bool,
    ) -> Self {
        let warnings = risks
            .into_iter()
            .map(|warning| ReviewWarning::Risk { warning })
            .chain(
                contracts
                    .into_iter()
                    .map(|warning| ReviewWarning::Contract { warning }),
            )
            .chain(own_address.then_some(ReviewWarning::OwnAddress))
            .collect();
        Self {
            warnings,
            ..Self::default()
        }
    }

    /// Adopt what the asset's rules do to the transfer, read from the
    /// prepared transaction, and say each part of it before signing.
    pub(crate) fn adopt_transfer_terms(&mut self, terms: Option<AssetTransferTerms>) {
        if let Some(terms) = &terms {
            if terms.fee != "0" {
                self.warnings.push(ReviewWarning::TokenFee {
                    fee: terms.fee.clone(),
                    received: terms.received.clone(),
                });
            }
            if let Some(amount) = &terms.carried_native {
                self.warnings.push(ReviewWarning::CarriesNative {
                    amount: amount.clone(),
                });
            }
            if let Some(deposit) = &terms.recipient_registration {
                self.warnings.push(ReviewWarning::RegistersRecipient {
                    deposit: deposit.clone(),
                });
            }
            if let Some(program) = &terms.hook_program {
                self.warnings.push(ReviewWarning::TransferHook {
                    program: program.clone(),
                });
            }
        }
        self.transfer_terms = terms;
    }
}

/// Something a review says before a transaction is signed. Which apply, and
/// in what order, are core's, so every page that reviews an artifact — the
/// composer, its signing prompt, a wallet tool's sheet — says the same
/// things. Front ends word each one; a new one is a compile error on every
/// front end that has not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewWarning {
    /// A reason the send looks risky.
    Risk {
        warning: crate::send::flow::HighRiskSendWarning,
    },
    /// A reason the EVM recipient or token contract looks wrong.
    Contract {
        warning: crate::send::flow::EvmRecipientPreflightWarning,
    },
    /// The destination is one of the user's own wallets: a self-send, which
    /// is to be meant.
    OwnAddress,
    /// The asset's rules take `fee` of the amount on the way, and the
    /// recipient receives `received`, both exact decimals of the asset.
    TokenFee { fee: String, received: String },
    /// `amount` of the network's coin goes with the token, the least its
    /// output can hold, and the recipient keeps it.
    CarriesNative { amount: String },
    /// The token has not registered the recipient: the send registers it
    /// first, depositing `deposit` of the network's coin with the token.
    RegistersRecipient { deposit: String },
    /// The token runs `program` on every transfer, and it can refuse this one.
    TransferHook { program: String },
}

/// What a reviewed transaction pays the network, in its own coin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct NetworkFee {
    /// An exact decimal of the network's coin.
    pub amount: String,
    /// How `amount` relates to what the network charges.
    pub bound: FeeBound,
    /// `amount` in the display currency when the transaction was built, when
    /// the coin had a quote.
    pub value: Option<f64>,
}

impl NetworkFee {
    /// `units` of `chain`'s coin, valued when the review is stored.
    pub(crate) fn of_units(chain: crate::registry::Chain, units: u128, bound: FeeBound) -> Self {
        Self {
            amount: crate::amount::decimal::from_units(units, u32::from(chain.native_decimals())),
            bound,
            value: None,
        }
    }
}

/// How a reviewed fee relates to what the network charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum FeeBound {
    /// What the quote the transaction was built from estimated.
    Estimate,
    /// What the signed transaction pays.
    Exact,
    /// The most it can cost: the network charges what it uses, up to this.
    AtMost,
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
                let sent = crate::amount::decimal::from_units(*amount, token_decimals?);
                Some(AssetTransferTerms {
                    debited: sent.clone(),
                    received: sent,
                    fee: "0".into(),
                    hook_program: None,
                    carried_native: None,
                    recipient_registration: Some(crate::amount::decimal::from_units(
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
/// Its fee is the review's, read from the prepared payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WalletOperation {
    /// Sets an ERC-20 allowance back to zero: `recipient` and `asset` are
    /// the token, and the prepared call is `approve(spender, 0)` on it.
    RevokeApproval { token: String, spender: String },
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
    },
    /// Closes empty token accounts into their owner, returning their rent:
    /// the owner is the recipient and `amount` the rent.
    CloseTokenAccounts {
        /// The closed accounts.
        accounts: Vec<String>,
        /// The rent they return, as an exact decimal of SOL.
        rent: String,
    },
    /// Merges a Sui coin type's objects into one: the account is the
    /// recipient and nothing leaves it.
    MergeCoins {
        coin_type: String,
        /// How many objects become one.
        objects: u64,
    },
    /// Opens a trust line (XRP Ledger) or trustline (Stellar) so the account
    /// can hold an issued asset: the account is the recipient and nothing
    /// leaves it but the fee.
    TrustAsset {
        /// `CODE.rIssuer` or `CODE:ISSUER`.
        asset: String,
        /// The reserve the line locks, as an exact decimal of the native coin.
        reserve: String,
    },
    /// Removes an empty trust line, freeing its reserve.
    RemoveTrustLine {
        asset: String,
        /// The reserve the line held, as an exact decimal of the native coin.
        reserve: String,
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
    },
    /// Moves a Zcash wallet's transparent funds into its own shielded pool:
    /// the recipient is the wallet's unified address and `amount` what
    /// arrives there.
    ShieldTransparent {
        /// What arrives in the shielded pool, as an exact decimal of ZEC.
        amount: String,
    },
    /// Pays the recipient the amount from a Zcash wallet's shielded funds.
    ShieldedPayment {
        /// The text memo a shielded recipient reads, exactly as it is signed.
        memo: Option<String>,
    },
    /// Deletes one of a NEAR account's function-call keys: the account is
    /// the recipient, and the dapp the key could call loses that access.
    DeleteAccessKey {
        /// The deleted key, as `ed25519:…`.
        public_key: String,
        /// The contract the key could call.
        receiver: String,
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

/// What a newly prepared artifact says beyond its request. The rest of its
/// view — wallet, network, recipient, amount and memo — is the request's,
/// which [`StoredSend::validate`] holds it to anyway.
pub(crate) struct NewArtifact {
    pub request: super::SendExecutionRequest,
    pub prepared: PreparedPayload,
    pub sender: String,
    pub asset: String,
    pub symbol: String,
    pub staking: Option<crate::staking::StakingRequest>,
    pub operation: Option<WalletOperation>,
    pub review: SendArtifactReview,
    pub signing_payload_hex: String,
}

impl StoredSend {
    /// A prepared artifact nothing has signed or sent yet, its review digest
    /// committing to everything it holds. A wallet operation's fee is read
    /// from its prepared payload; `value_fee` prices the fee in the display
    /// currency where the review has not.
    pub fn prepared(
        new: NewArtifact,
        value_fee: impl FnOnce(&str) -> Option<f64>,
    ) -> Result<Self, SendError> {
        let NewArtifact {
            request,
            prepared,
            sender,
            asset,
            symbol,
            staking,
            operation,
            review,
            signing_payload_hex,
        } = new;
        let mut stored = Self {
            view: SendArtifact {
                id: crate::store::new_transaction_id(),
                revision: 0,
                stage: SendStage::Prepared,
                wallet_id: request.wallet_id.clone(),
                chain_id: request.chain_id,
                sender,
                recipient: request.to_address.clone(),
                memo: request.memo.clone(),
                amount: request.amount_str.clone(),
                asset,
                symbol,
                staking,
                operation,
                created_at: crate::store::now_unix().floor(),
                review_digest: String::new(),
                review,
                prepared_details: serde_json::to_string_pretty(&prepared)?,
                signing_payload_hex,
                signed_payload: None,
                transaction_hash: None,
                attempts: Vec::new(),
                selected_endpoints: Vec::new(),
            },
            request,
            prepared,
            submission: None,
            signed_digest: None,
            substrate_verified_through: None,
            icp_staking_receipts: Vec::new(),
        };
        if let Some(operation) = &stored.view.operation {
            let (units, bound) = stored
                .operation_fee(operation)?
                .ok_or_else(|| SendError::invalid("The wallet operation was altered"))?;
            stored.view.review.network_fee =
                Some(NetworkFee::of_units(stored.view.chain_id, units, bound));
        }
        if let Some(fee) = &mut stored.view.review.network_fee
            && fee.value.is_none()
        {
            fee.value = value_fee(&fee.amount);
        }
        stored.view.review_digest = stored.digest()?;
        Ok(stored)
    }

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
            let active: &[super::icp_staking::SignedIcpStakingCall] =
                match self.submission.as_ref().map(|s| &s.payload) {
                    None => &[],
                    Some(super::payload::SignedPayload::IcpStaking(calls)) => calls,
                    Some(_) => {
                        return Err(SendError::invalid(
                            "Staking was not signed as ICP ingress calls",
                        ));
                    }
                };
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
                if self.view.signed_payload == Some(signed.payload.to_string())
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
            || crate::amount::decimal::from_units(u128::from(amount), 8) != self.view.amount
            || self.view.asset != chain.coin_symbol()
        {
            return Err(SendError::invalid("Transaction identity was altered"));
        }
        Ok(())
    }

    /// The operation is exactly what the prepared payload does, and a
    /// payload only an operation builds carries one.
    fn validate_operation(&self) -> Result<(), SendError> {
        self.validate_litecoin_mweb()?;
        let altered = || SendError::invalid("The wallet operation was altered");
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
                | PreparedPayload::StellarChangeTrust(_) => Err(altered()),
                // Staking builds a NEAR call too; anything else is an
                // operation's.
                PreparedPayload::NearFunctionCall(_) if self.view.staking.is_none() => {
                    Err(altered())
                }
                _ => Ok(()),
            };
        };
        if self.view.staking.is_some() {
            return Err(altered());
        }
        let (units, bound) = self.operation_fee(operation)?.ok_or_else(altered)?;
        let fee = NetworkFee::of_units(self.view.chain_id, units, bound);
        if self
            .view
            .review
            .network_fee
            .as_ref()
            .is_none_or(|reviewed| reviewed.amount != fee.amount || reviewed.bound != fee.bound)
        {
            return Err(altered());
        }
        Ok(())
    }

    /// What the prepared payload pays the network, in base units of the
    /// coin, and how — when it is exactly `operation` and nothing else.
    /// `None` when it is not.
    fn operation_fee(
        &self,
        operation: &WalletOperation,
    ) -> Result<Option<(u128, FeeBound)>, SendError> {
        use FeeBound::{AtMost, Exact};
        let fee = |exact: bool, units: u128, bound: FeeBound| exact.then_some((units, bound));
        Ok(match (operation, &self.prepared) {
            // `approve(spender, 0)` on its token, with no value: anything
            // else signed under its name would be another call. Its fee is
            // the call's ceiling.
            (
                WalletOperation::RevokeApproval { token, spender },
                PreparedPayload::Evm(prepared),
            ) => fee(
                prepared.to.eq_ignore_ascii_case(token)
                    && self.view.recipient.eq_ignore_ascii_case(token)
                    && prepared.value_wei == 0
                    && prepared.data == super::evm::encode_erc20_approve(spender, 0)?,
                prepared.maximum_fee_wei()?,
                AtMost,
            ),
            // Every transparent output moves to the wallet's own shielded
            // pool: no payment to anyone, the change is the amount.
            (
                WalletOperation::ShieldTransparent { amount },
                PreparedPayload::ZcashShielded(prepared),
            ) => fee(
                prepared.payments.is_empty()
                    && prepared.transparent_in_zat > 0
                    && prepared.shielded_in_zat == 0
                    && *amount == self.view.amount
                    && *amount
                        == crate::amount::decimal::from_units(u128::from(prepared.change_zat), 8),
                u128::from(prepared.fee_zat),
                Exact,
            ),
            // Transparent funds pegged into the wallet's own MWEB
            // address: the amount is what arrives there.
            (
                WalletOperation::ShieldTransparent { amount },
                PreparedPayload::LitecoinPegIn(prepared),
            ) => {
                let paid = prepared
                    .mweb_fee
                    .checked_add(prepared.canonical_fee)
                    .ok_or_else(|| SendError::invalid("The peg-in's fee overflows"))?;
                fee(*amount == self.view.amount, u128::from(paid), Exact)
            }
            // A payment out of MWEB funds carries no memo.
            (
                WalletOperation::ShieldedPayment { memo },
                PreparedPayload::LitecoinMweb(prepared),
            ) => fee(memo.is_none(), u128::from(prepared.fee), Exact),
            // The one payment the proposal makes, of the reviewed amount
            // and memo to the reviewed recipient, from shielded notes
            // alone.
            (
                WalletOperation::ShieldedPayment { memo },
                PreparedPayload::ZcashShielded(prepared),
            ) => fee(
                prepared.transparent_in_zat == 0
                    && matches!(prepared.payments.as_slice(), [payment]
                        if payment.address == self.view.recipient
                            && payment.memo == *memo
                            && crate::amount::decimal::from_units(u128::from(payment.zatoshis), 8)
                                == self.view.amount),
                u128::from(prepared.fee_zat),
                Exact,
            ),
            // The collection's `safeTransferFrom` of exactly this token
            // and quantity, from the wallet to the reviewed recipient,
            // with no value.
            (
                WalletOperation::TransferNft {
                    contract,
                    standard,
                    token_id,
                    quantity,
                    ..
                },
                PreparedPayload::Evm(prepared),
            ) => fee(
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
                        )?,
                prepared.maximum_fee_wei()?,
                AtMost,
            ),
            (
                WalletOperation::CloseAccount { destination, .. },
                PreparedPayload::XrpAccountDelete {
                    fee_drops: paid, ..
                }
                | PreparedPayload::StellarAccountMerge {
                    fee_stroops: paid, ..
                },
            ) => fee(
                self.view.recipient == *destination,
                u128::from(*paid),
                Exact,
            ),
            (
                WalletOperation::CloseTokenAccounts { accounts, .. },
                PreparedPayload::SolanaAccountClosure(prepared),
            ) => match prepared.transaction.network_fee {
                Some(paid) => fee(
                    self.view.recipient == self.view.sender
                        && prepared
                            .accounts
                            .iter()
                            .map(|(account, _)| account)
                            .eq(accounts.iter())
                        && prepared.is_exact(&self.view.sender),
                    u128::from(paid),
                    Exact,
                ),
                None => None,
            },
            // The gas budget is the most the merge costs; a merge often
            // refunds more storage than it spends.
            (
                WalletOperation::MergeCoins { coin_type, objects },
                PreparedPayload::SuiMerge(prepared),
            ) => fee(
                self.view.recipient == self.view.sender
                    && prepared.coin_type == *coin_type
                    && prepared.object_count() as u64 == *objects
                    && prepared.is_exact(&self.view.sender),
                u128::from(prepared.transaction.gas_budget),
                AtMost,
            ),
            // The line it names and nothing else, set to the largest
            // limit or removed: the account changes its own line.
            (
                WalletOperation::TrustAsset { asset, .. }
                | WalletOperation::RemoveTrustLine { asset, .. },
                PreparedPayload::XrpTrustSet(prepared),
            ) => {
                let removes = matches!(operation, WalletOperation::RemoveTrustLine { .. });
                fee(
                    self.view.recipient == self.view.sender
                        && prepared.asset == *asset
                        && prepared.removes() == removes
                        && (removes
                            || prepared.limit
                                == crate::api::xrpl_amount::IouValue::parse(
                                    super::xrp_issued::MAX_TRUST_LIMIT,
                                )
                                .expect("the largest limit")
                                .to_decimal()),
                    u128::from(prepared.fee_drops),
                    Exact,
                )
            }
            (
                WalletOperation::TrustAsset { asset, .. }
                | WalletOperation::RemoveTrustLine { asset, .. },
                PreparedPayload::StellarChangeTrust(prepared),
            ) => {
                let removes = matches!(operation, WalletOperation::RemoveTrustLine { .. });
                fee(
                    self.view.recipient == self.view.sender
                        && prepared.asset == *asset
                        && prepared.removes() == removes
                        && (removes || prepared.limit_stroops == super::stellar::MAX_TRUST_LIMIT),
                    u128::from(prepared.fee_stroops),
                    Exact,
                )
            }
            // A NEAR call's fee is the gas it may burn at most.
            (
                WalletOperation::DeleteAccessKey { public_key, .. },
                PreparedPayload::NearDeleteKey(prepared),
            ) => fee(
                self.view.recipient == self.view.sender
                    && prepared.signer == self.view.sender
                    && *public_key
                        == format!(
                            "ed25519:{}",
                            bs58::encode(prepared.deleted_key).into_string()
                        ),
                prepared.fee_budget.parse().map_err(SendError::invalid)?,
                AtMost,
            ),
            // The account unregistering itself from the token it names,
            // with no `force` and the one yoctoNEAR; the deposit returns
            // to it.
            (
                WalletOperation::RefundTokenStorage { contract, refund },
                PreparedPayload::NearFunctionCall(prepared),
            ) => fee(
                self.view.recipient == self.view.sender
                    && prepared.is_storage_unregister(&self.view.sender, contract)
                    && self.view.amount == *refund,
                prepared.fee_budget.parse().map_err(SendError::invalid)?,
                AtMost,
            ),
            _ => None,
        })
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
