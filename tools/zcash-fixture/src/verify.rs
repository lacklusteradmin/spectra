//! What a node checks of a transaction it is handed, as far as this chain
//! can: the consensus branch and expiry; every transparent input unspent and
//! signed by the key its script names, over the ZIP-244 digest; each
//! Orchard-family bundle's anchor known, its nullifiers unspent, and its
//! proof and signatures valid; the value it moves balanced by a fee at least
//! ZIP-317's. A Sapling bundle is refused: this chain holds no Sapling
//! parameters to check its proofs with.

use std::collections::HashMap;
use std::sync::OnceLock;

use orchard::circuit::{OrchardCircuitVersion, VerifyingKey};
use zcash_client_backend::TransferType;
use zcash_keys::keys::UnifiedFullViewingKey;
use zcash_primitives::transaction::Transaction;
use zcash_primitives::transaction::sighash::{SignableInput, signature_hash};
use zcash_primitives::transaction::txid::TxIdDigester;
use zcash_protocol::consensus::BlockHeight;

use crate::chain::Chain;

/// The transparent inputs' amounts and scripts, as ZIP-244 signs them.
#[derive(Clone, Debug)]
struct Prevouts {
    amounts: Vec<zcash_protocol::value::Zatoshis>,
    scripts: Vec<zcash_transparent::address::Script>,
}

impl zcash_transparent::bundle::Authorization for Prevouts {
    type ScriptSig = zcash_transparent::address::Script;
}

impl zcash_transparent::sighash::TransparentAuthorizingContext for Prevouts {
    fn input_amounts(&self) -> Vec<zcash_protocol::value::Zatoshis> {
        self.amounts.clone()
    }
    fn input_scriptpubkeys(&self) -> Vec<zcash_transparent::address::Script> {
        self.scripts.clone()
    }
}

/// A received transaction with its inputs' amounts and scripts beside it.
struct Checked;

impl zcash_primitives::transaction::Authorization for Checked {
    type TransparentAuth = Prevouts;
    type SaplingAuth = sapling_crypto::bundle::Authorized;
    type OrchardAuth = orchard::bundle::Authorized;
}

fn verifying_key(version: OrchardCircuitVersion) -> &'static VerifyingKey {
    static FIXED: OnceLock<VerifyingKey> = OnceLock::new();
    static NU6_3: OnceLock<VerifyingKey> = OnceLock::new();
    static OLD: OnceLock<VerifyingKey> = OnceLock::new();
    let cell = match version {
        OrchardCircuitVersion::FixedPostNu6_2 => &FIXED,
        OrchardCircuitVersion::PostNu6_3 => &NU6_3,
        OrchardCircuitVersion::InsecurePreNu6_2 => &OLD,
    };
    cell.get_or_init(|| VerifyingKey::build(version))
}

/// Read a script push: one length byte, then that many bytes.
fn push(script: &[u8]) -> Option<(&[u8], &[u8])> {
    let (&length, rest) = script.split_first()?;
    let length = usize::from(length);
    (length <= 75 && rest.len() >= length).then(|| rest.split_at(length))
}

/// Check `tx` against the chain; on success, what it did.
pub fn check(
    chain: &Chain,
    tx: &Transaction,
    accounts: &HashMap<&'static str, UnifiedFullViewingKey>,
) -> Result<serde_json::Value, String> {
    let height = chain.next_height();
    if tx.consensus_branch_id() != chain.branch() {
        return Err(format!(
            "consensus branch {:08x}, the next block's is {:08x}",
            u32::from(tx.consensus_branch_id()),
            u32::from(chain.branch())
        ));
    }
    let expiry = u32::from(tx.expiry_height());
    if expiry != 0 && expiry < height {
        return Err(format!("expired at {expiry}"));
    }
    // ZIP-244 commits every signature to the amounts and scripts of all the
    // transparent inputs, which the transaction does not carry: the chain
    // supplies them from what it holds.
    let mut prevouts = Prevouts {
        amounts: Vec::new(),
        scripts: Vec::new(),
    };
    if let Some(bundle) = tx.transparent_bundle() {
        for input in &bundle.vin {
            let prevout = input.prevout();
            let utxo = chain
                .utxos
                .iter()
                .find(|utxo| utxo.txid == *prevout.hash() && utxo.index == prevout.n())
                .ok_or("a transparent input is unknown or spent")?;
            prevouts.amounts.push(
                zcash_protocol::value::Zatoshis::from_u64(utxo.value)
                    .map_err(|_| "invalid input value")?,
            );
            prevouts.scripts.push(zcash_transparent::address::Script(
                zcash_script::script::Code(utxo.script.clone()),
            ));
        }
    }
    let mut raw = Vec::new();
    tx.write(&mut raw).map_err(|e| e.to_string())?;
    let data = Transaction::read(&raw[..], tx.consensus_branch_id())
        .map_err(|e| e.to_string())?
        .into_data()
        .map_bundles::<Checked>(
            |bundle| {
                bundle.map(|bundle| zcash_transparent::bundle::Bundle {
                    vin: bundle
                        .vin
                        .iter()
                        .map(|input| {
                            zcash_transparent::bundle::TxIn::from_parts(
                                input.prevout().clone(),
                                input.script_sig().clone(),
                                input.sequence(),
                            )
                        })
                        .collect(),
                    vout: bundle.vout.clone(),
                    authorization: prevouts.clone(),
                })
            },
            |sapling| sapling,
            |orchard| orchard,
        );
    let digest = data.digest(TxIdDigester);
    let shielded = signature_hash(&data, &SignableInput::Shielded, &digest);

    let mut value_in: i128 = 0;
    let mut value_out: i128 = 0;
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    if let Some(bundle) = tx.transparent_bundle() {
        for (index, input) in bundle.vin.iter().enumerate() {
            let prevout = input.prevout();
            let utxo = chain
                .utxos
                .iter()
                .find(|utxo| utxo.txid == *prevout.hash() && utxo.index == prevout.n())
                .ok_or("a transparent input is unknown or spent")?;
            let script_sig = &input.script_sig().0.0;
            let (signature, rest) = push(script_sig).ok_or("unreadable scriptSig")?;
            let (pubkey, rest) = push(rest).ok_or("unreadable scriptSig")?;
            if !rest.is_empty() {
                return Err("scriptSig carries more than a signature and a key".into());
            }
            let pubkey = secp256k1::PublicKey::from_slice(pubkey).map_err(|e| e.to_string())?;
            // P2PKH: the script names the key's HASH160.
            let script_code =
                zcash_transparent::address::Script(zcash_script::script::Code(utxo.script.clone()));
            let expected = zcash_transparent::address::TransparentAddress::from_pubkey(&pubkey);
            let named = zcash_transparent::bundle::TxOut::new(
                zcash_protocol::value::Zatoshis::ZERO,
                script_code.clone(),
            )
            .recipient_address();
            if named != Some(expected) {
                return Err("the input's key is not the one its script names".into());
            }
            let (&hash_type, der) = signature.split_last().ok_or("empty signature")?;
            let sighash_type = zcash_transparent::sighash::SighashType::parse(hash_type)
                .ok_or("unknown sighash type")?;
            let value = zcash_protocol::value::Zatoshis::from_u64(utxo.value)
                .map_err(|_| "invalid input value")?;
            let signable = zcash_transparent::sighash::SignableInput::from_parts(
                data.transparent_bundle().ok_or("no transparent bundle")?,
                sighash_type,
                index,
                &script_code,
                &script_code,
                value,
            )
            .map_err(|_| "an input index past the inputs")?;
            let sighash = signature_hash(&data, &SignableInput::Transparent(signable), &digest);
            let signature =
                secp256k1::ecdsa::Signature::from_der(der).map_err(|e| e.to_string())?;
            let message = secp256k1::Message::from_digest(*sighash.as_ref());
            secp256k1::SECP256K1
                .verify_ecdsa(&message, &signature, &pubkey)
                .map_err(|_| "a transparent signature does not verify")?;
            value_in += i128::from(utxo.value);
            inputs.push(serde_json::json!({"address": utxo.address, "value": utxo.value}));
        }
        for output in &bundle.vout {
            value_out += i128::from(u64::from(output.value()));
            outputs.push(serde_json::json!({
                "address": output.recipient_address().map(|address| {
                    zcash_keys::encoding::AddressCodec::encode(&address, &chain.network)
                }),
                "value": u64::from(output.value()),
            }));
        }
    }
    if tx.sapling_bundle().is_some() {
        return Err("a Sapling bundle, whose proofs this chain cannot check".into());
    }
    let mut actions = serde_json::Map::new();
    for (name, bundle, anchors) in [
        ("orchard", tx.orchard_bundle(), &chain.orchard_anchors),
        ("ironwood", tx.ironwood_bundle(), &chain.ironwood_anchors),
    ] {
        let Some(bundle) = bundle else { continue };
        if !anchors.contains(&bundle.anchor().to_bytes()) {
            return Err(format!("the {name} anchor is not one of the chain's"));
        }
        for action in bundle.actions() {
            if chain
                .nullifiers
                .contains(action.nullifier().to_bytes().as_slice())
            {
                return Err(format!("a {name} note is spent twice"));
            }
        }
        let key = verifying_key(bundle.bundle_version().circuit_version());
        let mut validator = orchard::bundle::BatchValidator::new(key);
        validator
            .add_bundle(bundle, *shielded.as_ref())
            .map_err(|_| format!("the {name} bundle cannot be checked"))?;
        if !validator.validate(rand::rngs::OsRng) {
            return Err(format!("the {name} proof or signatures do not verify"));
        }
        value_in += i128::from(i64::from(*bundle.value_balance()));
        actions.insert(name.into(), serde_json::json!(bundle.actions().len()));
    }
    let fee = value_in - value_out;
    if fee < 10_000 {
        return Err(format!("the fee is {fee} zatoshis, below ZIP-317's"));
    }

    // Each Orchard-family output an account the chain knows can read: as
    // its recipient ("incoming"), as change to itself ("change"), or as the
    // sender, through its outgoing viewing key ("outgoing").
    let decrypted = zcash_client_backend::decrypt_transaction(
        &chain.network,
        Some(BlockHeight::from_u32(height)),
        None,
        tx,
        accounts,
    );
    let mut paid = Vec::new();
    for output in decrypted
        .orchard_outputs()
        .iter()
        .chain(decrypted.ironwood_outputs())
    {
        let memo = match zcash_protocol::memo::Memo::try_from(output.memo().clone()) {
            Ok(zcash_protocol::memo::Memo::Text(text)) => Some(text.to_string()),
            _ => None,
        };
        paid.push(serde_json::json!({
            "pool": format!("{:?}", output.note().1).to_lowercase(),
            "account": output.account(),
            "value": output.note().0.value().inner(),
            "memo": memo,
            "transfer": match output.transfer_type() {
                TransferType::Incoming => "incoming",
                TransferType::AccountInternal | TransferType::WalletInternal => "change",
                TransferType::Outgoing => "outgoing",
            },
        }));
    }
    Ok(serde_json::json!({
        "txid": tx.txid().to_string(),
        "height": height,
        "transparent_inputs": inputs,
        "transparent_outputs": outputs,
        "actions": actions,
        "fee": fee,
        "paid": paid,
    }))
}
