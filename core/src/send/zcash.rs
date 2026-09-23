//! Zcash transparent send: build + sign V5 (ZIP-225) transactions and
//! broadcast via Trezor's Blockbook `/api/v2/sendtx`.
//!
//! Only transparent-only transactions are supported (empty Sapling and
//! Orchard bundles). Signing follows ZIP-244 (txid digest = personalised
//! BLAKE2b over header / transparent / sapling / orchard sub-digests).
//!
//! Consensus branch is read from the backend, checked against the registry,
//! and frozen in the reviewed artifact. See https://zips.z.cash/zip-0244.

pub(crate) use super::zcash_stages::PreparedZcashTransaction;

use super::bitcoin_wire::{decode_txid_le, p2pkh_script, varint};
#[cfg(test)]
use crate::derivation::zcash::decode_zcash_address;
#[cfg(test)]
use crate::fetch::blockbook::{BlockbookClient, BlockbookSendResult};

// ── Network upgrade descriptor ────────────────────────────────────────────

/// Zcash consensus rule set for V5 transaction construction.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ZcashNetworkUpgrade {
    pub version_group_id: u32,
    pub consensus_branch_id: u32,
}

impl ZcashNetworkUpgrade {
    /// NU5 fixture; production obtains the branch from verified backend status.
    #[cfg(test)]
    pub const NU5: Self = Self {
        version_group_id: 0x26A7_270A,
        consensus_branch_id: 0xC2D6_D0B4,
    };
}

// ── Network constants ─────────────────────────────────────────────────────

const TX_VERSION_V5: u32 = 5;
/// `nVersionGroupId` overwintered bit set on the version field.
const TX_VERSION_OVERWINTERED: u32 = 1 << 31;

const SIGHASH_ALL: u32 = 1;

const BLAKE2B_PERSONALIZED_LEN: usize = 32;

// ── Public broadcast + signing entrypoint ─────────────────────────────────

#[cfg(test)]
impl BlockbookClient {
    /// Fetch UTXOs + chain tip, sign a V5 transparent transaction, broadcast.
    #[cfg(test)]
    pub async fn sign_zcash_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
        network_upgrade: ZcashNetworkUpgrade,
        dust_threshold_zats: u64,
    ) -> Result<BlockbookSendResult, String> {
        self.require_chain(crate::registry::Chain::Zcash)?;
        let utxos = self.fetch_utxos(from_address).await?;
        let tip = self.fetch_chain_tip_height().await?;
        // Match zcashd default: 40-block expiry window.
        let expiry_height = expiry_height(tip)?;
        let from_hash = decode_zcash_address(from_address)?;
        let from_script = p2pkh_script(&from_hash);
        let utxo_tuples: Vec<(String, u32, u64, Vec<u8>)> = utxos
            .iter()
            .map(|u| (u.txid.clone(), u.vout, u.value_sat, from_script.clone()))
            .collect();
        let raw = sign_zcash_v5_p2pkh(
            &utxo_tuples,
            to_address,
            amount_sat,
            fee_sat,
            from_address,
            expiry_height,
            private_key_bytes,
            network_upgrade,
            dust_threshold_zats,
        )?;
        self.broadcast_raw_tx(&hex::encode(&raw)).await
    }
}

// ── Encoding helpers ──────────────────────────────────────────────────────

/// Personalised BLAKE2b-256. Personalisation is exactly 16 bytes (right-padded
/// with zero bytes if shorter) — this is how every ZIP-244 sub-digest is keyed.
fn blake2b_personalized(personalization: &[u8], data: &[u8]) -> [u8; BLAKE2B_PERSONALIZED_LEN] {
    let mut personal = [0u8; 16];
    let copy_len = personalization.len().min(16);
    personal[..copy_len].copy_from_slice(&personalization[..copy_len]);
    let hash = blake2b_simd::Params::new()
        .hash_length(BLAKE2B_PERSONALIZED_LEN)
        .personal(&personal)
        .hash(data);
    let mut out = [0u8; BLAKE2B_PERSONALIZED_LEN];
    out.copy_from_slice(hash.as_bytes());
    out
}

// ── ZIP-244 sub-digest constants ──────────────────────────────────────────

const PERSONAL_TX_HEADERS: &[u8] = b"ZTxIdHeadersHash";
const PERSONAL_TX_TRANSPARENT: &[u8] = b"ZTxIdTranspaHash";
const PERSONAL_TX_TRANSPARENT_OUTPUTS: &[u8] = b"ZTxIdOutputsHash";
const PERSONAL_TX_SAPLING: &[u8] = b"ZTxIdSaplingHash";
const PERSONAL_TX_ORCHARD: &[u8] = b"ZTxIdOrchardHash";
const PERSONAL_TX_TXID_BASE: &[u8] = b"ZcashTxHash_";
const PERSONAL_TX_PER_INPUT_AMOUNTS: &[u8] = b"ZTxTrAmountsHash";
const PERSONAL_TX_PER_INPUT_SCRIPTS: &[u8] = b"ZTxTrScriptsHash";
const PERSONAL_TX_PREVOUTS: &[u8] = b"ZTxIdPrevoutHash";
const PERSONAL_TX_SEQUENCE: &[u8] = b"ZTxIdSequencHash";
const PERSONAL_TX_SIG_DIGEST: &[u8] = b"Zcash___TxInHash";

// ── ZIP-244 / ZIP-244-revised sighash construction (NU5 transparent-only).

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) fn sign_zcash_v5_p2pkh(
    utxos: &[(String, u32, u64, Vec<u8>)],
    to_address: &str,
    amount_sat: u64,
    fee_sat: u64,
    change_address: &str,
    expiry_height: u32,
    private_key_bytes: &[u8],
    network_upgrade: ZcashNetworkUpgrade,
    dust_threshold_zats: u64,
) -> Result<Vec<u8>, String> {
    let change = super::accounting::checked_change(utxos.iter().map(|u| u.2), amount_sat, fee_sat)?;
    let mut outputs = vec![(p2pkh_script(&decode_zcash_address(to_address)?), amount_sat)];
    if change > dust_threshold_zats {
        outputs.push((p2pkh_script(&decode_zcash_address(change_address)?), change));
    }
    sign_transaction(
        utxos,
        &outputs,
        expiry_height,
        private_key_bytes,
        network_upgrade,
    )
    .map(|r| r.0)
}

pub(crate) fn sign_transaction(
    utxos: &[(String, u32, u64, Vec<u8>)],
    outputs: &[(Vec<u8>, u64)],
    expiry_height: u32,
    private_key_bytes: &[u8],
    network_upgrade: ZcashNetworkUpgrade,
) -> Result<(Vec<u8>, String), String> {
    use secp256k1::{Message, Secp256k1, SecretKey};
    let secp = Secp256k1::new();
    let key = SecretKey::from_slice(private_key_bytes).map_err(|e| e.to_string())?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &key).serialize();
    let expected_script = p2pkh_script(&crate::derivation::bitcoin::hash160(&pubkey_bytes));
    if utxos.is_empty() || utxos.iter().any(|u| u.3 != expected_script) {
        return Err("Zcash input does not belong to the signing key".into());
    }
    let secret_key = key;
    // Per-tx digests that are constant across all inputs.
    let prevouts_digest = compute_prevouts_digest(utxos)?;
    let amounts_digest = compute_amounts_digest(utxos);
    let scripts_digest = compute_scripts_digest(utxos);
    let sequence_digest = compute_sequence_digest(utxos.len());
    let outputs_digest = compute_outputs_digest(outputs);
    let header_digest = compute_header_digest(expiry_height, network_upgrade);
    let sapling_digest = compute_empty_sapling_digest();
    let orchard_digest = compute_empty_orchard_digest();

    let mut signed_inputs: Vec<Vec<u8>> = Vec::with_capacity(utxos.len());
    for (txid, vout, value, script_pubkey) in utxos {
        let txin_sig_digest = compute_txin_sig_digest(txid, *vout, *value, script_pubkey)?;
        let transparent_digest = compute_transparent_sig_digest(
            &prevouts_digest,
            &amounts_digest,
            &scripts_digest,
            &sequence_digest,
            &outputs_digest,
            &txin_sig_digest,
        );
        let sighash = compute_zip244_txid_digest(
            &header_digest,
            &transparent_digest,
            &sapling_digest,
            &orchard_digest,
            network_upgrade,
        );

        let msg = Message::from_digest_slice(&sighash).map_err(|e| e.to_string())?;
        let sig = secp.sign_ecdsa(&msg, &secret_key);
        let mut der = sig.serialize_der().to_vec();
        der.push(SIGHASH_ALL as u8);

        // P2PKH script_sig: <sig+sighash> <pubkey>.
        let mut script_sig = Vec::with_capacity(2 + der.len() + pubkey_bytes.len());
        script_sig.push(der.len() as u8);
        script_sig.extend_from_slice(&der);
        script_sig.push(pubkey_bytes.len() as u8);
        script_sig.extend_from_slice(&pubkey_bytes);

        let mut inp = Vec::new();
        inp.extend_from_slice(&decode_txid_le(txid)?);
        inp.extend_from_slice(&vout.to_le_bytes());
        inp.extend_from_slice(&varint(script_sig.len()));
        inp.extend_from_slice(&script_sig);
        inp.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        signed_inputs.push(inp);
    }

    // Final V5 transaction encoding.
    let mut raw = Vec::new();
    let header = TX_VERSION_V5 | TX_VERSION_OVERWINTERED;
    raw.extend_from_slice(&header.to_le_bytes());
    raw.extend_from_slice(&network_upgrade.version_group_id.to_le_bytes());
    raw.extend_from_slice(&network_upgrade.consensus_branch_id.to_le_bytes());
    raw.extend_from_slice(&0u32.to_le_bytes()); // nLockTime
    raw.extend_from_slice(&expiry_height.to_le_bytes());

    // Transparent bundle.
    raw.extend_from_slice(&varint(signed_inputs.len()));
    for inp in &signed_inputs {
        raw.extend_from_slice(inp);
    }
    raw.extend_from_slice(&varint(outputs.len()));
    for (s, val) in outputs {
        raw.extend_from_slice(&val.to_le_bytes());
        raw.extend_from_slice(&varint(s.len()));
        raw.extend_from_slice(s);
    }

    // Empty Sapling bundle: 0 spends + 0 outputs.
    raw.push(0x00);
    raw.push(0x00);

    // Empty Orchard bundle: 0 actions.
    raw.push(0x00);

    let mut transparent = Vec::new();
    transparent.extend(prevouts_digest);
    transparent.extend(sequence_digest);
    transparent.extend(outputs_digest);
    let transparent = blake2b_personalized(PERSONAL_TX_TRANSPARENT, &transparent);
    let mut txid = compute_zip244_txid_digest(
        &header_digest,
        &transparent,
        &sapling_digest,
        &orchard_digest,
        network_upgrade,
    );
    txid.reverse();
    Ok((raw, hex::encode(txid)))
}

// ── Sub-digests ───────────────────────────────────────────────────────────

fn compute_header_digest(expiry_height: u32, nu: ZcashNetworkUpgrade) -> [u8; 32] {
    let mut buf = Vec::with_capacity(20);
    let header = TX_VERSION_V5 | TX_VERSION_OVERWINTERED;
    buf.extend_from_slice(&header.to_le_bytes());
    buf.extend_from_slice(&nu.version_group_id.to_le_bytes());
    buf.extend_from_slice(&nu.consensus_branch_id.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes()); // nLockTime
    buf.extend_from_slice(&expiry_height.to_le_bytes());
    blake2b_personalized(PERSONAL_TX_HEADERS, &buf)
}

fn compute_prevouts_digest(utxos: &[(String, u32, u64, Vec<u8>)]) -> Result<[u8; 32], String> {
    let mut buf = Vec::with_capacity(36 * utxos.len());
    for (txid, vout, _, _) in utxos {
        buf.extend_from_slice(&decode_txid_le(txid)?);
        buf.extend_from_slice(&vout.to_le_bytes());
    }
    Ok(blake2b_personalized(PERSONAL_TX_PREVOUTS, &buf))
}

fn compute_amounts_digest(utxos: &[(String, u32, u64, Vec<u8>)]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(8 * utxos.len());
    for (_, _, value, _) in utxos {
        buf.extend_from_slice(&value.to_le_bytes());
    }
    blake2b_personalized(PERSONAL_TX_PER_INPUT_AMOUNTS, &buf)
}

fn compute_scripts_digest(utxos: &[(String, u32, u64, Vec<u8>)]) -> [u8; 32] {
    let mut buf = Vec::new();
    for (_, _, _, script) in utxos {
        buf.extend_from_slice(&varint(script.len()));
        buf.extend_from_slice(script);
    }
    blake2b_personalized(PERSONAL_TX_PER_INPUT_SCRIPTS, &buf)
}

fn compute_sequence_digest(n_inputs: usize) -> [u8; 32] {
    let mut buf = Vec::with_capacity(4 * n_inputs);
    for _ in 0..n_inputs {
        buf.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    }
    blake2b_personalized(PERSONAL_TX_SEQUENCE, &buf)
}

fn compute_outputs_digest(outputs: &[(Vec<u8>, u64)]) -> [u8; 32] {
    let mut buf = Vec::new();
    for (script, value) in outputs {
        buf.extend_from_slice(&value.to_le_bytes());
        buf.extend_from_slice(&varint(script.len()));
        buf.extend_from_slice(script);
    }
    blake2b_personalized(PERSONAL_TX_TRANSPARENT_OUTPUTS, &buf)
}

fn compute_txin_sig_digest(
    txid: &str,
    vout: u32,
    value: u64,
    script_pubkey: &[u8],
) -> Result<[u8; 32], String> {
    // ZIP-244 txin_sig_digest preimage:
    //   prevout (36) || value (8) || script_pubkey (with varint length) ||
    //   nSequence (4). Hash type belongs only in transparent_sig_digest.
    let mut buf = Vec::new();
    buf.extend_from_slice(&decode_txid_le(txid)?);
    buf.extend_from_slice(&vout.to_le_bytes());
    buf.extend_from_slice(&value.to_le_bytes());
    buf.extend_from_slice(&varint(script_pubkey.len()));
    buf.extend_from_slice(script_pubkey);
    buf.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); // nSequence
    Ok(blake2b_personalized(PERSONAL_TX_SIG_DIGEST, &buf))
}

#[allow(clippy::too_many_arguments)]
fn compute_transparent_sig_digest(
    prevouts_digest: &[u8; 32],
    amounts_digest: &[u8; 32],
    scripts_digest: &[u8; 32],
    sequence_digest: &[u8; 32],
    outputs_digest: &[u8; 32],
    txin_sig_digest: &[u8; 32],
) -> [u8; 32] {
    let mut combined = Vec::with_capacity(6 * 32 + 1);
    combined.push(SIGHASH_ALL as u8);
    combined.extend_from_slice(prevouts_digest);
    combined.extend_from_slice(amounts_digest);
    combined.extend_from_slice(scripts_digest);
    combined.extend_from_slice(sequence_digest);
    combined.extend_from_slice(outputs_digest);
    combined.extend_from_slice(txin_sig_digest);
    blake2b_personalized(PERSONAL_TX_TRANSPARENT, &combined)
}

fn compute_empty_sapling_digest() -> [u8; 32] {
    blake2b_personalized(PERSONAL_TX_SAPLING, &[])
}

fn compute_empty_orchard_digest() -> [u8; 32] {
    blake2b_personalized(PERSONAL_TX_ORCHARD, &[])
}

fn compute_zip244_txid_digest(
    header: &[u8; 32],
    transparent: &[u8; 32],
    sapling: &[u8; 32],
    orchard: &[u8; 32],
    nu: ZcashNetworkUpgrade,
) -> [u8; 32] {
    // Personalisation includes the consensus branch id in the trailing bytes:
    // "ZcashTxHash_" + LE-bytes(branch_id).
    let mut personal = [0u8; 16];
    personal[..PERSONAL_TX_TXID_BASE.len()].copy_from_slice(PERSONAL_TX_TXID_BASE);
    personal[12..16].copy_from_slice(&nu.consensus_branch_id.to_le_bytes());

    let mut buf = Vec::with_capacity(4 * 32);
    buf.extend_from_slice(header);
    buf.extend_from_slice(transparent);
    buf.extend_from_slice(sapling);
    buf.extend_from_slice(orchard);
    blake2b_personalized(&personal, &buf)
}

pub(crate) fn expiry_height(tip: u64) -> Result<u32, String> {
    let height = tip.checked_add(40).ok_or("expiry height overflow")?;
    u32::try_from(height).map_err(|_| "expiry height out of range".into())
}

#[cfg(test)]
mod expiry_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
    #[test]
    fn expiry_is_checked() {
        assert_eq!(expiry_height(2000000).unwrap(), 2000040);
        assert_eq!(expiry_height(u64::from(u32::MAX) - 40).unwrap(), u32::MAX);
        assert!(expiry_height(u64::from(u32::MAX) - 39).is_err());
        assert!(expiry_height(u64::MAX).is_err());
    }
    #[tokio::test]
    async fn unavailable_tip_never_broadcasts() {
        let server = MockServer::start().await;
        Mock::given(path("/api/v2/utxo/from"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        Mock::given(path("/api/v2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"backend":{}})),
            )
            .mount(&server)
            .await;
        let client =
            BlockbookClient::new(Arc::new(vec![server.uri()]), crate::registry::Chain::Zcash);
        let err = client
            .sign_zcash_and_broadcast("from", "to", 1, 1, &[1; 32], ZcashNetworkUpgrade::NU5, 546)
            .await
            .unwrap_err();
        assert!(err.contains("json decode"), "{err}");
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path() == "/api/v2"));
        assert!(server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !r.url.path().contains("sendtx")));
    }
}

#[cfg(test)]
mod zip244_tests {
    use super::*;
    #[test]
    fn transparent_signature_and_txid_match_official_python_reference() {
        // Generated with zcash/zcash-test-vectors zip_0244.py signature_digest
        // and txid_digest (NU6.2), not with the Rust implementation under test.
        let mut key = [0; 32];
        key[31] = 1;
        let script = hex::decode("76a914751e76e8199196d454941c45d1b3a323f1433bd688ac").unwrap();
        let inputs = vec![("11".repeat(32), 2, 1_000_000, script.clone())];
        let outputs = vec![(script.clone(), 100_000), (script, 890_000)];
        let (raw, txid) = sign_transaction(
            &inputs,
            &outputs,
            3_400_040,
            &key,
            ZcashNetworkUpgrade {
                version_group_id: 0x26a7_270a,
                consensus_branch_id: 0x5437_f330,
            },
        )
        .unwrap();
        assert_eq!(
            txid,
            "3fabcec39a66c0be46b8b0232a1065d9fbafcd2186afd67428b97fd88c2e316a"
        );
        // Header 20, input count 1, outpoint 36, script length 1, signature push 1.
        let sig_len = usize::from(raw[58]);
        assert_eq!(raw[59 + sig_len - 1], 1);
        let sig = secp256k1::ecdsa::Signature::from_der(&raw[59..59 + sig_len - 1]).unwrap();
        let digest =
            hex::decode("d011c516e0ae1972bf7ba228d4fb783ab71c72008b2080afec2e661babe6d479")
                .unwrap();
        let secp = secp256k1::Secp256k1::new();
        let public = secp256k1::PublicKey::from_secret_key(
            &secp,
            &secp256k1::SecretKey::from_slice(&key).unwrap(),
        );
        secp.verify_ecdsa(
            &secp256k1::Message::from_digest_slice(&digest).unwrap(),
            &sig,
            &public,
        )
        .unwrap();
        assert!(sign_transaction(
            &inputs,
            &outputs,
            3_400_040,
            &[2; 32],
            ZcashNetworkUpgrade::NU5
        )
        .is_err());
    }
}
