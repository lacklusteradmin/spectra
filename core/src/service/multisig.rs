//! A multisig account's sessions: a spend its signers sign, built here or
//! read from another signer or coordinator, kept until it is submitted or
//! discarded. Every scheme keeps what Bitcoin's PSBTs keep: a policy core
//! reads or derives itself, a review digest each signature is given for,
//! signatures verified before they count, and the session itself in
//! `multisig_sessions`, so a restart loses nothing. No scheme needs a
//! coordination service: what one signer hands the next is the session's
//! data, as a PSBT is.
//!
//! What a session shows is reviewed again from its stored transaction and
//! signatures each time, against the policy it was built under; where that
//! policy lives on the network, it is read again before a signature or a
//! submission, and a session built under another one is refused.
//!
//! Each scheme's own rules sit in its `multisig_<scheme>` sibling; this
//! module owns the records, the storage and the dispatch.
use super::*;
use crate::store::state::WalletState;

/// How an account's signatures combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum MultisigScheme {
    /// A `sortedmulti` account on a UTXO network: P2WSH on Bitcoin and
    /// Litecoin, spent through PSBTs (BIP-174); P2SH on Bitcoin Cash,
    /// through BCHN's PSBTs, and on Dogecoin, through partially signed
    /// transactions as Dogecoin Core passes them.
    SortedMulti,
    /// A Safe contract on an EVM network: its owners sign a `SafeTx`, and
    /// one of them executes it.
    Safe,
    /// A Tron account's owner and active permissions: weighted keys sign a
    /// transaction naming the permission that covers it.
    TronPermissions,
    /// An XRP Ledger account's signer list: weighted accounts sign a
    /// transaction whose every field was fixed first.
    XrplSignerList,
    /// A Stellar account's weighted signers and its low, medium and high
    /// thresholds: every signature travels in the envelope.
    StellarSigners,
    /// A Sui multisig address: weighted keys whose signatures combine into
    /// one `MultiSig` signature.
    SuiMultisig,
    /// An Aptos MultiKey account: a number of its keys sign, under one
    /// authenticator.
    AptosMultiKey,
    /// A Cardano native script: its cosigners' keys witness a spend that
    /// carries the script.
    CardanoNativeScript,
    /// A Substrate multisig (`pallet-multisig`): its signatories approve a
    /// call on chain, each from its own account, and the approval that meets
    /// the threshold executes it.
    SubstrateMultisig,
    /// TON's multisig v2 contract: its signers propose and approve an order
    /// on chain, each from its own wallet, and the approval that meets the
    /// threshold executes it.
    TonMultisig,
}

impl MultisigScheme {
    /// The scheme a wallet spends through, or `None` for a wallet that is
    /// one key's alone.
    pub(crate) fn of(wallet: &WalletState) -> Option<Self> {
        let chain = wallet.chain_id;
        if wallet.multisig_policy.is_some() {
            if chain.utxo_multisig_script().is_some() {
                return Some(Self::SortedMulti);
            }
            return match chain.mainnet_counterpart() {
                Chain::Sui => Some(Self::SuiMultisig),
                Chain::Aptos => Some(Self::AptosMultiKey),
                Chain::Cardano => Some(Self::CardanoNativeScript),
                Chain::Polkadot | Chain::Bittensor => Some(Self::SubstrateMultisig),
                _ => None,
            };
        }
        // A key's own address is never a contract: a Safe is watched.
        if chain.is_evm() && wallet.is_watch_only() {
            return Some(Self::Safe);
        }
        // Nor is a key's TON wallet a multisig: a multisig is watched.
        if chain.mainnet_counterpart() == Chain::Ton && wallet.is_watch_only() {
            return Some(Self::TonMultisig);
        }
        // Any Tron account has permissions, its own key's or several.
        if chain.mainnet_counterpart() == Chain::Tron {
            return Some(Self::TronPermissions);
        }
        // So has any XRP account, a signer list or its own key.
        if chain.mainnet_counterpart() == Chain::Xrp {
            return Some(Self::XrplSignerList);
        }
        // And any Stellar account, signers or its master key.
        if chain.mainnet_counterpart() == Chain::Stellar {
            return Some(Self::StellarSigners);
        }
        None
    }
}

/// One of the keys or accounts an account's policy names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigSigner {
    /// How the network names it: an address, a public key, or a cosigner's
    /// key fingerprint.
    pub signer: String,
    /// What its signature counts toward the threshold.
    pub weight: u64,
    /// The session carries its valid signature (or, where approvals are
    /// transactions, its approval is on the network). Always `false` in a
    /// policy.
    pub signed: bool,
    /// The wallet on this device that holds it and can sign as it, if any.
    pub wallet_id: Option<String>,
}

/// One coin a UTXO session spends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigInput {
    /// `txid:vout`.
    pub outpoint: String,
    pub address: String,
    /// In the network's smallest unit.
    pub value: String,
    /// How many signers' valid signatures it carries.
    pub signatures: u32,
}

/// What a session pays: a payment, change back to the account, or a call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigOutput {
    pub address: String,
    /// In the network's smallest unit.
    pub value: String,
    /// Change back to the account.
    pub is_change: bool,
    /// Call data the transaction carries to `address`, hex; `None` for a
    /// plain payment.
    pub data: Option<String>,
    /// The token paid, by its contract or id; `None` for the network's own
    /// coin.
    pub asset: Option<String>,
    /// The destination tag or memo the payment carries.
    pub memo: Option<String>,
}

/// A session as a signer reviews it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigSession {
    pub id: String,
    pub wallet_id: String,
    pub chain: Chain,
    pub scheme: MultisigScheme,
    /// The transaction's id on its network: fixed before anyone signs,
    /// except on the XRP Ledger, where it covers the signatures gathered.
    pub transaction_id: String,
    /// What a signature is given for. Signing names it, and a session that
    /// no longer has it is refused.
    pub review_digest: String,
    pub threshold: u64,
    pub signers: Vec<MultisigSigner>,
    /// The summed weight of the signers who signed.
    pub signed_weight: u64,
    /// The coins a UTXO session spends; empty on an account network.
    pub inputs: Vec<MultisigInput>,
    pub outputs: Vec<MultisigOutput>,
    /// The fee the transaction pays, in the network's smallest unit.
    pub fee: String,
    /// The account sequence, nonce or order number the transaction takes.
    pub sequence: Option<String>,
    /// When the network stops accepting it, in unix seconds.
    pub expires_at: Option<u64>,
    /// The last ledger, slot or block that may include it, where the
    /// network bounds it so rather than by time.
    pub expires_at_height: Option<u64>,
    /// Enough signers signed for the network to accept it.
    pub complete: bool,
    /// What to hand the next signer.
    pub data: String,
    pub submitted_txid: Option<String>,
}

/// One set of signers and the threshold they meet together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigPermission {
    /// What the network calls it.
    pub name: String,
    pub threshold: u64,
    pub signers: Vec<MultisigSigner>,
    /// What it authorizes, where the network limits it.
    pub covers: Vec<String>,
}

/// How a session's signatures reach the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum MultisigSubmission {
    /// Once enough signers signed, the finished transaction is submitted as
    /// it is.
    AsIs,
    /// Once enough signers signed, one of them executes it in a transaction
    /// of its own, paying its fee (a Safe).
    ByExecutor,
    /// Each signature is an approval its signer submits; the one that meets
    /// the threshold executes the transfer, and nothing is left to submit.
    ByApproval,
}

/// A multisig account's policy, as core read or derived it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigAccount {
    pub wallet_id: String,
    pub chain: Chain,
    pub scheme: MultisigScheme,
    pub address: String,
    pub permissions: Vec<MultisigPermission>,
    /// What can move or block the account's funds beside its signers.
    pub warnings: Vec<crate::LocalizableMessage>,
    pub submission: MultisigSubmission,
    /// The wallets on this device that may sign for it: the account wallet
    /// itself where it holds a cosigner's phrase, else wallets of the
    /// signers' own.
    pub signer_wallet_ids: Vec<String>,
}

impl MultisigScheme {
    pub(crate) fn submission(self) -> MultisigSubmission {
        match self {
            Self::Safe => MultisigSubmission::ByExecutor,
            Self::SubstrateMultisig | Self::TonMultisig => MultisigSubmission::ByApproval,
            _ => MultisigSubmission::AsIs,
        }
    }
}

/// A payment a session spends from the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MultisigSpend {
    pub to_address: String,
    /// In the network's coin, as a decimal.
    pub amount: String,
    /// A UTXO network's fee rate in its unit per virtual byte; `None` takes
    /// the network's.
    pub fee_rate: Option<String>,
    /// How long signatures may be gathered, in seconds, where the network
    /// bounds it; `None` takes the scheme's default.
    pub expires_in_secs: Option<u64>,
    /// A destination tag or memo, where the network's payments carry one.
    pub memo: Option<crate::send::payment_memo::PaymentMemo>,
}

/// A session's scheme-specific content: the transaction and the
/// signatures gathered for it, nothing derived from them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scheme", rename_all = "camelCase")]
pub(crate) enum SessionBody {
    Psbt { psbt: String },
    P2sh(crate::send::p2sh_multisig::P2shSpend),
    Safe(Box<super::multisig_safe::SafeSession>),
    Tron(super::multisig_tron::TronSession),
    Xrp(super::multisig_xrp::XrpSession),
    Stellar(super::multisig_stellar::StellarSession),
    Sui(super::multisig_sui::SuiSession),
    Aptos(super::multisig_aptos::AptosSession),
    Cardano(super::multisig_cardano::CardanoSession),
    Substrate(super::multisig_substrate::SubstrateSession),
    Ton(super::multisig_ton::TonSession),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredSession {
    pub id: String,
    pub wallet_id: String,
    pub chain: Chain,
    pub created_at: f64,
    pub submitted_txid: Option<String>,
    pub body: SessionBody,
}

impl StoredSession {
    pub(crate) fn new(wallet: &WalletState, body: SessionBody) -> Self {
        Self {
            id: crate::store::new_event_id(),
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            created_at: crate::wallet_db::now_secs() as f64,
            submitted_txid: None,
            body,
        }
    }
}

/// What a scheme's review of a session says, before the session's identity
/// is put around it.
pub(crate) struct SessionReview {
    pub transaction_id: String,
    pub digest: String,
    pub threshold: u64,
    /// Every signer of the policy, `signed` for those whose valid signature
    /// the session carries.
    pub signers: Vec<MultisigSigner>,
    pub inputs: Vec<MultisigInput>,
    pub outputs: Vec<MultisigOutput>,
    pub fee: String,
    pub sequence: Option<String>,
    pub expires_at: Option<u64>,
    pub expires_at_height: Option<u64>,
    pub complete: bool,
    pub data: String,
}

/// The signer's weight summed over those who signed.
pub(crate) fn signed_weight(signers: &[MultisigSigner]) -> u64 {
    signers
        .iter()
        .filter(|signer| signer.signed)
        .map(|signer| signer.weight)
        .sum()
}

impl WalletService {
    pub(super) async fn multisig_wallet(
        &self,
        wallet_id: &str,
    ) -> Result<(WalletState, MultisigScheme), SpectraBridgeError> {
        let wallet = self.stored_wallet(wallet_id).await?;
        let scheme = MultisigScheme::of(&wallet)
            .ok_or_else(|| SpectraBridgeError::invalid("This wallet is not a multisig account."))?;
        Ok((wallet, scheme))
    }

    pub(super) async fn multisig_load(
        &self,
        id: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let id = id.to_string();
        let payload =
            tokio::task::spawn_blocking(move || crate::wallet_db::multisig_session_load(&db, &id))
                .await
                .map_err(SpectraBridgeError::failure)??
                .ok_or_else(|| SpectraBridgeError::invalid("No such multisig session"))?;
        Ok(serde_json::from_str(&payload)?)
    }

    pub(super) async fn multisig_save(
        &self,
        stored: &StoredSession,
    ) -> Result<(), SpectraBridgeError> {
        let db = self.bound_database().await?;
        let payload = serde_json::to_string(stored)?;
        let (id, wallet_id) = (stored.id.clone(), stored.wallet_id.clone());
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::multisig_session_save(&db, &id, &wallet_id, &payload)
        })
        .await
        .map_err(SpectraBridgeError::failure)??;
        Ok(())
    }

    pub(super) async fn multisig_stored_for_wallet(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<StoredSession>, SpectraBridgeError> {
        let db = self.bound_database().await?;
        let wallet_id = wallet_id.to_string();
        tokio::task::spawn_blocking(move || {
            crate::wallet_db::multisig_sessions_for_wallet(&db, &wallet_id)
        })
        .await
        .map_err(SpectraBridgeError::failure)??
        .iter()
        .map(|payload| serde_json::from_str(payload).map_err(SpectraBridgeError::from))
        .collect()
    }

    /// The open sessions of `wallet_id`: not yet submitted.
    pub(super) async fn multisig_open_sessions(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<StoredSession>, SpectraBridgeError> {
        Ok(self
            .multisig_stored_for_wallet(wallet_id)
            .await?
            .into_iter()
            .filter(|stored| stored.submitted_txid.is_none())
            .collect())
    }

    /// `stored` as a signer reviews it: the scheme's review, with each
    /// signer this device holds named by its wallet.
    pub(super) async fn multisig_view(
        &self,
        wallet: &WalletState,
        stored: &StoredSession,
    ) -> Result<MultisigSession, SpectraBridgeError> {
        let (scheme, review) = match &stored.body {
            SessionBody::Psbt { psbt } => (
                MultisigScheme::SortedMulti,
                super::multisig_psbt::review(wallet, psbt)?,
            ),
            SessionBody::P2sh(spend) => (
                MultisigScheme::SortedMulti,
                super::multisig_psbt::review_p2sh(wallet, spend)?,
            ),
            SessionBody::Safe(session) => (
                MultisigScheme::Safe,
                super::multisig_safe::review(wallet, session)?,
            ),
            SessionBody::Tron(session) => (
                MultisigScheme::TronPermissions,
                super::multisig_tron::review(wallet, session)?,
            ),
            SessionBody::Xrp(session) => (
                MultisigScheme::XrplSignerList,
                super::multisig_xrp::review(wallet, session)?,
            ),
            SessionBody::Stellar(session) => (
                MultisigScheme::StellarSigners,
                super::multisig_stellar::review(wallet, session)?,
            ),
            SessionBody::Sui(session) => (
                MultisigScheme::SuiMultisig,
                super::multisig_sui::review(wallet, session)?,
            ),
            SessionBody::Aptos(session) => (
                MultisigScheme::AptosMultiKey,
                super::multisig_aptos::review(wallet, session)?,
            ),
            SessionBody::Cardano(session) => (
                MultisigScheme::CardanoNativeScript,
                super::multisig_cardano::review(wallet, session)?,
            ),
            SessionBody::Substrate(session) => (
                MultisigScheme::SubstrateMultisig,
                super::multisig_substrate::review(wallet, session)?,
            ),
            SessionBody::Ton(session) => (
                MultisigScheme::TonMultisig,
                super::multisig_ton::review(wallet, session)?,
            ),
        };
        let mut signers = review.signers;
        self.name_signer_wallets(wallet, scheme, &mut signers).await;
        Ok(MultisigSession {
            id: stored.id.clone(),
            wallet_id: stored.wallet_id.clone(),
            chain: stored.chain,
            scheme,
            transaction_id: review.transaction_id,
            review_digest: review.digest,
            threshold: review.threshold,
            signed_weight: signed_weight(&signers),
            signers,
            inputs: review.inputs,
            outputs: review.outputs,
            fee: review.fee,
            sequence: review.sequence,
            expires_at: review.expires_at,
            expires_at_height: review.expires_at_height,
            complete: review.complete,
            data: review.data,
            submitted_txid: stored.submitted_txid.clone(),
        })
    }

    /// Name the wallet on this device that holds each signer, where one
    /// does: a wallet with keys, on the account's network, at the signer's
    /// address.
    pub(super) async fn name_signer_wallets(
        &self,
        account: &WalletState,
        scheme: MultisigScheme,
        signers: &mut [MultisigSigner],
    ) {
        let chain = account.chain_id;
        let state = self.wallet_state.read().await;
        for signer in signers.iter_mut() {
            let Some(address) = signer_address(scheme, chain, &signer.signer) else {
                continue;
            };
            let address = crate::send::flow::normalize_address(chain, &address);
            signer.wallet_id = state
                .wallets
                .iter()
                .filter(|wallet| wallet.id != account.id && !wallet.is_watch_only())
                .find(|wallet| {
                    wallet.chain_id == chain
                        && wallet.address_on(chain).is_some_and(|own| {
                            crate::send::flow::normalize_address(chain, own) == address
                        })
                })
                .map(|wallet| wallet.id.clone());
        }
    }
}

/// Refuse a wallet to submit through where the finished transaction is
/// submitted as it is.
fn refuse_executor(
    executor_wallet_id: &Option<String>,
    password: &Option<String>,
) -> Result<(), SpectraBridgeError> {
    if executor_wallet_id.is_some() || password.is_some() {
        return Err(SpectraBridgeError::invalid(
            "A finished transaction is submitted as it is; no wallet signs it again.",
        ));
    }
    Ok(())
}

/// The address a signer signs from, for the schemes whose signers are
/// accounts or keys of their own; `None` where a signer is a cosigner key
/// the account wallet itself holds.
fn signer_address(scheme: MultisigScheme, _chain: Chain, signer: &str) -> Option<String> {
    match scheme {
        MultisigScheme::SortedMulti | MultisigScheme::CardanoNativeScript => None,
        MultisigScheme::Safe
        | MultisigScheme::TronPermissions
        | MultisigScheme::XrplSignerList
        | MultisigScheme::StellarSigners
        | MultisigScheme::SubstrateMultisig
        | MultisigScheme::TonMultisig => Some(signer.to_string()),
        MultisigScheme::SuiMultisig => super::multisig_sui::member_address(signer),
        MultisigScheme::AptosMultiKey => super::multisig_aptos::member_address(signer),
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// The account's policy: its signers, their weights and thresholds,
    /// read from the network where the network keeps it.
    pub async fn multisig_account(
        &self,
        wallet_id: String,
    ) -> Result<MultisigAccount, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, scheme) = this.multisig_wallet(&wallet_id).await?;
            let mut account = match scheme {
                MultisigScheme::SortedMulti => super::multisig_psbt::account(&wallet)?,
                MultisigScheme::Safe => this.safe_account(&wallet).await?,
                MultisigScheme::TronPermissions => this.tron_account(&wallet).await?,
                MultisigScheme::XrplSignerList => this.xrp_account(&wallet).await?,
                MultisigScheme::StellarSigners => this.stellar_account(&wallet).await?,
                MultisigScheme::SuiMultisig => super::multisig_sui::account(&wallet)?,
                MultisigScheme::AptosMultiKey => super::multisig_aptos::account(&wallet)?,
                MultisigScheme::CardanoNativeScript => super::multisig_cardano::account(&wallet)?,
                MultisigScheme::SubstrateMultisig => super::multisig_substrate::account(&wallet)?,
                MultisigScheme::TonMultisig => this.ton_account(&wallet).await?,
            };
            for permission in &mut account.permissions {
                this.name_signer_wallets(&wallet, scheme, &mut permission.signers)
                    .await;
            }
            account.signer_wallet_ids = match scheme {
                MultisigScheme::SortedMulti => {
                    if matches!(
                        wallet.signing,
                        crate::store::state::WalletSigning::SeedPhrase { .. }
                    ) {
                        vec![wallet.id.clone()]
                    } else {
                        Vec::new()
                    }
                }
                // A cosigner's CIP-1854 key is found in any phrase wallet.
                MultisigScheme::CardanoNativeScript => this
                    .wallet_state
                    .read()
                    .await
                    .wallets
                    .iter()
                    .filter(|other| {
                        other.chain_id == wallet.chain_id
                            && matches!(
                                other.signing,
                                crate::store::state::WalletSigning::SeedPhrase { .. }
                            )
                    })
                    .map(|other| other.id.clone())
                    .collect(),
                _ => {
                    let mut ids: Vec<String> = account
                        .permissions
                        .iter()
                        .flat_map(|permission| &permission.signers)
                        .filter_map(|signer| signer.wallet_id.clone())
                        .collect();
                    ids.dedup();
                    ids.sort();
                    ids.dedup();
                    ids
                }
            };
            Ok(account)
        })
        .await
    }

    /// A session paying `spend` from the account, for its signers to sign.
    pub async fn create_multisig(
        &self,
        wallet_id: String,
        spend: MultisigSpend,
    ) -> Result<MultisigSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let (wallet, scheme) = this.multisig_wallet(&wallet_id).await?;
            // A memo only where the network's payments carry one.
            let mut spend = spend;
            spend.memo = spend
                .memo
                .as_ref()
                .map(|memo| memo.validated(wallet.chain_id))
                .transpose()?;
            let body = match scheme {
                MultisigScheme::SortedMulti => this.create_psbt(&wallet, &spend).await?,
                MultisigScheme::Safe => this.create_safe(&wallet, &spend).await?,
                MultisigScheme::TronPermissions => this.create_tron(&wallet, &spend).await?,
                MultisigScheme::XrplSignerList => this.create_xrp(&wallet, &spend).await?,
                MultisigScheme::StellarSigners => this.create_stellar(&wallet, &spend).await?,
                MultisigScheme::SuiMultisig => this.create_sui(&wallet, &spend).await?,
                MultisigScheme::AptosMultiKey => this.create_aptos(&wallet, &spend).await?,
                MultisigScheme::CardanoNativeScript => this.create_cardano(&wallet, &spend).await?,
                MultisigScheme::SubstrateMultisig => {
                    let open = this.multisig_open_sessions(&wallet_id).await?;
                    this.create_substrate(&wallet, &spend, &open).await?
                }
                MultisigScheme::TonMultisig => this.create_ton(&wallet, &spend).await?,
            };
            let stored = StoredSession::new(&wallet, body);
            let view = this.multisig_view(&wallet, &stored).await?;
            this.multisig_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    /// Read a session another signer or coordinator wrote, refused unless
    /// it is this account's. A copy of a transaction an open session holds
    /// joins it, its signatures with the session's.
    pub async fn import_multisig(
        &self,
        wallet_id: String,
        data: String,
    ) -> Result<MultisigSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let (wallet, scheme) = this.multisig_wallet(&wallet_id).await?;
            let open = this.multisig_open_sessions(&wallet_id).await?;
            let stored = match scheme {
                MultisigScheme::SortedMulti
                    if wallet.chain_id.utxo_multisig_script()
                        == Some(crate::derivation::multisig::UtxoMultisigScript::Sh) =>
                {
                    this.import_p2sh(&wallet, open, data.trim()).await?
                }
                MultisigScheme::SortedMulti => {
                    super::multisig_psbt::import(&wallet, open, data.trim())?
                }
                MultisigScheme::Safe => this.import_safe(&wallet, open, data.trim()).await?,
                MultisigScheme::TronPermissions => {
                    this.import_tron(&wallet, open, data.trim()).await?
                }
                MultisigScheme::XrplSignerList => {
                    this.import_xrp(&wallet, open, data.trim()).await?
                }
                MultisigScheme::StellarSigners => {
                    this.import_stellar(&wallet, open, data.trim()).await?
                }
                MultisigScheme::SuiMultisig => this.import_sui(&wallet, open, data.trim())?,
                MultisigScheme::AptosMultiKey => this.import_aptos(&wallet, open, data.trim())?,
                MultisigScheme::CardanoNativeScript => {
                    this.import_cardano(&wallet, open, data.trim()).await?
                }
                MultisigScheme::SubstrateMultisig => {
                    this.import_substrate(&wallet, open, data.trim()).await?
                }
                MultisigScheme::TonMultisig => this.import_ton(&wallet, open, data.trim()).await?,
            };
            let view = this.multisig_view(&wallet, &stored).await?;
            this.multisig_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    /// Sign session `session_id`, once it is still the one reviewed
    /// (`review_digest`): as the account's own cosigner key where the
    /// scheme keeps cosigner keys in the account wallet, or as
    /// `signer_wallet_id`, one of the policy's signers, elsewhere.
    /// `password` unlocks whichever wallet signs.
    pub async fn sign_multisig(
        &self,
        session_id: String,
        review_digest: String,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<MultisigSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let mut stored = this.multisig_load(&session_id).await?;
            if stored.submitted_txid.is_some() {
                return Err(SpectraBridgeError::invalid(
                    "This session was already submitted.",
                ));
            }
            let (wallet, scheme) = this.multisig_wallet(&stored.wallet_id).await?;
            let reviewed = this.multisig_view(&wallet, &stored).await?;
            if reviewed.review_digest != review_digest.trim() {
                return Err(SpectraBridgeError::invalid(
                    "The session is not the one reviewed; review it again.",
                ));
            }
            match scheme {
                MultisigScheme::SortedMulti => {
                    if signer_wallet_id.is_some_and(|id| id != wallet.id) {
                        return Err(SpectraBridgeError::invalid(
                            "A cosigner signs with the phrase added to the multisig wallet itself.",
                        ));
                    }
                    this.sign_psbt(&wallet, &mut stored, password).await?;
                }
                MultisigScheme::Safe => {
                    this.sign_safe(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::TronPermissions => {
                    this.sign_tron(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::XrplSignerList => {
                    this.sign_xrp(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::StellarSigners => {
                    this.sign_stellar(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::SuiMultisig => {
                    this.sign_sui(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::AptosMultiKey => {
                    this.sign_aptos(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::CardanoNativeScript => {
                    this.sign_cardano(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::SubstrateMultisig => {
                    this.sign_substrate(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
                MultisigScheme::TonMultisig => {
                    this.sign_ton(&wallet, &mut stored, signer_wallet_id, password)
                        .await?;
                }
            }
            let view = this.multisig_view(&wallet, &stored).await?;
            this.multisig_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    pub async fn multisig_session(
        &self,
        session_id: String,
    ) -> Result<MultisigSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let stored = this.multisig_load(&session_id).await?;
            let (wallet, _) = this.multisig_wallet(&stored.wallet_id).await?;
            this.multisig_view(&wallet, &stored).await
        })
        .await
    }

    /// The account's sessions, oldest first.
    pub async fn multisig_sessions(
        &self,
        wallet_id: String,
    ) -> Result<Vec<MultisigSession>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let (wallet, _) = this.multisig_wallet(&wallet_id).await?;
            let mut sessions = Vec::new();
            for stored in this.multisig_stored_for_wallet(&wallet_id).await? {
                sessions.push(this.multisig_view(&wallet, &stored).await?);
            }
            Ok(sessions)
        })
        .await
    }

    /// The finished transaction, as its network takes it, once enough
    /// signers signed. Nothing is submitted.
    pub async fn finalize_multisig(
        &self,
        session_id: String,
    ) -> Result<String, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let stored = this.multisig_load(&session_id).await?;
            let (wallet, _) = this.multisig_wallet(&stored.wallet_id).await?;
            match &stored.body {
                SessionBody::Psbt { psbt } => super::multisig_psbt::finalize(&wallet, psbt),
                SessionBody::P2sh(spend) => super::multisig_psbt::finalize_p2sh(&wallet, spend),
                SessionBody::Safe(session) => super::multisig_safe::finalize(&wallet, session),
                SessionBody::Sui(session) => {
                    let (transaction, signature) = Self::finalize_sui(&wallet, session)?;
                    Ok(json!({ "txBytes": transaction, "signature": signature }).to_string())
                }
                SessionBody::Aptos(session) => {
                    Ok(hex::encode(Self::finalize_aptos(&wallet, session)?))
                }
                SessionBody::Substrate(_) => Err(super::multisig_substrate::refuse_submission()),
                SessionBody::Ton(_) => Err(super::multisig_ton::refuse_submission()),
                SessionBody::Tron(_)
                | SessionBody::Xrp(_)
                | SessionBody::Stellar(_)
                | SessionBody::Cardano(_) => {
                    let reviewed = this.multisig_view(&wallet, &stored).await?;
                    if !reviewed.complete {
                        return Err(SpectraBridgeError::invalid(
                            "The signers' weights do not yet meet the threshold.",
                        ));
                    }
                    Ok(reviewed.data)
                }
            }
        })
        .await
    }

    /// Submit the session's transaction once enough signers signed, its
    /// state first read again from the network. `executor_wallet_id` and
    /// `password` name and unlock the signer whose own transaction carries
    /// it, where the scheme submits through one.
    pub async fn submit_multisig(
        &self,
        session_id: String,
        executor_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<MultisigSession, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let mut stored = this.multisig_load(&session_id).await?;
            if stored.submitted_txid.is_some() {
                return Err(SpectraBridgeError::invalid(
                    "This session was already submitted.",
                ));
            }
            let (wallet, _) = this.multisig_wallet(&stored.wallet_id).await?;
            let txid = match &stored.body {
                SessionBody::Psbt { .. } | SessionBody::P2sh(_) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.broadcast_psbt(&wallet, &stored.body).await?
                }
                SessionBody::Safe(session) => {
                    this.execute_safe(&wallet, session, executor_wallet_id, password)
                        .await?
                }
                SessionBody::Tron(session) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.broadcast_tron(&wallet, session).await?
                }
                SessionBody::Xrp(session) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.submit_xrp(&wallet, session).await?
                }
                SessionBody::Stellar(session) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.submit_stellar(&wallet, session).await?
                }
                SessionBody::Sui(session) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.execute_sui(&wallet, session).await?
                }
                SessionBody::Aptos(session) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.submit_aptos(&wallet, session).await?
                }
                SessionBody::Cardano(session) => {
                    refuse_executor(&executor_wallet_id, &password)?;
                    this.submit_cardano(&wallet, session).await?
                }
                SessionBody::Substrate(_) => {
                    return Err(super::multisig_substrate::refuse_submission());
                }
                SessionBody::Ton(_) => {
                    return Err(super::multisig_ton::refuse_submission());
                }
            };
            stored.submitted_txid = Some(txid);
            let view = this.multisig_view(&wallet, &stored).await?;
            this.multisig_save(&stored).await?;
            Ok(view)
        })
        .await
    }

    /// Forget a session; what it would spend is free for another.
    pub async fn discard_multisig(&self, session_id: String) -> Result<(), SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let db = this.bound_database().await?;
            tokio::task::spawn_blocking(move || {
                crate::wallet_db::multisig_session_delete(&db, &session_id)
            })
            .await
            .map_err(SpectraBridgeError::failure)??;
            Ok(())
        })
        .await
    }
}
