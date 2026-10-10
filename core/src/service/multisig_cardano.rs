//! A Cardano native-script account's sessions: a transfer from the script's
//! address carrying the script, signed by its cosigners' CIP-1854 keys.
//! Each cosigner signs from a Cardano wallet of its own holding a phrase:
//! the key at `m/1854'/1815'/0'/0/index` the script names. The transaction
//! travels as its CBOR with the witnesses gathered, which any Cardano
//! tool reads. Before a witness and before submission, the inputs are read
//! again (still the script's, unspent) and the slot checked against the
//! spend's validity.
use super::multisig::{
    MultisigAccount, MultisigOutput, MultisigPermission, MultisigScheme, MultisigSigner,
    MultisigSpend, SessionBody, SessionReview, StoredSession,
};
use super::*;
use crate::derivation::cardano_script::NativeScript;
use crate::send::cardano::{CardanoTransfer, PreparedCardanoTransaction};
use crate::send::cardano_multisig::{self as cardano, Witness};
use crate::store::state::{WalletSigning, WalletState};

/// How long, in slots (seconds), a session gathers witnesses unless asked
/// otherwise: what an ordinary send allows.
const DEFAULT_LIFETIME_SLOTS: u64 = 7200;
/// How many of a phrase's CIP-1854 keys are searched for a cosigner's.
const COSIGNER_KEYS: u32 = 20;

/// A session's content: the transaction as built or read, the outputs it
/// spends included, and the witnesses gathered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CardanoSession {
    pub transaction: PreparedCardanoTransaction,
    /// `(vkey, signature)`, hex.
    pub witnesses: Vec<(String, String)>,
}

fn script(wallet: &WalletState) -> Result<NativeScript, SpectraBridgeError> {
    Ok(NativeScript::parse(
        wallet.multisig_policy.as_deref().ok_or_else(|| {
            SpectraBridgeError::invalid("This wallet is not a Cardano multisig account.")
        })?,
    )?)
}

fn witnesses(session: &CardanoSession) -> Result<Vec<Witness>, SpectraBridgeError> {
    session
        .witnesses
        .iter()
        .map(|(vkey, signature)| {
            Ok((
                hex::decode(vkey)?
                    .try_into()
                    .map_err(|_| SpectraBridgeError::invalid("A witness key is 32 bytes."))?,
                hex::decode(signature)?
                    .try_into()
                    .map_err(|_| SpectraBridgeError::invalid("A witness signature is 64 bytes."))?,
            ))
        })
        .collect()
}

fn members(script: &NativeScript, signed: &[[u8; 28]]) -> Vec<MultisigSigner> {
    script
        .key_hashes()
        .iter()
        .map(|hash| MultisigSigner {
            signer: hex::encode(hash),
            weight: 1,
            signed: signed.contains(hash),
            wallet_id: None,
        })
        .collect()
}

/// The session's script, transaction and verified signers.
fn read(
    wallet: &WalletState,
    session: &CardanoSession,
) -> Result<(NativeScript, Vec<Witness>, Vec<[u8; 28]>), SpectraBridgeError> {
    let script = script(wallet)?;
    let transaction = &session.transaction;
    let expected = cardano::spend(
        &script,
        transaction.script.as_ref().and_then(|s| s.validity_start),
    );
    if transaction.script.as_ref() != Some(&expected) {
        return Err(SpectraBridgeError::invalid(
            "This transaction spends another script's outputs.",
        ));
    }
    let witnesses = witnesses(session)?;
    let signed = cardano::signed_keys(transaction, &script, &witnesses)?;
    Ok((script, witnesses, signed))
}

pub(super) fn review(
    wallet: &WalletState,
    session: &CardanoSession,
) -> Result<SessionReview, SpectraBridgeError> {
    let (script, witnesses, signed) = read(wallet, session)?;
    let transaction = &session.transaction;
    let start = transaction
        .script
        .as_ref()
        .and_then(|spend| spend.validity_start);
    let hash = hex::encode(transaction.transaction_hash()?);
    Ok(SessionReview {
        transaction_id: hash.clone(),
        digest: hash,
        threshold: script.min_signers(),
        signers: members(&script, &signed),
        inputs: transaction
            .inputs
            .iter()
            .map(|input| super::multisig::MultisigInput {
                outpoint: format!("{}#{}", input.tx_hash, input.tx_index),
                address: script.address(wallet.chain_id).unwrap_or_default(),
                value: input.lovelace.to_string(),
                signatures: signed.len() as u32,
            })
            .collect(),
        outputs: transaction
            .outputs
            .iter()
            .map(|output| MultisigOutput {
                address: output.address.clone(),
                value: output.lovelace.to_string(),
                is_change: script
                    .address(wallet.chain_id)
                    .is_ok_and(|own| own == output.address),
                data: None,
                asset: (!output.assets.is_empty()).then(|| {
                    output
                        .assets
                        .iter()
                        .map(|asset| format!("{} {}", asset.quantity, asset.asset))
                        .collect::<Vec<_>>()
                        .join(", ")
                }),
                memo: None,
            })
            .collect(),
        fee: transaction.fee.to_string(),
        sequence: None,
        expires_at: None,
        expires_at_height: Some(transaction.ttl),
        complete: script.satisfied(&signed, start, transaction.ttl),
        data: hex::encode(transaction.encode_signed(&witnesses)?),
    })
}

pub(super) fn account(wallet: &WalletState) -> Result<MultisigAccount, SpectraBridgeError> {
    let script = script(wallet)?;
    let mut warnings = Vec::new();
    let (after, before) = script.validity();
    if let Some(slot) = after {
        warnings.push(crate::LocalizableMessage::new(
            "The script spends only from slot %@ on.",
            [slot],
        ));
    }
    if let Some(slot) = before {
        warnings.push(crate::LocalizableMessage::new(
            "The script spends only before slot %@; after it the funds are locked for good.",
            [slot],
        ));
    }
    Ok(MultisigAccount {
        wallet_id: wallet.id.clone(),
        chain: wallet.chain_id,
        scheme: MultisigScheme::CardanoNativeScript,
        address: script.address(wallet.chain_id)?,
        permissions: vec![MultisigPermission {
            name: "native script".into(),
            threshold: script.min_signers(),
            signers: members(&script, &[]),
            covers: Vec::new(),
        }],
        warnings,
        submission: MultisigScheme::CardanoNativeScript.submission(),
        signer_wallet_ids: Vec::new(),
    })
}

impl WalletService {
    async fn koios(&self, chain: Chain) -> Result<KoiosClient, SpectraBridgeError> {
        let client = KoiosClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
        );
        client.verify_network(chain).await?;
        Ok(client)
    }

    /// The slot now, refused unless the transaction is valid in it, and
    /// every input still one of the script's unspent outputs, as it was.
    async fn cardano_unchanged(
        &self,
        wallet: &WalletState,
        script: &NativeScript,
        transaction: &PreparedCardanoTransaction,
    ) -> Result<(), SpectraBridgeError> {
        let chain = wallet.chain_id;
        let slot = self.koios(chain).await?.fetch_latest_slot().await?;
        if slot >= transaction.ttl {
            return Err(SpectraBridgeError::invalid(
                "The transaction's last slot has passed; build it again.",
            ));
        }
        if transaction
            .script
            .as_ref()
            .and_then(|spend| spend.validity_start)
            .is_some_and(|start| slot < start)
        {
            return Err(SpectraBridgeError::invalid(
                "The transaction is not valid yet: the script spends only from a later slot.",
            ));
        }
        let held = self.cardano_inputs(chain, &script.address(chain)?).await?;
        if transaction.inputs.iter().any(|input| !held.contains(input)) {
            return Err(SpectraBridgeError::invalid(
                "An output the transaction spends is spent or changed; build it again.",
            ));
        }
        Ok(())
    }

    /// An ADA transfer from the script's outputs, valid for
    /// `spend.expires_in_secs` slots and within every bound the script
    /// names.
    pub(super) async fn create_cardano(
        &self,
        wallet: &WalletState,
        spend: &MultisigSpend,
    ) -> Result<SessionBody, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let script = script(wallet)?;
        let address = script.address(chain)?;
        let to = spend.to_address.trim();
        if !crate::send::flow::is_valid_send_address(chain, to.to_string()) {
            return Err(SpectraBridgeError::refused(
                "Not an address on %@: %@",
                [chain.chain_display_name(), to],
            ));
        }
        let lovelace =
            crate::decimal::to_units(spend.amount.trim(), u32::from(chain.native_decimals()))
                .and_then(|units| u64::try_from(units).ok())
                .filter(|units| *units > 0)
                .ok_or_else(|| SpectraBridgeError::invalid("Invalid amount"))?;
        let client = self.koios(chain).await?;
        let slot = client.fetch_latest_slot().await?;
        let (after, before) = script.validity();
        if after.is_some_and(|after| slot < after) {
            return Err(SpectraBridgeError::refused(
                "The script spends only from slot %@ on.",
                [after.unwrap_or_default()],
            ));
        }
        let ttl = slot
            .checked_add(
                spend
                    .expires_in_secs
                    .unwrap_or(DEFAULT_LIFETIME_SLOTS)
                    .max(1),
            )
            .ok_or_else(|| SpectraBridgeError::failure("Slot overflow"))?
            .min(before.unwrap_or(u64::MAX));
        if ttl <= slot {
            return Err(SpectraBridgeError::invalid(
                "The script's last slot has passed: its funds can no longer be spent.",
            ));
        }
        let utxos = self.cardano_inputs(chain, &address).await?;
        let transaction = PreparedCardanoTransaction::plan(
            &utxos,
            &client.fetch_protocol_params().await?,
            &address,
            to,
            &CardanoTransfer::Ada(lovelace),
            ttl,
            Some(&cardano::spend(&script, after)),
        )?;
        Ok(SessionBody::Cardano(CardanoSession {
            transaction,
            witnesses: Vec::new(),
        }))
    }

    /// A transaction another cosigner wrote, its CBOR hex: refused unless it
    /// spends this script's unspent outputs and carries the script, each
    /// witness a cosigner's. A copy of a transaction an open session holds
    /// joins it.
    pub(super) async fn import_cardano(
        &self,
        wallet: &WalletState,
        open: Vec<StoredSession>,
        data: &str,
    ) -> Result<StoredSession, SpectraBridgeError> {
        let chain = wallet.chain_id;
        let script = script(wallet)?;
        let raw = hex::decode(data.trim())
            .map_err(|_| SpectraBridgeError::invalid("Not a Cardano transaction."))?;
        let decoded = cardano::decode(&raw, &script)?;
        let held = self.cardano_inputs(chain, &script.address(chain)?).await?;
        let transaction = decoded.prepared(&script, &held, &raw)?;
        cardano::signed_keys(&transaction, &script, &decoded.witnesses)?;
        let incoming = CardanoSession {
            transaction,
            witnesses: decoded
                .witnesses
                .iter()
                .map(|(vkey, signature)| (hex::encode(vkey), hex::encode(signature)))
                .collect(),
        };
        for mut stored in open {
            let SessionBody::Cardano(held) = &stored.body else {
                continue;
            };
            if held.transaction != incoming.transaction {
                continue;
            }
            let mut merged = held.clone();
            for witness in &incoming.witnesses {
                if merged.witnesses.iter().all(|(vkey, _)| *vkey != witness.0) {
                    merged.witnesses.push(witness.clone());
                }
            }
            read(wallet, &merged)?;
            stored.body = SessionBody::Cardano(merged);
            return Ok(stored);
        }
        Ok(StoredSession::new(wallet, SessionBody::Cardano(incoming)))
    }

    /// Witness the session as `signer_wallet_id`, a Cardano wallet whose
    /// phrase holds one of the script's CIP-1854 keys.
    pub(super) async fn sign_cardano(
        &self,
        wallet: &WalletState,
        stored: &mut StoredSession,
        signer_wallet_id: Option<String>,
        password: Option<String>,
    ) -> Result<(), SpectraBridgeError> {
        let SessionBody::Cardano(session) = &stored.body else {
            return Err(SpectraBridgeError::failure("not a Cardano session"));
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
        if !matches!(signer.signing, WalletSigning::SeedPhrase { .. }) {
            return Err(SpectraBridgeError::refused(
                "%@ holds no phrase, and a cosigner's key derives from one.",
                [signer.name.as_str()],
            ));
        }
        let (script, mut gathered, signed) = read(wallet, session)?;
        self.cardano_unchanged(wallet, &script, &session.transaction)
            .await?;
        let phrase = crate::store::wallet_secrets::load_seed_phrase(
            &*self.secrets()?,
            &signer.id,
            password.as_deref(),
        )?;
        let named = script.key_hashes();
        let keys = crate::derivation::cardano::cosigner_keys(
            &phrase,
            signer
                .derivation_overrides
                .passphrase
                .as_deref()
                .unwrap_or_default(),
            COSIGNER_KEYS,
        )?;
        let (private, public, hash) = keys
            .iter()
            .find(|(_, _, hash)| named.contains(hash))
            .ok_or_else(|| {
                SpectraBridgeError::refused(
                    "%@'s phrase holds none of the script's keys.",
                    [signer.name.as_str()],
                )
            })?;
        if signed.contains(hash) {
            return Err(SpectraBridgeError::invalid(
                "This key already signed the session.",
            ));
        }
        let signature = crate::send::cardano::sign_extended(
            private,
            public,
            &session.transaction.transaction_hash()?,
        )?;
        gathered.push((*public, signature));
        cardano::signed_keys(&session.transaction, &script, &gathered)?;
        let mut session = session.clone();
        session
            .witnesses
            .push((hex::encode(public), hex::encode(signature)));
        stored.body = SessionBody::Cardano(session);
        Ok(())
    }

    /// Submit the session once its witnesses satisfy the script, its inputs
    /// unspent and its slot valid. The transaction id.
    pub(super) async fn submit_cardano(
        &self,
        wallet: &WalletState,
        session: &CardanoSession,
    ) -> Result<String, SpectraBridgeError> {
        let (script, _, _) = read(wallet, session)?;
        self.cardano_unchanged(wallet, &script, &session.transaction)
            .await?;
        let reviewed = review(wallet, session)?;
        if !reviewed.complete {
            return Err(SpectraBridgeError::invalid(
                "The witnesses do not yet satisfy the script.",
            ));
        }
        let txid = self
            .broadcast_raw_extract(
                wallet.chain_id,
                json!({ "cbor_hex": reviewed.data }).to_string(),
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
#[path = "tests/multisig_cardano.rs"]
mod multisig_cardano_tests;
