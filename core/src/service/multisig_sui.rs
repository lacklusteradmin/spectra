//! A Sui multisig account's sessions: a SUI transfer from the account's
//! address with its members' signatures, gathered as data (Sui's own
//! serialized signatures) and combined into one `MultiSig` signature once
//! their weights meet the threshold. The policy is the wallet's own, from
//! which its address derives; the gas objects the transfer pays with are
//! read again before a signature and before execution, so a session whose
//! gas the account spent elsewhere is refused rather than signed.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::derivation::sui_multisig::{SuiMultisig, SuiScheme};
use crate::send::sui_multisig::{self as sui, MemberSignature};
use crate::store::state::WalletState;
use base64::Engine;

/// The gas budget a session's transfer carries, in MIST: what an ordinary
/// SUI send carries.
const GAS_BUDGET: u64 = 10_000_000;

/// A session's content: the transaction and the members' signatures, both
/// base64, as Sui writes them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SuiSession {
    pub transaction: String,
    pub signatures: Vec<String>,
}

fn policy(wallet: &WalletState) -> Result<SuiMultisig, SpectraBridgeError> {
    Ok(SuiMultisig::parse(
        wallet.multisig_policy.as_deref().ok_or_else(|| {
            SpectraBridgeError::invalid("This wallet is not a Sui multisig account.")
        })?,
    )?)
}

fn engine() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn members(policy: &SuiMultisig, signed: &[usize]) -> Vec<MultisigSigner> {
    policy
        .members
        .iter()
        .enumerate()
        .map(|(index, member)| MultisigSigner {
            signer: member.sui_public_key(),
            weight: u64::from(member.weight),
            signed: signed.contains(&index),
            wallet_id: None,
        })
        .collect()
}

/// The session's transfer and its verified signatures.
fn read(
    wallet: &WalletState,
    session: &SuiSession,
) -> Result<(SuiMultisig, Vec<u8>, sui::SuiTransfer, Vec<MemberSignature>), SpectraBridgeError> {
    let policy = policy(wallet)?;
    let bytes = engine()
        .decode(session.transaction.trim())
        .map_err(|_| SpectraBridgeError::invalid("Not a Sui transaction."))?;
    let transfer = sui::decode(&bytes)?;
    if format!("0x{}", hex::encode(transfer.sender)) != policy.address() {
        return Err(SpectraBridgeError::invalid(
            "This transaction spends another account.",
        ));
    }
    let signatures = session
        .signatures
        .iter()
        .map(|signature| sui::member_signature(&policy, &bytes, signature))
        .collect::<Result<Vec<_>, _>>()?;
    sui::signed_weight(&policy, &signatures)?;
    Ok((policy, bytes, transfer, signatures))
}

pub(super) fn review(
    wallet: &WalletState,
    session: &SuiSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let (policy, bytes, transfer, signatures) = read(wallet, session)?;
    let (signed, weight) = sui::signed_weight(&policy, &signatures)?;
    Ok(SessionReview {
        transaction_id: sui::transaction_digest(&bytes),
        digest: hex::encode(sui::intent_digest(&bytes)),
        threshold: u64::from(policy.threshold),
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
        fee: transfer.gas_budget.to_string(),
        sequence: None,
        expires_at: None,
        expires_at_height: None,
        complete: weight >= u64::from(policy.threshold),
        data: serde_json::to_string(session)?,
    })
}

pub(super) fn account(wallet: &WalletState) -> Result<MultisigAccount, SpectraBridgeError> {
    let policy = policy(wallet)?;
    Ok(MultisigAccount {
        wallet_id: wallet.id.clone(),
        chain: wallet.chain_id,
        scheme: MultisigScheme::SuiMultisig,
        address: policy.address(),
        permissions: vec![MultisigPermission {
            name: "multisig".into(),
            threshold: u64::from(policy.threshold),
            signers: members(&policy, &[]),
            covers: Vec::new(),
        }],
        warnings: Vec::new(),
        submission: MultisigScheme::SuiMultisig.submission(),
        signer_wallet_ids: Vec::new(),
    })
}

/// The address a member's key holds alone, for naming the wallet that
/// holds it.
pub(super) fn member_address(signer: &str) -> Option<String> {
    let bytes = engine().decode(signer).ok()?;
    let (&flag, key) = bytes.split_first()?;
    let scheme = SuiScheme::of_flag(flag)?;
    (key.len() == scheme.key_length()).then(|| {
        crate::derivation::sui_multisig::SuiMember {
            scheme,
            public_key: key.to_vec(),
            weight: 1,
        }
        .address()
    })
}

impl WalletService {
    async fn sui_verified_client(&self, chain: Chain) -> Result<SuiClient, SpectraBridgeError> {
        let client = SuiClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        client.verify_network(chain).await?;
        Ok(client)
    }

    /// Every gas object the transfer pays with is still the account's, at
    /// the version and digest the transaction names.
    async fn sui_gas_unchanged(
        &self,
        wallet: &WalletState,
        transfer: &sui::SuiTransfer,
    ) -> Result<(), SpectraBridgeError> {
        let client = self.sui_verified_client(wallet.chain_id).await?;
        let owned = crate::send::sui::owned_coins(
            &client,
            &policy(wallet)?.address(),
            "0x2::sui::SUI",
            256,
        )
        .await?;
        for (id, version, digest) in &transfer.gas {
            if !owned
                .iter()
                .any(|coin| coin.id == *id && coin.version == *version && coin.digest == *digest)
            {
                return Err(SpectraBridgeError::invalid(
                    "A gas object the transaction pays with was spent or changed; build it again.",
                ));
            }
        }
        Ok(())
    }

    /// A SUI transfer from the account, its gas its own largest coins.
    pub(super) async fn create_sui(
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
        let mist =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let client = self.sui_verified_client(chain).await?;
        let prepared = crate::send::sui::prepare_native_transfer(
            &client,
            &policy(wallet)?.address(),
            to,
            mist,
            GAS_BUDGET,
        )
        .await?;
        Ok(SessionBody::Sui(SuiSession {
            transaction: engine().encode(&prepared.bytes),
            signatures: Vec::new(),
        }))
    }

    /// A session another member wrote: refused unless it spends from this
    /// account, each signature a member's. A copy of a transaction an open
    /// session holds joins it.
    pub(super) fn import_sui(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let incoming: SuiSession = serde_json::from_str(data)
            .map_err(|_| SpectraBridgeError::invalid("Not a Sui multisig session."))?;
        let (policy, bytes, _, signatures) = read(wallet, &incoming)?;
        for mut stored in open {
            let SessionBody::Sui(held) = &stored.body else {
                continue;
            };
            if engine().decode(held.transaction.trim()).ok().as_deref() != Some(bytes.as_slice()) {
                continue;
            }
            let (_, _, _, mut known) = read(wallet, held)?;
            let mut merged = held.clone();
            for (signature, serialized) in signatures.iter().zip(&incoming.signatures) {
                if known.iter().all(|own| own.member != signature.member) {
                    known.push(signature.clone());
                    merged.signatures.push(serialized.clone());
                }
            }
            sui::signed_weight(&policy, &known)?;
            stored.body = SessionBody::Sui(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Sui(incoming)))
    }

    /// Sign the session as `signer_wallet_id`, a wallet holding one of the
    /// account's Ed25519 keys, its gas unchanged.
    pub(super) async fn sign_sui(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Sui(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a Sui session"));
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
        let (policy, bytes, transfer, signatures) = read(wallet, session)?;
        self.sui_gas_unchanged(wallet, &transfer).await?;
        let identity = self
            .resolve_send_identity(wallet.chain_id, &signer_wallet_id, password.as_deref())
            .await?;
        let seed = crate::send::keys::Ed25519Seed::from_hex(&identity.private_key_hex)?;
        let member = policy
            .members
            .iter()
            .position(|member| {
                member.scheme == SuiScheme::Ed25519 && member.public_key == seed.public_key()
            })
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
        let serialized = sui::sign_ed25519(&bytes, &seed);
        if sui::member_signature(&policy, &bytes, &serialized)?.member != member {
            return Err(SpectraBridgeError::failure("signature does not verify"));
        }
        let mut session = session.clone();
        session.signatures.push(serialized);
        stored.body = SessionBody::Sui(session);
        Ok(())
    }

    /// The transaction and its combined signature, base64, once the
    /// members' weights meet the threshold.
    pub(super) fn finalize_sui(
        wallet: &WalletState,
        session: &SuiSession,
    ) -> Result<(String, String), SpectraBridgeError> {
        let (policy, bytes, _, signatures) = read(wallet, session)?;
        let combined = sui::combine(&policy, &signatures)?;
        Ok((engine().encode(bytes), engine().encode(combined)))
    }

    /// Execute the session with its combined signature, its gas unchanged.
    /// The transaction digest.
    pub(super) async fn execute_sui(
        &self,
        wallet: &WalletState,
        session: &SuiSession,
    ) -> Result<String, SpectraBridgeError> {
        let (_, bytes, transfer, _) = read(wallet, session)?;
        self.sui_gas_unchanged(wallet, &transfer).await?;
        let (transaction, signature) = Self::finalize_sui(wallet, session)?;
        let digest = sui::transaction_digest(&bytes);
        let accepted = self
            .broadcast_raw_extract(
                wallet.chain_id,
                json!({ "tx_bytes_b64": transaction, "sig_b64": signature }).to_string(),
                "digest".into(),
            )
            .await?;
        if !accepted.is_empty() && accepted != digest {
            return Err(SpectraBridgeError::failure(
                "The network executed another transaction digest than the session's",
            ));
        }
        Ok(digest)
    }
}
