//! An XRP Ledger account's signer list and the payments it authorizes.
//!
//! Every XRP wallet reads its account's policy. An ordinary send signs with
//! the account's master key, so it is refused before signing, saying why,
//! once that key is disabled. A session is a payment of XRP every field of
//! which is fixed before the first signature: its sequence the account's
//! next, its fee the base fee for every signer the list holds, and its
//! `LastLedgerSequence` the deadline for gathering signatures. The list is
//! read again before every signature and the submission; a session built
//! under another list, past its ledger, or whose sequence was used is
//! refused. Setting a signer list (`SignerListSet`) is not done here.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::api::xrpl_json_rpc::XrpAccountPolicy;
use crate::send::xrp_multisig::{self as xrp, XrpPayment, XrpSignerList};
use crate::store::state::WalletState;

/// How long a session gathers signatures unless asked otherwise.
const DEFAULT_LIFETIME_SECS: u64 = 60 * 60;
/// What one ledger takes to close, about: how a lifetime in seconds
/// becomes a `LastLedgerSequence`.
const LEDGER_SECONDS: u64 = 4;

/// A session's content: the signer list as the account held it, and the
/// payment with the signatures gathered, as a blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XrpSession {
    pub list: XrpSignerList,
    pub blob: String,
}

fn account_address(wallet: &WalletState) -> Result<String, SpectraBridgeError> {
    Ok(wallet
        .address_on(wallet.chain_id)
        .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
        .to_string())
}

fn signers(list: &XrpSignerList, signed: &[String]) -> Vec<MultisigSigner> {
    list.entries
        .iter()
        .map(|(address, weight)| MultisigSigner {
            signer: address.clone(),
            weight: *weight,
            signed: signed.contains(address),
            wallet_id: None,
        })
        .collect()
}

/// The open ledger the policy was read at, which a deadline counts from.
fn ledger(policy: &XrpAccountPolicy) -> Result<u32, SpectraBridgeError> {
    policy
        .ledger
        .ok_or_else(|| SpectraBridgeError::failure("The node did not say which ledger it read"))
}

/// The account's next sequence, as the node read it.
fn sequence(policy: &XrpAccountPolicy) -> Result<u32, SpectraBridgeError> {
    policy
        .sequence
        .ok_or_else(|| SpectraBridgeError::failure("The node gave no valid account sequence"))
}

fn decoded(session: &XrpSession) -> Result<(XrpPayment, Vec<xrp::XrpSigner>), SpectraBridgeError> {
    Ok(xrp::decode(&hex::decode(&session.blob)?)?)
}

pub(super) fn review(
    wallet: &WalletState,
    session: &XrpSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let (payment, read) = decoded(session)?;
    if payment.account != xrp::account_id(&account_address(wallet)?)? {
        return Err(SpectraBridgeError::invalid(
            "This payment spends another account.",
        ));
    }
    let (signed, weight) = xrp::signed_weight(&payment, &session.list, &read)?;
    let blob = payment.blob(&read)?;
    Ok(SessionReview {
        transaction_id: hex::encode_upper(XrpPayment::hash(&blob)),
        digest: hex::encode(payment.digest()?),
        threshold: session.list.quorum,
        signers: signers(&session.list, &signed),
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address: crate::derivation::xrp::address_of_account_id(&payment.destination)?,
            value: payment.amount_drops.to_string(),
            is_change: false,
            data: None,
            asset: None,
            memo: payment.destination_tag.map(|tag| tag.to_string()),
        }],
        fee: payment.fee_drops.to_string(),
        sequence: Some(payment.sequence.to_string()),
        expires_at: None,
        expires_at_height: Some(u64::from(payment.last_ledger_sequence)),
        complete: weight >= session.list.quorum,
        data: hex::encode_upper(blob),
    })
}

impl WalletService {
    async fn xrp_policy(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<XrpAccountPolicy, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(XrplClient::new(endpoints)
            .fetch_account_policy(chain, address, false)
            .await?)
    }

    /// Before an ordinary send builds or signs: the account's master key,
    /// which the wallet holds, still signs for it.
    pub(super) async fn validate_xrp_master_key(
        &self,
        chain: Chain,
        sender: &str,
    ) -> Result<(), SpectraBridgeError> {
        if self.xrp_policy(chain, sender).await?.master_disabled {
            return Err(SpectraBridgeError::invalid(
                "This account's master key is disabled, so this wallet's key no longer signs for it. Spend from it through its signers' Multisig page.",
            ));
        }
        Ok(())
    }

    pub(super) async fn xrp_account(
        &self,
        wallet: &WalletState,
    ) -> Result<MultisigAccount, SpectraBridgeError> {
        let address = account_address(wallet)?;
        let policy = self.xrp_policy(wallet.chain_id, &address).await?;
        let alone = |name: &str, signer: &str| MultisigPermission {
            name: name.to_string(),
            threshold: 1,
            signers: vec![MultisigSigner {
                signer: signer.to_string(),
                weight: 1,
                signed: false,
                wallet_id: None,
            }],
            covers: Vec::new(),
        };
        let mut permissions = Vec::new();
        if !policy.master_disabled {
            permissions.push(alone("master key", &address));
        }
        if let Some(regular) = &policy.regular_key {
            permissions.push(alone("regular key", regular));
        }
        if let Some(list) = &policy.signer_list {
            permissions.push(MultisigPermission {
                name: "signer list".into(),
                threshold: list.quorum,
                signers: signers(list, &[]),
                covers: Vec::new(),
            });
        }
        let mut warnings = Vec::new();
        if policy.master_disabled && !wallet.is_watch_only() {
            warnings.push(
                "The account's master key is disabled: this wallet's key no longer signs for it."
                    .into(),
            );
        }
        Ok(MultisigAccount {
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            scheme: MultisigScheme::XrplSignerList,
            address,
            permissions,
            warnings,
            submission: MultisigScheme::XrplSignerList.submission(),
            signer_wallet_ids: Vec::new(),
        })
    }

    /// A payment of XRP the account's signer list authorizes, gathering
    /// signatures until about `spend.expires_in_secs` from now.
    pub(super) async fn create_xrp(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let address = account_address(wallet)?;
        let to = spend.to_address.trim();
        if !crate::send::flow::is_valid_send_address(chain, to.to_string()) || to == address {
            return Err(SpectraBridgeError::refused(
                "Not an address on %@: %@",
                [chain.chain_display_name(), to],
            ));
        }
        self.require_payment_memo(chain, to, spend.memo.as_ref())
            .await?;
        let lifetime = spend
            .expires_in_secs
            .unwrap_or(DEFAULT_LIFETIME_SECS)
            .max(1);
        let amount_drops =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        crate::send::xrp::validate_drops(u128::from(amount_drops))?;
        let policy = XrplClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        )
        .fetch_account_policy(chain, &address, true)
        .await?;
        let list = policy.signer_list.clone().ok_or_else(|| {
            SpectraBridgeError::invalid(
                "This account has no signer list: its own key signs for it alone.",
            )
        })?;
        // One base fee for the transaction and one for each signature it
        // can carry.
        let fee_drops = policy
            .fee_drops
            .unwrap_or_default()
            .checked_mul(list.entries.len() as u64 + 1)
            .ok_or_else(|| SpectraBridgeError::invalid("Fee overflow"))?;
        let reserve = XrplClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        )
        .fetch_reserve_state(chain, &address)
        .await?;
        let held = u128::from(reserve.reserve_base)
            + u128::from(reserve.owner_count) * u128::from(reserve.reserve_increment);
        if u128::from(policy.balance_drops.unwrap_or(0))
            < u128::from(amount_drops) + u128::from(fee_drops) + held
        {
            return Err(crate::send::error::SendError::insufficient_funds().into());
        }
        let last_ledger_sequence =
            u32::try_from(u64::from(ledger(&policy)?) + lifetime.div_ceil(LEDGER_SECONDS))
                .map_err(|_| {
                    SpectraBridgeError::invalid("The deadline is past the ledger's range.")
                })?;
        let payment = XrpPayment {
            account: xrp::account_id(&address)?,
            destination: xrp::account_id(to)?,
            destination_tag: crate::send::payment_memo::PaymentMemo::destination_tag(
                spend.memo.as_ref(),
            )?,
            amount_drops,
            fee_drops,
            sequence: sequence(&policy)?,
            last_ledger_sequence,
        };
        Ok(SessionBody::Xrp(XrpSession {
            list,
            blob: hex::encode_upper(payment.blob(&[])?),
        }))
    }

    /// A blob another signer wrote: refused unless it pays from this
    /// account, each signature its list's as the list is now. A copy of a
    /// payment an open session holds joins it.
    pub(super) async fn import_xrp(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let blob = hex::decode(data.trim())
            .map_err(|_| SpectraBridgeError::invalid("Not an XRP transaction blob."))?;
        let (payment, read) = xrp::decode(&blob)?;
        let policy = self
            .xrp_policy(wallet.chain_id, &account_address(wallet)?)
            .await?;
        let list = policy.signer_list.ok_or_else(|| {
            SpectraBridgeError::invalid(
                "This account has no signer list: its own key signs for it alone.",
            )
        })?;
        let incoming = XrpSession {
            list,
            blob: hex::encode_upper(&blob),
        };
        review(wallet, &incoming)?;
        for mut stored in open {
            let SessionBody::Xrp(held) = &stored.body else {
                continue;
            };
            let (held_payment, mut signers) = decoded(held)?;
            if held_payment != payment {
                continue;
            }
            for signer in &read {
                if signers.iter().all(|known| known.account != signer.account) {
                    signers.push(signer.clone());
                }
            }
            let merged = XrpSession {
                list: held.list.clone(),
                blob: hex::encode_upper(payment.blob(&signers)?),
            };
            review(wallet, &merged)?;
            stored.body = SessionBody::Xrp(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Xrp(incoming)))
    }

    /// The account's policy now, refused unless the session's list is still
    /// the account's, its sequence not yet used and its ledger not passed.
    async fn xrp_unchanged(
        &self,
        wallet: &WalletState,
        session: &XrpSession,
    ) -> Result<XrpAccountPolicy, SpectraBridgeError> {
        let (payment, _) = decoded(session)?;
        let policy = self
            .xrp_policy(wallet.chain_id, &account_address(wallet)?)
            .await?;
        if policy.signer_list.as_ref() != Some(&session.list) {
            return Err(SpectraBridgeError::invalid(
                "The account's signer list changed since this session was built; build it again.",
            ));
        }
        if sequence(&policy)? != payment.sequence {
            return Err(SpectraBridgeError::invalid(
                "The account's sequence moved on: another transaction used this one's. Build it again.",
            ));
        }
        if ledger(&policy)? > payment.last_ledger_sequence {
            return Err(SpectraBridgeError::invalid(
                "The payment's last ledger has passed; build it again.",
            ));
        }
        Ok(policy)
    }

    /// Sign the session as `signer_wallet_id`, an account on the list,
    /// with its master key.
    pub(super) async fn sign_xrp(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Xrp(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not an XRP session"));
        };
        let signer_wallet_id = signer_wallet_id
            .ok_or_else(|| SpectraBridgeError::invalid("Name the wallet that signs."))?;
        let signer = self.stored_wallet(&signer_wallet_id).await?;
        if signer.chain_id != wallet.chain_id {
            return Err(SpectraBridgeError::refused(
                "%@ is not on %@.",
                [signer.name.as_str(), wallet.chain_id.chain_display_name()],
            ));
        }
        self.xrp_unchanged(wallet, session).await?;
        let identity = self
            .resolve_send_identity(wallet.chain_id, &signer_wallet_id, password.as_deref())
            .await?;
        if session.list.weight_of(&identity.from_address) == 0 {
            return Err(SpectraBridgeError::refused(
                "%@ is not on the account's signer list.",
                [signer.name.as_str()],
            ));
        }
        if self
            .xrp_policy(wallet.chain_id, &identity.from_address)
            .await?
            .master_disabled
        {
            return Err(SpectraBridgeError::refused(
                "%@'s master key is disabled, so it cannot sign as a signer.",
                [signer.name.as_str()],
            ));
        }
        let (payment, mut signers) = decoded(session)?;
        let own = xrp::account_id(&identity.from_address)?;
        if signers.iter().any(|known| known.account == own) {
            return Err(SpectraBridgeError::invalid(
                "This signer already signed the session.",
            ));
        }
        let key = super::decode_private_key(&identity.private_key_hex)?;
        let signed = xrp::sign(&payment, &key)?;
        if signed.account != own || !xrp::verify(&payment, &signed) {
            return Err(SpectraBridgeError::failure("signature does not verify"));
        }
        signers.push(signed);
        stored.body = SessionBody::Xrp(XrpSession {
            list: session.list.clone(),
            blob: hex::encode_upper(payment.blob(&signers)?),
        });
        Ok(())
    }

    /// Submit the session once its signers' weights meet the quorum, its
    /// list unchanged, its sequence unused, its ledger not passed, and no
    /// signer's master key disabled since it signed. The transaction id.
    pub(super) async fn submit_xrp(
        &self,
        wallet: &WalletState,
        session: &XrpSession,
    ) -> Result<String, SpectraBridgeError> {
        self.xrp_unchanged(wallet, session).await?;
        let reviewed = review(wallet, session)?;
        if !reviewed.complete {
            return Err(SpectraBridgeError::invalid(
                "The signers' weights do not yet meet the list's quorum.",
            ));
        }
        for signer in reviewed.signers.iter().filter(|signer| signer.signed) {
            if self
                .xrp_policy(wallet.chain_id, &signer.signer)
                .await?
                .master_disabled
            {
                return Err(SpectraBridgeError::refused(
                    "%@'s master key is disabled since it signed; its signature no longer counts.",
                    [signer.signer.as_str()],
                ));
            }
        }
        let txid = self
            .broadcast_raw_extract(
                wallet.chain_id,
                json!({ "tx_blob_hex": reviewed.data }).to_string(),
                "txid".into(),
            )
            .await?;
        if !txid.is_empty() && !txid.eq_ignore_ascii_case(&reviewed.transaction_id) {
            return Err(SpectraBridgeError::failure(
                "The network accepted another transaction id than the session's",
            ));
        }
        Ok(reviewed.transaction_id)
    }
}

#[cfg(test)]
#[path = "tests/multisig_xrp.rs"]
mod multisig_xrp_tests;
