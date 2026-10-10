//! A Stellar account's signers and thresholds, and the payments they sign
//! together.
//!
//! Every Stellar wallet reads its account's signers. An ordinary send
//! signs with the account's master key, so it is refused before signing,
//! saying why, when that key's weight no longer meets the threshold the
//! operation needs: medium for a payment or a trustline, high for a merge.
//! A session is a payment of lumens whose sequence number is fixed when it
//! is built and whose time bounds are its deadline: each signer's signature
//! is gathered as data and checked against the account's keys, and the
//! summed weights against the medium threshold before submission. The
//! signers are read again before every signature and the submission; a
//! session built under other signers, past its deadline or whose sequence
//! was used is refused. Adding or removing signers (`SetOptions`) is not
//! done here.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::api::horizon::StellarAccountSigners;
use crate::send::stellar_multisig::{self as stellar, OwnedMemo, StellarPayment, StellarSigners};
use crate::store::state::WalletState;
use base64::Engine;

/// How long a session gathers signatures unless asked otherwise.
const DEFAULT_LIFETIME_SECS: u64 = 60 * 60;

/// A session's content: the account's signers as it held them, and the
/// envelope with the signatures gathered, base64.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StellarSession {
    pub signers: StellarSigners,
    pub envelope: String,
}

/// The operation an ordinary send's master key signs needs this threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StellarThreshold {
    Medium,
    High,
}

fn account_address(wallet: &WalletState) -> Result<String, SpectraBridgeError> {
    Ok(wallet
        .address_on(wallet.chain_id)
        .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
        .to_string())
}

fn signer_views(signers: &StellarSigners, signed: &[String]) -> Vec<MultisigSigner> {
    signers
        .keys
        .iter()
        .chain(&signers.other)
        .map(|(key, weight)| MultisigSigner {
            signer: key.clone(),
            weight: *weight,
            signed: signed.contains(key),
            wallet_id: None,
        })
        .collect()
}

fn engine() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn decoded(
    session: &StellarSession,
) -> Result<(StellarPayment, Vec<stellar::StellarSignature>), SpectraBridgeError> {
    let bytes = engine()
        .decode(session.envelope.trim())
        .map_err(|_| SpectraBridgeError::invalid("Not a Stellar transaction envelope."))?;
    Ok(stellar::decode(&bytes)?)
}

pub(super) fn review(
    wallet: &WalletState,
    session: &StellarSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let chain = wallet.chain_id;
    let passphrase = chain.stellar_network_passphrase()?;
    let (payment, signatures) = decoded(session)?;
    if stellar::address_of(&payment.source) != account_address(wallet)? {
        return Err(SpectraBridgeError::invalid(
            "This payment spends another account.",
        ));
    }
    let (signed, weight) =
        stellar::signed_weight(&payment, passphrase, &session.signers, &signatures)?;
    let hash = hex::encode(payment.hash(passphrase)?);
    Ok(SessionReview {
        transaction_id: hash.clone(),
        digest: hash,
        threshold: session.signers.payment_threshold(),
        signers: signer_views(&session.signers, &signed),
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address: stellar::address_of(&payment.destination),
            value: payment.stroops.to_string(),
            is_change: false,
            data: None,
            asset: None,
            memo: payment.memo.as_ref().map(OwnedMemo::text),
        }],
        fee: payment.fee.to_string(),
        sequence: Some(payment.sequence.to_string()),
        expires_at: Some(payment.max_time),
        expires_at_height: None,
        complete: weight >= session.signers.payment_threshold(),
        data: engine().encode(payment.envelope(&signatures)?),
    })
}

impl WalletService {
    async fn stellar_signers(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<Option<StellarAccountSigners>, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(HorizonClient::new(endpoints)
            .fetch_account_signers(chain, address)
            .await?)
    }

    /// Before an ordinary send builds or signs: the account's master key,
    /// which the wallet holds, still meets the threshold its operation
    /// needs alone.
    pub(super) async fn validate_stellar_master_key(
        &self,
        chain: Chain,
        sender: &str,
        needs: StellarThreshold,
    ) -> Result<(), SpectraBridgeError> {
        let Some(account) = self.stellar_signers(chain, sender).await? else {
            return Ok(());
        };
        let signers = &account.signers;
        let threshold = match needs {
            StellarThreshold::Medium => signers.medium,
            StellarThreshold::High => signers.high,
        }
        .max(1);
        let weight = signers.weight_of(sender);
        if weight < threshold {
            return Err(SpectraBridgeError::refused(
                "This wallet's key cannot send from the account alone: the operation needs weight %@ and the key holds %@. Spend from it through its signers' Multisig page.",
                [threshold, weight],
            ));
        }
        Ok(())
    }

    pub(super) async fn stellar_account(
        &self,
        wallet: &WalletState,
    ) -> Result<MultisigAccount, SpectraBridgeError> {
        let address = account_address(wallet)?;
        let account = self
            .stellar_signers(wallet.chain_id, &address)
            .await?
            .ok_or_else(|| {
                SpectraBridgeError::invalid("This account is not on the network yet.")
            })?;
        let signers = &account.signers;
        let permission = |name: &str, threshold: u64| MultisigPermission {
            name: name.to_string(),
            threshold,
            signers: signer_views(signers, &[]),
            covers: Vec::new(),
        };
        let mut warnings = Vec::new();
        for (key, _) in &signers.other {
            warnings.push(crate::LocalizableMessage::new(
                "Signer %@ is a hash or a pre-authorized transaction: it counts toward the thresholds, but no wallet here signs as it.",
                [key],
            ));
        }
        if !wallet.is_watch_only() && signers.weight_of(&address) < signers.medium.max(1) {
            warnings.push(
                "This wallet's key alone no longer meets the threshold a payment needs.".into(),
            );
        }
        Ok(MultisigAccount {
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            scheme: MultisigScheme::StellarSigners,
            address,
            permissions: vec![
                permission("low threshold", signers.low),
                permission("medium threshold (payments, trustlines)", signers.medium),
                permission(
                    "high threshold (signers, thresholds, merging)",
                    signers.high,
                ),
            ],
            warnings,
            submission: MultisigScheme::StellarSigners.submission(),
            signer_wallet_ids: Vec::new(),
        })
    }

    /// A payment of lumens at the account's next sequence, valid until
    /// `spend.expires_in_secs` from now.
    pub(super) async fn create_stellar(
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
        let stroops =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| i64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let lifetime = spend
            .expires_in_secs
            .unwrap_or(DEFAULT_LIFETIME_SECS)
            .max(1);
        let account = self
            .stellar_signers(chain, &address)
            .await?
            .ok_or_else(|| {
                SpectraBridgeError::invalid("This account is not on the network yet.")
            })?;
        let horizon = HorizonClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        if horizon.fetch_account_state(to).await?.is_none() {
            return Err(SpectraBridgeError::refused(
                "%@ is not on the network, and a payment does not create an account.",
                [to],
            ));
        }
        let fee = horizon.fetch_base_fee().await?;
        let reserve = horizon.fetch_reserve_state(chain, &address).await?;
        let held = (2 + u128::from(reserve.subentries) + u128::from(reserve.sponsoring))
            .saturating_sub(u128::from(reserve.sponsored))
            * u128::from(reserve.base_reserve);
        if u128::from(reserve.balance_stroops) < stroops as u128 + u128::from(fee) + held {
            return Err(crate::send::error::SendError::insufficient_funds().into());
        }
        let payment = StellarPayment {
            source: crate::derivation::stellar::decode_stellar_address(&address)?,
            fee: u32::try_from(fee).map_err(|_| SpectraBridgeError::invalid("Fee overflow"))?,
            sequence: account
                .sequence
                .checked_add(1)
                .ok_or_else(|| SpectraBridgeError::failure("Sequence exhausted"))?,
            min_time: 0,
            max_time: crate::store::now_unix() as u64 + lifetime,
            memo: OwnedMemo::of(crate::send::payment_memo::PaymentMemo::stellar(
                spend.memo.as_ref(),
            )?),
            destination: crate::derivation::stellar::decode_stellar_address(to)?,
            stroops,
        };
        Ok(SessionBody::Stellar(StellarSession {
            signers: account.signers,
            envelope: engine().encode(payment.envelope(&[])?),
        }))
    }

    /// An envelope another signer wrote: refused unless it pays from this
    /// account, each signature one of its signers' as they are now. A copy
    /// of a payment an open session holds joins it.
    pub(super) async fn import_stellar(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let account = self
            .stellar_signers(wallet.chain_id, &account_address(wallet)?)
            .await?
            .ok_or_else(|| {
                SpectraBridgeError::invalid("This account is not on the network yet.")
            })?;
        let incoming = StellarSession {
            signers: account.signers,
            envelope: data.trim().to_string(),
        };
        review(wallet, &incoming)?;
        let (payment, read) = decoded(&incoming)?;
        let passphrase = wallet.chain_id.stellar_network_passphrase()?;
        for mut stored in open {
            let SessionBody::Stellar(held) = &stored.body else {
                continue;
            };
            let (held_payment, mut signatures) = decoded(held)?;
            if held_payment != payment {
                continue;
            }
            let (known, _) =
                stellar::signed_weight(&payment, passphrase, &held.signers, &signatures)?;
            for signature in read {
                let (signer, _) = stellar::signed_weight(
                    &payment,
                    passphrase,
                    &held.signers,
                    std::slice::from_ref(&signature),
                )?;
                if !known.contains(&signer[0]) && !signatures.contains(&signature) {
                    signatures.push(signature);
                }
            }
            let merged = StellarSession {
                signers: held.signers.clone(),
                envelope: engine().encode(payment.envelope(&signatures)?),
            };
            review(wallet, &merged)?;
            stored.body = SessionBody::Stellar(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Stellar(incoming)))
    }

    /// The account now, refused unless its signers and thresholds are the
    /// session's, its sequence is the one before the payment's and the
    /// deadline has not passed.
    async fn stellar_unchanged(
        &self,
        wallet: &WalletState,
        session: &StellarSession,
    ) -> Result<(), SpectraBridgeError> {
        let (payment, _) = decoded(session)?;
        if crate::store::now_unix() as u64 >= payment.max_time {
            return Err(SpectraBridgeError::invalid(
                "The payment's deadline has passed; build it again.",
            ));
        }
        let account = self
            .stellar_signers(wallet.chain_id, &account_address(wallet)?)
            .await?
            .ok_or_else(|| {
                SpectraBridgeError::invalid("This account is not on the network yet.")
            })?;
        if account.signers != session.signers {
            return Err(SpectraBridgeError::invalid(
                "The account's signers or thresholds changed since this session was built; build it again.",
            ));
        }
        if account.sequence.checked_add(1) != Some(payment.sequence) {
            return Err(SpectraBridgeError::invalid(
                "The account's sequence moved on: another transaction used this one's. Build it again.",
            ));
        }
        Ok(())
    }

    /// Sign the session as `signer_wallet_id`, one of the account's keys.
    pub(super) async fn sign_stellar(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Stellar(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a Stellar session"));
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
        self.stellar_unchanged(wallet, session).await?;
        let identity = self
            .resolve_send_identity(wallet.chain_id, &signer_wallet_id, password.as_deref())
            .await?;
        if session.signers.weight_of(&identity.from_address) == 0 {
            return Err(SpectraBridgeError::refused(
                "%@ is not one of the account's signers.",
                [signer.name.as_str()],
            ));
        }
        let passphrase = wallet.chain_id.stellar_network_passphrase()?;
        let (payment, mut signatures) = decoded(session)?;
        let (signed, _) =
            stellar::signed_weight(&payment, passphrase, &session.signers, &signatures)?;
        if signed.contains(&identity.from_address) {
            return Err(SpectraBridgeError::invalid(
                "This signer already signed the session.",
            ));
        }
        let bytes = zeroize::Zeroizing::new(hex::decode(identity.private_key_hex.as_str())?);
        let seed: &[u8; 32] = bytes
            .get(..32)
            .and_then(|seed| seed.try_into().ok())
            .ok_or_else(|| SpectraBridgeError::failure("Invalid Stellar seed"))?;
        let signature = stellar::sign(&payment, passphrase, seed)?;
        signatures.push(signature);
        let (signed, _) =
            stellar::signed_weight(&payment, passphrase, &session.signers, &signatures)?;
        if !signed.contains(&identity.from_address) {
            return Err(SpectraBridgeError::failure("signature does not verify"));
        }
        stored.body = SessionBody::Stellar(StellarSession {
            signers: session.signers.clone(),
            envelope: engine().encode(payment.envelope(&signatures)?),
        });
        Ok(())
    }

    /// Submit the session once its signers' weights meet the medium
    /// threshold, its signers unchanged, its sequence unused and its
    /// deadline not passed. The transaction hash.
    pub(super) async fn submit_stellar(
        &self,
        wallet: &WalletState,
        session: &StellarSession,
    ) -> Result<String, SpectraBridgeError> {
        self.stellar_unchanged(wallet, session).await?;
        let reviewed = review(wallet, session)?;
        if !reviewed.complete {
            return Err(SpectraBridgeError::invalid(
                "The signers' weights do not yet meet the account's payment threshold.",
            ));
        }
        let hash = self
            .broadcast_raw_extract(
                wallet.chain_id,
                json!({ "signed_xdr_b64": reviewed.data }).to_string(),
                "txid".into(),
            )
            .await?;
        if !hash.is_empty() && !hash.eq_ignore_ascii_case(&reviewed.transaction_id) {
            return Err(SpectraBridgeError::failure(
                "The network accepted another transaction hash than the session's",
            ));
        }
        Ok(reviewed.transaction_id)
    }
}

#[cfg(test)]
#[path = "tests/multisig_stellar.rs"]
mod multisig_stellar_tests;
