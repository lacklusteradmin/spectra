//! Zcash transparent send: build + sign V5 (ZIP-225) transactions and
//! broadcast via Trezor's Blockbook `/api/v2/sendtx`.
//!
//! Only transparent-only transactions are supported (empty Sapling and
//! Orchard bundles). Signing follows ZIP-244 (txid digest = personalised
//! BLAKE2b over header / transparent / sapling / orchard sub-digests).
//!
//! Network upgrade: NU5 mainnet — version_group_id `0x26A7270A`,
//! consensus_branch_id `0xC2D6D0B4`. We hardcode NU5 because the next
//! upgrade (NU6) requires a fresh sighash table and a code update anyway.

use crate::http::{with_fallback, RetryProfile};

use crate::derivation::chains::zcash::{decode_zcash_address, zcash_p2pkh_script};
use crate::fetch::chains::zcash::{ZcashClient, ZecSendResult};

// ── Network upgrade descriptor ────────────────────────────────────────────

/// Zcash consensus rule set for V5 transaction construction.
#[derive(Clone, Copy)]
pub struct ZcashNetworkUpgrade {
    pub version_group_id: u32,
    pub consensus_branch_id: u32,
}

impl ZcashNetworkUpgrade {
    /// NU5 mainnet (ZIP-225). Next upgrade requires a fresh sighash table anyway.
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

impl ZcashClient {
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<ZecSendResult, String> {
        let hex = hex_tx.to_string();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let hex = hex.clone();
            let url = format!("{}/api/v2/sendtx/", base.trim_end_matches('/'));
            async move {
                let raw_tx_hex = hex.clone();
                let txid: String = client
                    .post_text(&url, hex, RetryProfile::ChainWrite)
                    .await?;
                let txid = txid.trim().to_string();
                Ok(ZecSendResult { txid, raw_tx_hex })
            }
        })
        .await
    }

    /// Fetch UTXOs + chain tip, sign a V5 transparent transaction, broadcast.
    pub async fn sign_and_broadcast(
        &self,
        from_address: &str,
        to_address: &str,
        amount_sat: u64,
        fee_sat: u64,
        private_key_bytes: &[u8],
        network_upgrade: ZcashNetworkUpgrade,
        dust_threshold_zats: u64,
    ) -> Result<ZecSendResult, String> {
        let utxos = self.fetch_utxos(from_address).await?;
        let tip = self.fetch_chain_tip_height().await.unwrap_or(0);
        // Match zcashd default: 40-block expiry window.
        let expiry_height = (tip + 40) as u32;
        let from_hash = decode_zcash_address(from_address)?;
        let from_script = zcash_p2pkh_script(&from_hash);
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

fn varint(n: usize) -> Vec<u8> {
    match n {
        0..=0xfc => vec![n as u8],
        0xfd..=0xffff => {
            let mut v = vec![0xfd];
            v.extend_from_slice(&(n as u16).to_le_bytes());
            v
        }
        _ => {
            let mut v = vec![0xfe];
            v.extend_from_slice(&(n as u32).to_le_bytes());
            v
        }
    }
}

fn decode_txid(txid: &str) -> Result<Vec<u8>, String> {
    let mut bytes = hex::decode(txid).map_err(|e| format!("txid decode: {e}"))?;
    bytes.reverse();
    Ok(bytes)
}

fn dsha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(Sha256::digest(data)).into()
}

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

// ----------------------------------------------------------------
// ZIP-244 / ZIP-244-revised sighash construction (NU5 transparent-only).
// ----------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn sign_zcash_v5_p2pkh(
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
    use secp256k1::{Message, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key =
        SecretKey::from_slice(private_key_bytes).map_err(|e| format!("invalid key: {e}"))?;
    let pubkey_bytes = secp256k1::PublicKey::from_secret_key(&secp, &secret_key).serialize();

    let total_in: u64 = utxos.iter().map(|(_, _, v, _)| v).sum();
    let change = total_in.saturating_sub(amount_sat + fee_sat);

    let mut outputs: Vec<(Vec<u8>, u64)> = vec![(
        zcash_p2pkh_script(&decode_zcash_address(to_address)?),
        amount_sat,
    )];
    if change > dust_threshold_zats {
        outputs.push((
            zcash_p2pkh_script(&decode_zcash_address(change_address)?),
            change,
        ));
    }

    // Per-tx digests that are constant across all inputs.
    let prevouts_digest = compute_prevouts_digest(utxos)?;
    let amounts_digest = compute_amounts_digest(utxos);
    let scripts_digest = compute_scripts_digest(utxos);
    let sequence_digest = compute_sequence_digest(utxos.len());
    let outputs_digest = compute_outputs_digest(&outputs);
    let header_digest = compute_header_digest(expiry_height, network_upgrade);
    let sapling_digest = compute_empty_sapling_digest();
    let orchard_digest = compute_empty_orchard_digest();

    let mut signed_inputs: Vec<Vec<u8>> = Vec::with_capacity(utxos.len());
    for (input_index, (txid, vout, value, script_pubkey)) in utxos.iter().enumerate() {
        let txin_sig_digest = compute_txin_sig_digest(
            txid,
            *vout,
            *value,
            script_pubkey,
            input_index as u32,
        )?;
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
        inp.extend_from_slice(&decode_txid(txid)?);
        inp.extend_from_slice(&vout.to_le_bytes());
        inp.extend_from_slice(&varint(script_sig.len()));
        inp.extend_from_slice(&script_sig);
        inp.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        signed_inputs.push(inp);
    }

    // Final V5 transaction encoding.
    let mut raw = Vec::new();
    // Header.
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
    for (s, val) in &outputs {
        raw.extend_from_slice(&val.to_le_bytes());
        raw.extend_from_slice(&varint(s.len()));
        raw.extend_from_slice(s);
    }

    // Empty Sapling bundle: 0 spends + 0 outputs.
    raw.push(0x00);
    raw.push(0x00);

    // Empty Orchard bundle: 0 actions.
    raw.push(0x00);

    Ok(raw)
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
        buf.extend_from_slice(&decode_txid(txid)?);
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
    input_index: u32,
) -> Result<[u8; 32], String> {
    // ZIP-244 txin_sig_digest preimage:
    //   prevout (36) || value (8) || script_pubkey (with varint length) ||
    //   nSequence (4) || input_index (4) || hash_type (4)
    let mut buf = Vec::new();
    buf.extend_from_slice(&decode_txid(txid)?);
    buf.extend_from_slice(&vout.to_le_bytes());
    buf.extend_from_slice(&value.to_le_bytes());
    buf.extend_from_slice(&varint(script_pubkey.len()));
    buf.extend_from_slice(script_pubkey);
    buf.extend_from_slice(&0xffff_ffffu32.to_le_bytes()); // nSequence
    buf.extend_from_slice(&input_index.to_le_bytes());
    buf.extend_from_slice(&SIGHASH_ALL.to_le_bytes());
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
    let mut combined = Vec::with_capacity(7 * 32 + 4);
    combined.extend_from_slice(&[SIGHASH_ALL as u8, 0, 0, 0]);
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

// Suppress unused-warning for the legacy double-sha helper imported above.
// (We don't use it in V5 but `dsha256` is still referenced symbolically by
// readers comparing against the Bitcoin/Litecoin path.)
#[allow(dead_code)]
fn _legacy_dsha_unused(data: &[u8]) -> [u8; 32] {
    dsha256(data)
}

