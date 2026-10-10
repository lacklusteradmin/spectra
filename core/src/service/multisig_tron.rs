//! A Tron account's permissions and the transactions several of its keys
//! sign.
//!
//! Every Tron wallet reads its account's permissions. An ordinary send is
//! signed under the permission the wallet's key meets alone (the owner's,
//! else an active one covering the contract), read at build and again
//! before signing, and refused, saying why, when none is: the usual sign of
//! a permission hijack, which the network otherwise reports only at
//! broadcast.
//!
//! A session is a transaction under the permission that covers it, with an
//! explicit expiration at most a day out: each key's signature gathered as
//! data, checked against the permission, and the summed weights checked
//! before broadcast. The permission is read again before every signature
//! and the broadcast, and a session whose permission changed is refused.
//! Changing permissions (`AccountPermissionUpdate`) is not done here.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::api::tron_http::TronAccountPolicy;
use crate::send::tron_multisig::{self as tron, TRANSFER_CONTRACT, TronPayment, TronPermission};
use crate::store::state::WalletState;

/// How long a session gathers signatures unless asked otherwise.
const DEFAULT_LIFETIME_SECS: u64 = 60 * 60;

/// A session's content: the permission its transaction names as the
/// account held it, the network's multi-signature fee then, and the
/// transaction's `raw_data` with the signatures gathered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TronSession {
    pub permission: TronPermission,
    pub multi_sign_fee_sun: u64,
    pub raw: String,
    pub signatures: Vec<String>,
}

fn account_address(wallet: &WalletState) -> Result<String, SpectraBridgeError> {
    Ok(wallet
        .address_on(wallet.chain_id)
        .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?
        .to_string())
}

/// The contract types an active permission covers, by name, for display.
fn covered(permission: &TronPermission) -> Vec<String> {
    if permission.operations.is_none() {
        return Vec::new();
    }
    (0..256u64)
        .filter(|kind| permission.covers(*kind))
        .map(|kind| match kind {
            tron::TRANSFER_CONTRACT => "TransferContract".to_string(),
            tron::TRANSFER_ASSET_CONTRACT => "TransferAssetContract".to_string(),
            tron::TRIGGER_SMART_CONTRACT => "TriggerSmartContract".to_string(),
            other => format!("contract type {other}"),
        })
        .collect()
}

fn signers(permission: &TronPermission, signed: &[String]) -> Vec<MultisigSigner> {
    permission
        .keys
        .iter()
        .map(|key| MultisigSigner {
            signer: key.address.clone(),
            weight: key.weight,
            signed: signed.contains(&key.address),
            wallet_id: None,
        })
        .collect()
}

fn signature_bytes(session: &TronSession) -> Result<Vec<Vec<u8>>, SpectraBridgeError> {
    session
        .signatures
        .iter()
        .map(|signature| hex::decode(signature).map_err(SpectraBridgeError::from))
        .collect()
}

/// The transaction as TronWeb holds one: its JSON, written from its bytes,
/// with the signatures gathered.
fn transaction_json(
    decoded: &tron::DecodedTronTransaction,
    session: &TronSession,
) -> Result<serde_json::Value, SpectraBridgeError> {
    let mut body = decoded.prepared()?.body;
    body["signature"] = json!(session.signatures);
    Ok(body)
}

pub(super) fn review(
    wallet: &WalletState,
    session: &TronSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let raw = hex::decode(&session.raw)?;
    let decoded = tron::decode(&raw)?;
    if decoded.owner_address() != account_address(wallet)? {
        return Err(SpectraBridgeError::invalid(
            "This transaction spends another account.",
        ));
    }
    if decoded.reference.permission_id != session.permission.id
        || !session.permission.covers(decoded.payment.contract_type())
    {
        return Err(SpectraBridgeError::invalid(
            "The transaction names a permission that does not cover it.",
        ));
    }
    let (signed, weight) =
        tron::signed_weight(&raw, &session.permission, &signature_bytes(session)?)?;
    let (address, value, asset) = match &decoded.payment {
        TronPayment::Trx { to, amount } => (tron::base58(to), amount.to_string(), None),
        TronPayment::Trc10 {
            asset_id,
            to,
            amount,
        } => (tron::base58(to), amount.to_string(), Some(asset_id.clone())),
        TronPayment::Trc20 {
            contract,
            to,
            amount,
            ..
        } => (
            tron::base58(to),
            amount.to_string(),
            Some(tron::base58(contract)),
        ),
    };
    let txid = hex::encode(tron::transaction_id(&raw));
    Ok(SessionReview {
        digest: txid.clone(),
        transaction_id: txid,
        threshold: session.permission.threshold,
        signers: signers(&session.permission, &signed),
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address,
            value,
            is_change: false,
            data: None,
            asset,
            memo: None,
        }],
        fee: session.multi_sign_fee_sun.to_string(),
        sequence: None,
        expires_at: Some(decoded.reference.expiration / 1000),
        expires_at_height: None,
        complete: weight >= session.permission.threshold,
        data: transaction_json(&decoded, session)?.to_string(),
    })
}

fn now_ms() -> u64 {
    (crate::store::now_unix() * 1000.0) as u64
}

impl WalletService {
    async fn tron_policy(
        &self,
        chain: Chain,
        address: &str,
        multi_sign: bool,
    ) -> Result<TronAccountPolicy, SpectraBridgeError> {
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        Ok(TronHttpClient::new(endpoints)
            .fetch_permissions(chain, address, multi_sign)
            .await?)
    }

    /// The permission `sender`'s key signs a contract of `contract_type`
    /// alone, read from the network; refused, saying why, when its key
    /// meets none.
    pub(super) async fn tron_permission_alone(
        &self,
        chain: Chain,
        sender: &str,
        contract_type: u64,
    ) -> Result<u8, SpectraBridgeError> {
        let policy = self.tron_policy(chain, sender, false).await?;
        policy
            .permissions
            .alone(sender, contract_type)
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "This wallet's key cannot send from the account alone: its owner permission needs weight %@ and the key holds %@. Spend from it through its signers' Multisig page.",
                    [
                        policy.permissions.owner.threshold,
                        policy.permissions.owner.weight_of(sender),
                    ],
                )
            })
    }

    /// Before an ordinary send signs: the permission its transaction names
    /// still covers it and the wallet's key still meets it alone.
    pub(super) async fn validate_tron_permission(
        &self,
        chain: Chain,
        prepared: &crate::send::tron::PreparedTronTransfer,
        sender: &str,
    ) -> Result<(), SpectraBridgeError> {
        let decoded = tron::decode(&prepared.raw)?;
        let policy = self.tron_policy(chain, sender, false).await?;
        let holds = policy
            .permissions
            .by_id(decoded.reference.permission_id)
            .is_some_and(|permission| {
                permission.covers(decoded.payment.contract_type())
                    && permission.weight_of(sender) >= permission.threshold
            });
        if !holds {
            return Err(SpectraBridgeError::invalid(
                "The account's permissions changed since review: this wallet's key no longer signs for it alone. Build the send again.",
            ));
        }
        Ok(())
    }

    pub(super) async fn tron_account(
        &self,
        wallet: &WalletState,
    ) -> Result<MultisigAccount, SpectraBridgeError> {
        let address = account_address(wallet)?;
        let policy = self.tron_policy(wallet.chain_id, &address, false).await?;
        let mut warnings = Vec::new();
        if !wallet.is_watch_only()
            && policy
                .permissions
                .alone(&address, TRANSFER_CONTRACT)
                .is_none()
        {
            warnings.push(
                "This wallet's key alone no longer meets any permission that can send TRX from the account.".into(),
            );
        }
        Ok(MultisigAccount {
            wallet_id: wallet.id.clone(),
            chain: wallet.chain_id,
            scheme: MultisigScheme::TronPermissions,
            address,
            permissions: policy
                .permissions
                .all()
                .map(|permission| MultisigPermission {
                    name: format!("{} ({})", permission.name, permission.id),
                    threshold: permission.threshold,
                    signers: signers(permission, &[]),
                    covers: covered(permission),
                })
                .collect(),
            warnings,
            submission: MultisigScheme::TronPermissions.submission(),
            signer_wallet_ids: Vec::new(),
        })
    }

    /// A TRX payment under the permission that covers it, expiring
    /// `spend.expires_in_secs` (at most a day) from now.
    pub(super) async fn create_tron(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let address = account_address(wallet)?;
        let to = spend.to_address.trim();
        if !crate::send::flow::is_valid_send_address(chain, to.to_string()) {
            return Err(SpectraBridgeError::refused(
                "Not an address on %@: %@",
                [chain.chain_display_name(), to],
            ));
        }
        let lifetime_secs = spend.expires_in_secs.unwrap_or(DEFAULT_LIFETIME_SECS);
        if lifetime_secs == 0 || lifetime_secs * 1000 > tron::MAX_LIFETIME_MS {
            return Err(SpectraBridgeError::invalid(
                "A Tron transaction expires within a day of being built.",
            ));
        }
        let amount =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let policy = self.tron_policy(chain, &address, true).await?;
        let permission = policy.permissions.for_contract(TRANSFER_CONTRACT).clone();
        if policy.balance_sun < amount.saturating_add(policy.multi_sign_fee_sun) {
            return Err(crate::send::error::SendError::insufficient_funds().into());
        }
        let endpoints = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let reference = TronHttpClient::new(endpoints)
            .transfer_reference_for(chain)
            .await?;
        let prepared = crate::send::tron::prepare_transfer_under(
            &address,
            crate::send::tron::Transfer::Native { to, amount },
            reference,
            permission.id,
            lifetime_secs * 1000,
        )?;
        Ok(SessionBody::Tron(TronSession {
            permission,
            multi_sign_fee_sun: policy.multi_sign_fee_sun,
            raw: hex::encode(&prepared.raw),
            signatures: Vec::new(),
        }))
    }

    /// A transaction another signer wrote, as TronWeb holds one: refused
    /// unless it spends this account under one of its permissions as they
    /// are now, each signature that permission's. A copy of a transaction
    /// an open session holds joins it.
    pub(super) async fn import_tron(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let not_one = || SpectraBridgeError::invalid("Not a Tron transaction.");
        let value: serde_json::Value = serde_json::from_str(data).map_err(|_| not_one())?;
        let raw = hex::decode(value["raw_data_hex"].as_str().ok_or_else(not_one)?)
            .map_err(|_| not_one())?;
        if value["txID"].as_str().is_some_and(|txid| {
            !txid.eq_ignore_ascii_case(&hex::encode(tron::transaction_id(&raw)))
        }) {
            return Err(SpectraBridgeError::invalid(
                "The transaction's id is not its raw data's hash.",
            ));
        }
        let signatures: Vec<String> = match value.get("signature") {
            None => Vec::new(),
            Some(list) => list
                .as_array()
                .ok_or_else(not_one)?
                .iter()
                .map(|signature| {
                    signature
                        .as_str()
                        .map(str::to_ascii_lowercase)
                        .ok_or_else(not_one)
                })
                .collect::<Result<_, _>>()?,
        };
        let decoded = tron::decode(&raw)?;
        let address = account_address(wallet)?;
        let policy = self.tron_policy(wallet.chain_id, &address, true).await?;
        let permission = policy
            .permissions
            .by_id(decoded.reference.permission_id)
            .ok_or_else(|| {
                SpectraBridgeError::invalid("The account has no permission the transaction names.")
            })?
            .clone();
        let incoming = TronSession {
            permission,
            multi_sign_fee_sun: policy.multi_sign_fee_sun,
            raw: hex::encode(&raw),
            signatures,
        };
        review(wallet, &incoming)?;
        for mut stored in open {
            let SessionBody::Tron(held) = &stored.body else {
                continue;
            };
            if held.raw != incoming.raw {
                continue;
            }
            let mut merged = held.clone();
            for signature in incoming.signatures {
                if !merged.signatures.contains(&signature) {
                    merged.signatures.push(signature);
                }
            }
            review(wallet, &merged)?;
            stored.body = SessionBody::Tron(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Tron(incoming)))
    }

    /// The account's policy now, refused unless the session's permission is
    /// still exactly what it was and the transaction has not expired.
    async fn tron_unchanged(
        &self,
        wallet: &WalletState,
        session: &TronSession,
    ) -> Result<TronAccountPolicy, SpectraBridgeError> {
        let decoded = tron::decode(&hex::decode(&session.raw)?)?;
        if decoded.reference.expiration <= now_ms() {
            return Err(SpectraBridgeError::invalid(
                "The transaction's deadline has passed; build it again.",
            ));
        }
        let policy = self
            .tron_policy(wallet.chain_id, &account_address(wallet)?, false)
            .await?;
        if policy.permissions.by_id(session.permission.id) != Some(&session.permission) {
            return Err(SpectraBridgeError::invalid(
                "The account's permission changed since this session was built; build it again.",
            ));
        }
        Ok(policy)
    }

    /// Sign the session as `signer_wallet_id`, one of its permission's keys,
    /// before its deadline and with its permission unchanged.
    pub(super) async fn sign_tron(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Tron(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a Tron session"));
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
        self.tron_unchanged(wallet, session).await?;
        let identity = self
            .resolve_send_identity(wallet.chain_id, &signer_wallet_id, password.as_deref())
            .await?;
        if session.permission.weight_of(&identity.from_address) == 0 {
            return Err(SpectraBridgeError::refused(
                "%@ holds no weight in the permission this transaction names.",
                [signer.name.as_str()],
            ));
        }
        let raw = hex::decode(&session.raw)?;
        let (signed, _) =
            tron::signed_weight(&raw, &session.permission, &signature_bytes(session)?)?;
        if signed.contains(&identity.from_address) {
            return Err(SpectraBridgeError::invalid(
                "This key already signed the session.",
            ));
        }
        let key = super::decode_private_key(&identity.private_key_hex)?;
        let signature = tron::sign(&raw, &key)?;
        if tron::signer(&raw, &signature)? != identity.from_address {
            return Err(SpectraBridgeError::failure("signature does not recover"));
        }
        let mut session = session.clone();
        session.signatures.push(hex::encode(signature));
        stored.body = SessionBody::Tron(session);
        Ok(())
    }

    /// Broadcast the session's transaction once its signers' weights meet
    /// the permission's threshold, its deadline not passed and its
    /// permission unchanged. The transaction id.
    pub(super) async fn broadcast_tron(
        &self,
        wallet: &WalletState,
        session: &TronSession,
    ) -> Result<String, SpectraBridgeError> {
        self.tron_unchanged(wallet, session).await?;
        let reviewed = review(wallet, session)?;
        if !reviewed.complete {
            return Err(SpectraBridgeError::invalid(
                "The signers' weights do not yet meet the permission's threshold.",
            ));
        }
        let txid = self
            .broadcast_raw_extract(wallet.chain_id, reviewed.data, "txid".into())
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
#[path = "tests/multisig_tron.rs"]
mod multisig_tron_tests;
