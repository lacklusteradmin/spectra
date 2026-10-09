//! An Aptos MultiKey account's sessions: an APT transfer from the account
//! with its members' signatures gathered as data, verified against their
//! keys, and assembled into one MultiKey authenticator once the required
//! number have signed. The account's authentication key, its sequence and
//! the transfer's expiration are read again before a signature and before
//! submission: an account whose key was rotated away from the policy, a
//! sequence another transaction used, or a passed expiration is refused.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::derivation::aptos_multikey::{AptosKey, AptosMultiKey};
use crate::send::aptos_multikey::{self as aptos, AptosTransfer, MemberSignature};
use crate::store::state::WalletState;

/// How long a session gathers signatures unless asked otherwise.
const DEFAULT_LIFETIME_SECS: u64 = 60 * 60;

/// One member's signature, by its place among the keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AptosSignatureData {
    pub index: usize,
    /// `0x…`, 64 bytes.
    pub signature: String,
}

/// A session's content: the raw transaction, `0x…` BCS, and the members'
/// signatures.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AptosSession {
    pub transaction: String,
    pub signatures: Vec<AptosSignatureData>,
}

fn policy(wallet: &WalletState) -> Result<AptosMultiKey, SpectraBridgeError> {
    Ok(AptosMultiKey::parse(
        wallet.multisig_policy.as_deref().ok_or_else(|| {
            SpectraBridgeError::invalid("This wallet is not an Aptos MultiKey account.")
        })?,
    )?)
}

fn hex_bytes(text: &str) -> Result<Vec<u8>, SpectraBridgeError> {
    let text = text.trim();
    Ok(hex::decode(text.strip_prefix("0x").unwrap_or(text))?)
}

fn members(policy: &AptosMultiKey, signed: &[usize]) -> Vec<MultisigSigner> {
    policy
        .keys
        .iter()
        .enumerate()
        .map(|(index, key)| MultisigSigner {
            signer: key.text(),
            weight: 1,
            signed: signed.contains(&index),
            wallet_id: None,
        })
        .collect()
}

fn read(
    wallet: &WalletState,
    session: &AptosSession,
) -> Result<(AptosMultiKey, AptosTransfer, Vec<MemberSignature>), SpectraBridgeError> {
    let policy = policy(wallet)?;
    let transfer = aptos::decode(&hex_bytes(&session.transaction)?)?;
    let address = wallet
        .address_on(wallet.chain_id)
        .ok_or_else(|| SpectraBridgeError::failure("Wallet has no address on this network"))?;
    if format!("0x{}", hex::encode(transfer.sender)) != address.to_ascii_lowercase() {
        return Err(SpectraBridgeError::invalid(
            "This transaction spends another account.",
        ));
    }
    let signatures = session
        .signatures
        .iter()
        .map(|signature| {
            aptos::member_signature(
                &policy,
                &transfer,
                signature.index,
                &hex_bytes(&signature.signature)?,
            )
            .map_err(SpectraBridgeError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    aptos::signers(&signatures)?;
    Ok((policy, transfer, signatures))
}

pub(super) fn review(
    wallet: &WalletState,
    session: &AptosSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let (policy, transfer, signatures) = read(wallet, session)?;
    let signed = aptos::signers(&signatures)?;
    let complete = signed.len() >= usize::from(policy.required);
    let transaction_id = if complete {
        aptos::transaction_hash(&aptos::signed_transaction(&policy, &transfer, &signatures)?)
    } else {
        format!("0x{}", hex::encode(sha3_256(&transfer.message()?)))
    };
    Ok(SessionReview {
        transaction_id,
        digest: hex::encode(sha3_256(&transfer.message()?)),
        threshold: u64::from(policy.required),
        signers: members(&policy, &signed),
        inputs: Vec::new(),
        outputs: vec![MultisigOutput {
            address: format!("0x{}", hex::encode(transfer.recipient)),
            value: transfer.amount.to_string(),
            is_change: false,
            data: None,
            asset: None,
            memo: None,
        }],
        fee: (u128::from(transfer.max_gas) * u128::from(transfer.gas_price)).to_string(),
        sequence: Some(transfer.sequence.to_string()),
        expires_at: Some(transfer.expiration),
        expires_at_height: None,
        complete,
        data: serde_json::to_string(session)?,
    })
}

fn sha3_256(data: &[u8]) -> [u8; 32] {
    use sha3::Digest;
    sha3::Sha3_256::digest(data).into()
}

pub(super) fn account(wallet: &WalletState) -> Result<MultisigAccount, SpectraBridgeError> {
    let policy = policy(wallet)?;
    Ok(MultisigAccount {
        wallet_id: wallet.id.clone(),
        chain: wallet.chain_id,
        scheme: MultisigScheme::AptosMultiKey,
        address: policy.authentication_key(),
        permissions: vec![MultisigPermission {
            name: "MultiKey".into(),
            threshold: u64::from(policy.required),
            signers: members(&policy, &[]),
            covers: Vec::new(),
        }],
        warnings: Vec::new(),
        submission: MultisigScheme::AptosMultiKey.submission(),
        signer_wallet_ids: Vec::new(),
    })
}

/// The address a member key holds alone, for naming the wallet that holds
/// it.
pub(super) fn member_address(signer: &str) -> Option<String> {
    let policy = AptosMultiKey::parse(
        &serde_json::json!({"signaturesRequired": 1, "publicKeys": [signer]}).to_string(),
    )
    .ok()?;
    Some(policy.keys[0].single_address())
}

impl WalletService {
    async fn aptos_verified_client(&self, chain: Chain) -> Result<AptosClient, SpectraBridgeError> {
        let client = AptosClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        let (ledger_chain, _) = client.fetch_ledger_info().await?;
        if Some(ledger_chain) != chain.aptos_chain_id().map(u64::from) {
            return Err(SpectraBridgeError::invalid(
                "The Aptos endpoint is on another network.",
            ));
        }
        Ok(client)
    }

    /// The account's next sequence, refused unless its authentication key
    /// is still the policy's.
    async fn aptos_sequence(
        &self,
        client: &AptosClient,
        policy: &AptosMultiKey,
    ) -> Result<u64, SpectraBridgeError> {
        let address = policy.authentication_key();
        let (sequence, key) = client.fetch_account_auth(&address).await?.ok_or_else(|| {
            SpectraBridgeError::invalid("This account is not on the network yet.")
        })?;
        if key != address {
            return Err(SpectraBridgeError::invalid(
                "The account's authentication key is no longer its MultiKey's: its key was rotated.",
            ));
        }
        Ok(sequence)
    }

    /// The account now, refused unless its key is the policy's, its
    /// sequence the transfer's and the expiration not passed.
    async fn aptos_unchanged(
        &self,
        wallet: &WalletState,
        policy: &AptosMultiKey,
        transfer: &AptosTransfer,
    ) -> Result<(), SpectraBridgeError> {
        if crate::store::now_unix() as u64 >= transfer.expiration {
            return Err(SpectraBridgeError::invalid(
                "The transaction's expiration has passed; build it again.",
            ));
        }
        let client = self.aptos_verified_client(wallet.chain_id).await?;
        if self.aptos_sequence(&client, policy).await? != transfer.sequence {
            return Err(SpectraBridgeError::invalid(
                "The account's sequence moved on: another transaction used this one's. Build it again.",
            ));
        }
        Ok(())
    }

    /// An APT transfer from the account at its next sequence, expiring
    /// `spend.expires_in_secs` from now.
    pub(super) async fn create_aptos(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let to = spend.to_address.trim();
        if !crate::send::flow::is_valid_send_address(chain, to.to_string()) {
            return Err(SpectraBridgeError::refused(
                "Not an address on %@: %@",
                [chain.chain_display_name(), to],
            ));
        }
        let octas =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let policy = policy(wallet)?;
        let client = self.aptos_verified_client(chain).await?;
        let sequence = self.aptos_sequence(&client, &policy).await?;
        let max_gas = chain
            .aptos_max_gas_amount()
            .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos gas limit"))?;
        let gas_price = client.fetch_gas_price().await?;
        let balance = client
            .fetch_balance(&policy.authentication_key())
            .await?
            .octas;
        if u128::from(balance) < u128::from(octas) + u128::from(max_gas) * u128::from(gas_price) {
            return Err(crate::send::error::SendError::insufficient_funds().into());
        }
        let lifetime = spend
            .expires_in_secs
            .unwrap_or(DEFAULT_LIFETIME_SECS)
            .max(1);
        let transfer = AptosTransfer {
            sender: aptos::account_address(&policy.authentication_key())?,
            sequence,
            recipient: aptos::account_address(to)?,
            amount: octas,
            max_gas,
            gas_price,
            expiration: crate::store::now_unix() as u64 + lifetime,
            chain_id: chain
                .aptos_chain_id()
                .ok_or_else(|| SpectraBridgeError::invalid("Missing Aptos chain id"))?,
        };
        Ok(SessionBody::Aptos(AptosSession {
            transaction: format!("0x{}", hex::encode(transfer.raw()?)),
            signatures: Vec::new(),
        }))
    }

    /// A session another member wrote: refused unless it spends from this
    /// account, each signature its member's. A copy of a transaction an
    /// open session holds joins it.
    pub(super) fn import_aptos(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let incoming: AptosSession = serde_json::from_str(data)
            .map_err(|_| SpectraBridgeError::invalid("Not an Aptos MultiKey session."))?;
        let (_, transfer, _) = read(wallet, &incoming)?;
        for mut stored in open {
            let SessionBody::Aptos(held) = &stored.body else {
                continue;
            };
            let (_, held_transfer, _) = read(wallet, held)?;
            if held_transfer != transfer {
                continue;
            }
            let mut merged = held.clone();
            for signature in &incoming.signatures {
                if merged
                    .signatures
                    .iter()
                    .all(|own| own.index != signature.index)
                {
                    merged.signatures.push(signature.clone());
                }
            }
            read(wallet, &merged)?;
            stored.body = SessionBody::Aptos(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Aptos(incoming)))
    }

    /// Sign the session as `signer_wallet_id`, a wallet holding one of the
    /// account's Ed25519 keys.
    pub(super) async fn sign_aptos(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Aptos(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not an Aptos session"));
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
        let (policy, transfer, signatures) = read(wallet, session)?;
        self.aptos_unchanged(wallet, &policy, &transfer).await?;
        let identity = self
            .resolve_send_identity(wallet.chain_id, &signer_wallet_id, password.as_deref())
            .await?;
        let seed = crate::send::keys::Ed25519Seed::from_hex(&identity.private_key_hex)?;
        let member = policy
            .keys
            .iter()
            .position(|key| *key == AptosKey::Ed25519(seed.public_key()))
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "%@ holds none of the account's keys.",
                    [signer.name.as_str()],
                )
            })?;
        if signatures
            .iter()
            .any(|signature| signature.member == member)
        {
            return Err(SpectraBridgeError::invalid(
                "This key already signed the session.",
            ));
        }
        let signature = aptos::sign_ed25519(&transfer, &seed)?;
        aptos::member_signature(&policy, &transfer, member, &signature)?;
        let mut session = session.clone();
        session.signatures.push(AptosSignatureData {
            index: member,
            signature: format!("0x{}", hex::encode(signature)),
        });
        stored.body = SessionBody::Aptos(session);
        Ok(())
    }

    /// The signed transaction, BCS, once the required members signed.
    pub(super) fn finalize_aptos(
        wallet: &WalletState,
        session: &AptosSession,
    ) -> Result<Vec<u8>, SpectraBridgeError> {
        let (policy, transfer, signatures) = read(wallet, session)?;
        Ok(aptos::signed_transaction(&policy, &transfer, &signatures)?)
    }

    /// Submit the session's signed transaction, the account read again
    /// first. The transaction hash.
    pub(super) async fn submit_aptos(
        &self,
        wallet: &WalletState,
        session: &AptosSession,
    ) -> Result<String, SpectraBridgeError> {
        let (policy, transfer, _) = read(wallet, session)?;
        self.aptos_unchanged(wallet, &policy, &transfer).await?;
        let signed = Self::finalize_aptos(wallet, session)?;
        let hash = aptos::transaction_hash(&signed);
        let accepted = self
            .broadcast_raw_extract(
                wallet.chain_id,
                json!({ "signed_bcs_hex": hex::encode(&signed) }).to_string(),
                "txid".into(),
            )
            .await?;
        if !accepted.is_empty() && !accepted.eq_ignore_ascii_case(&hash) {
            return Err(SpectraBridgeError::failure(
                "The network accepted another transaction hash than the session's",
            ));
        }
        Ok(hash)
    }
}
