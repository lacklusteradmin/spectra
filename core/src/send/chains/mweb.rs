//! MWEB (MimbleWimble Extension Blocks) peg-in transaction construction.
//!
//! A peg-in sends on-chain LTC to an `ltcmweb1` stealth address by:
//!   1. Adding a HogEx output (witness v8 + Pedersen commitment) to the
//!      standard Litecoin transaction.
//!   2. Appending an MWEB extension block (1 Output + 1 PegIn Kernel) after
//!      the transaction's locktime field.
//!
//! Cryptography follows Litecoin Core mweb branch src/libmw/ exactly:
//!   - BLAKE3 for all domain-separated hashes (EHashTag ASCII constants)
//!   - DKSAP stealth address protocol (nonce n → sending key s → shared t)
//!   - Pedersen commitments + Bulletproof range proofs via grin_secp256k1zkp
//!   - Schnorr signatures via secp256k1 0.29

use rand::RngCore;

use crate::derivation::chains::litecoin::MwebAddress;

// ── EHashTag constants (ASCII values from libmw/include/mw/crypto/Hasher.h) ──
const HTAG_DERIVE: u8 = b'D';   // shared secret: t = BLAKE3('D' | s·A)
const HTAG_NONCE: u8 = b'N';    // nonce:          n = BLAKE3('N' | k_s)[:16]
const HTAG_OUT_KEY: u8 = b'O';  // receiver key:   Ko = BLAKE3('O'|t) · B
const HTAG_SEND_KEY: u8 = b'S'; // sending key:    s  = BLAKE3('S'|A|B|v_le8|n)
const HTAG_TAG: u8 = b'T';      // view tag:        t[0] of BLAKE3('T'|s·A)

// MWEB kernel feature flags
const KERNEL_FEAT_HAS_FEE: u8 = 0x01;
const KERNEL_FEAT_PEGIN: u8 = 0x02;

/// Approximate byte overhead of the MWEB extension block for a single peg-in:
///   Output:  33+33+33 + 59 + ~677 + 64 ≈ 899 bytes
///   Kernel:  1+8+8+33+64 = 114 bytes
///   varints: 3 bytes
pub const MWEB_PEGIN_OVERHEAD_BYTES: u64 = 1017;

// ── BLAKE3 helpers ────────────────────────────────────────────────────────────

/// BLAKE3(tag_byte | data) → 32 bytes.
fn b3(tag: u8, data: &[u8]) -> [u8; 32] {
    blake3::Hasher::new().update(&[tag]).update(data).finalize().into()
}

/// BLAKE3(parts[0] | parts[1] | …) → 32 bytes, no tag prefix.
fn b3_cat(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// BLAKE3(data) → 64 bytes via XOF.
fn b3_64(data: &[u8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    blake3::Hasher::new().update(data).finalize_xof().fill(&mut out);
    out
}

// ── Stealth address output key derivation ────────────────────────────────────

struct MwebOutputKeys {
    /// k_s — ephemeral sender private key (for signing the output).
    sender_sk: [u8; 32],
    /// K_s = k_s·G — sender public key (Output.senderPubKey).
    sender_pubkey: [u8; 33],
    /// K_o = BLAKE3('O'|t)·B — receiver one-time key (Output.receiverPubKey).
    receiver_pubkey: [u8; 33],
    /// Blinding factor from HASH64(t)[0..32] for the Pedersen commitment.
    blinding: [u8; 32],
    /// Serialised OutputMessage (59 bytes): passed as extra_data to bulletproof.
    output_message: [u8; 59],
}

fn derive_output_keys(
    addr: &MwebAddress,
    value: u64,
    secp: &secp256k1zkp::Secp256k1,
) -> Result<MwebOutputKeys, String> {
    // Step 1: random sender keypair (k_s, K_s = k_s·G)
    let mut k_s = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k_s);
    let ks_sk = secp256k1zkp::key::SecretKey::from_slice(secp, &k_s)
        .map_err(|e| format!("mweb k_s: {e}"))?;
    let ks_pk = secp256k1zkp::key::PublicKey::from_secret_key(secp, &ks_sk)
        .map_err(|e| format!("mweb K_s: {e}"))?;
    let sender_pubkey = pk33(secp, &ks_pk)?;

    // Step 2: nonce n = BLAKE3('N' | k_s)[:16]
    let n: [u8; 16] = b3(HTAG_NONCE, &k_s)[..16].try_into().unwrap();

    // Step 3: sending key s = BLAKE3('S' | A | B | v_le8 | n)
    let s_bytes = b3_cat(&[
        &[HTAG_SEND_KEY],
        &addr.scan_pubkey,
        &addr.spend_pubkey,
        &value.to_le_bytes(),
        &n,
    ]);
    let s_sk = secp256k1zkp::key::SecretKey::from_slice(secp, &s_bytes)
        .map_err(|e| format!("mweb sending key s: {e}"))?;

    // Step 4: shared point sA = s · A_scan;  shared secret t = BLAKE3('D' | sA)
    let mut sa = secp256k1zkp::key::PublicKey::from_slice(secp, &addr.scan_pubkey)
        .map_err(|e| format!("mweb scan pubkey: {e}"))?;
    sa.mul_assign(secp, &s_sk).map_err(|e| format!("mweb sA: {e}"))?;
    let sa_compressed = pk33(secp, &sa)?;
    let t = b3(HTAG_DERIVE, &sa_compressed);

    // Step 5: K_o = BLAKE3('O' | t) · B_spend
    let ok_scalar = b3(HTAG_OUT_KEY, &t);
    let ok_sk = secp256k1zkp::key::SecretKey::from_slice(secp, &ok_scalar)
        .map_err(|e| format!("mweb out_key scalar: {e}"))?;
    let mut ko = secp256k1zkp::key::PublicKey::from_slice(secp, &addr.spend_pubkey)
        .map_err(|e| format!("mweb spend pubkey Ko: {e}"))?;
    ko.mul_assign(secp, &ok_sk).map_err(|e| format!("mweb Ko: {e}"))?;
    let receiver_pubkey = pk33(secp, &ko)?;

    // Step 6: K_e = s · B_spend  (key exchange pubkey, goes in OutputMessage)
    let mut ke = secp256k1zkp::key::PublicKey::from_slice(secp, &addr.spend_pubkey)
        .map_err(|e| format!("mweb spend pubkey Ke: {e}"))?;
    ke.mul_assign(secp, &s_sk).map_err(|e| format!("mweb Ke: {e}"))?;
    let key_exchange_pubkey = pk33(secp, &ke)?;

    // Step 7: view_tag = BLAKE3('T' | sA)[0]
    let view_tag = b3(HTAG_TAG, &sa_compressed)[0];

    // Step 8: 64-byte mask m = BLAKE3_64(t)
    let m = b3_64(&t);
    let blinding: [u8; 32] = m[0..32].try_into().unwrap();
    let value_mask: [u8; 8] = m[32..40].try_into().unwrap();
    let nonce_mask: [u8; 16] = m[40..56].try_into().unwrap();

    // Step 9: v' = v_le8 XOR m[32..40]
    let v_le = value.to_le_bytes();
    let masked_value: [u8; 8] = std::array::from_fn(|i| v_le[i] ^ value_mask[i]);

    // Step 10: n' = n XOR m[40..56]
    let masked_nonce: [u8; 16] = std::array::from_fn(|i| n[i] ^ nonce_mask[i]);

    // Serialise OutputMessage (59 bytes):
    //   features(1=0x01) | K_e(33) | view_tag(1) | v'(8) | n'(16)
    let mut msg = [0u8; 59];
    msg[0] = 0x01; // STANDARD_FIELDS_FEATURE_BIT
    msg[1..34].copy_from_slice(&key_exchange_pubkey);
    msg[34] = view_tag;
    msg[35..43].copy_from_slice(&masked_value);
    msg[43..59].copy_from_slice(&masked_nonce);

    Ok(MwebOutputKeys { sender_sk: k_s, sender_pubkey, receiver_pubkey, blinding, output_message: msg })
}

// ── MWEB Output construction ─────────────────────────────────────────────────

/// Build a serialised MWEB Output; also returns the commitment and blinding
/// factor (the caller needs the blinding to build the matching kernel).
fn build_output(
    addr: &MwebAddress,
    value: u64,
    secp: &secp256k1zkp::Secp256k1,
) -> Result<(Vec<u8>, [u8; 33], [u8; 32]), String> {
    let keys = derive_output_keys(addr, value, secp)?;

    // Pedersen commitment C = value·H + blinding·G
    let blind_sk = secp256k1zkp::key::SecretKey::from_slice(secp, &keys.blinding)
        .map_err(|e| format!("mweb blind sk: {e}"))?;
    let commit = secp
        .commit(value, blind_sk.clone())
        .map_err(|e| format!("mweb commit: {e}"))?;
    let commitment: [u8; 33] = commit.0;

    // Bulletproof range proof.
    // extra_data = serialised OutputMessage (per spec — binds proof to message).
    // Nonces are random; value recovery uses masked_value, not BP rewind.
    let mut rn = [0u8; 32];
    let mut pn = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut rn);
    rand::thread_rng().fill_bytes(&mut pn);
    let rewind_nonce = secp256k1zkp::key::SecretKey::from_slice(secp, &rn)
        .map_err(|e| format!("mweb rewind nonce: {e}"))?;
    let private_nonce = secp256k1zkp::key::SecretKey::from_slice(secp, &pn)
        .map_err(|e| format!("mweb private nonce: {e}"))?;
    let proof = secp.bullet_proof(
        value,
        blind_sk,
        rewind_nonce,
        private_nonce,
        Some(keys.output_message.to_vec()),
        None,
    );
    let proof_bytes = &proof.proof[..proof.plen];

    // Output signature pre-image (BLAKE3, no tag prefix):
    //   BLAKE3(C | K_s | K_o | BLAKE3(OutputMessage) | BLAKE3(proof_bytes))
    let msg_hash: [u8; 32] = blake3::hash(&keys.output_message).into();
    let proof_hash: [u8; 32] = blake3::hash(proof_bytes).into();
    let sig_msg = b3_cat(&[
        &commitment,
        &keys.sender_pubkey,
        &keys.receiver_pubkey,
        &msg_hash,
        &proof_hash,
    ]);
    let out_sig = schnorr_sign(&sig_msg, &keys.sender_sk)?;

    // Serialise Output (no top-level features byte):
    //   C(33) | K_s(33) | K_o(33) | OutputMessage(59) | proof_varint | proof | sig(64)
    let mut out = Vec::with_capacity(256 + proof_bytes.len());
    out.extend_from_slice(&commitment);
    out.extend_from_slice(&keys.sender_pubkey);
    out.extend_from_slice(&keys.receiver_pubkey);
    out.extend_from_slice(&keys.output_message);
    mweb_write_varint(&mut out, proof_bytes.len());
    out.extend_from_slice(proof_bytes);
    out.extend_from_slice(&out_sig);

    Ok((out, commitment, keys.blinding))
}

// ── MWEB Kernel (PegIn) construction ─────────────────────────────────────────

/// Builds a serialised MWEB PegIn kernel.
///
/// For a peg-in there are no MWEB inputs, so excess = blinding·G, satisfying
/// the MW balance:  C_out − v_pegin·H = blinding·G.
fn build_kernel(fee_sat: u64, pegin_amount: u64, blinding: &[u8; 32]) -> Result<Vec<u8>, String> {
    // excess = blinding·G  (secp256k1 compressed public key)
    let btc_secp = secp256k1::Secp256k1::signing_only();
    let bf_sk = secp256k1::SecretKey::from_slice(blinding)
        .map_err(|e| format!("mweb kernel excess sk: {e}"))?;
    let excess: [u8; 33] = secp256k1::PublicKey::from_secret_key(&btc_secp, &bf_sk).serialize();

    let features = KERNEL_FEAT_HAS_FEE | KERNEL_FEAT_PEGIN;

    // Kernel signature pre-image (BLAKE3, no tag prefix):
    //   features(1) | excess(33) | fee(8 LE) | pegin(8 LE)
    let sig_msg = b3_cat(&[
        &[features],
        &excess,
        &fee_sat.to_le_bytes(),
        &pegin_amount.to_le_bytes(),
    ]);
    let kernel_sig = schnorr_sign(&sig_msg, blinding)?;

    // Serialise kernel: features(1) | fee(8 LE) | pegin(8 LE) | excess(33) | sig(64)
    let mut kern = Vec::with_capacity(114);
    kern.push(features);
    kern.extend_from_slice(&fee_sat.to_le_bytes());
    kern.extend_from_slice(&pegin_amount.to_le_bytes());
    kern.extend_from_slice(&excess);
    kern.extend_from_slice(&kernel_sig);

    Ok(kern)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Builds the MWEB extension block for a peg-in send and the on-chain HogEx
/// output script.
///
/// Returns `(ext_block_bytes, hog_ex_script)`.
///
/// `ext_block_bytes` is appended after the standard tx locktime field.
/// `hog_ex_script` is `OP_8 PUSH_33 <commitment_33>` (witness v8).
pub fn build_peg_in_extension(
    addr: &MwebAddress,
    pegin_amount: u64,
    fee_sat: u64,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let secp = secp256k1zkp::Secp256k1::with_caps(secp256k1zkp::ContextFlag::Commit);

    let (output_bytes, commitment, blinding) = build_output(addr, pegin_amount, &secp)?;
    let kernel_bytes = build_kernel(fee_sat, pegin_amount, &blinding)?;

    // MWEB TxBody: varint(0 inputs) varint(1 output) [output] varint(1 kernel) [kernel]
    let mut ext = Vec::with_capacity(output_bytes.len() + kernel_bytes.len() + 8);
    mweb_write_varint(&mut ext, 0);
    mweb_write_varint(&mut ext, 1);
    ext.extend_from_slice(&output_bytes);
    mweb_write_varint(&mut ext, 1);
    ext.extend_from_slice(&kernel_bytes);

    // HogEx script: OP_8 PUSH_33 <commitment>  (witness version 8, 33-byte program)
    let mut hog_script = Vec::with_capacity(35);
    hog_script.push(0x58); // OP_8
    hog_script.push(0x21); // PUSH 33 bytes
    hog_script.extend_from_slice(&commitment);

    Ok((ext, hog_script))
}

// ── Schnorr signing (secp256k1 0.29) ─────────────────────────────────────────

fn schnorr_sign(msg_hash: &[u8; 32], sk_bytes: &[u8; 32]) -> Result<[u8; 64], String> {
    use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
    let secp = Secp256k1::signing_only();
    let sk = SecretKey::from_slice(sk_bytes).map_err(|e| format!("schnorr_sign sk: {e}"))?;
    let msg = Message::from_digest_slice(msg_hash).map_err(|e| format!("schnorr_sign msg: {e}"))?;
    let kp = Keypair::from_secret_key(&secp, &sk);
    Ok(secp.sign_schnorr(&msg, &kp).serialize())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn pk33(secp: &secp256k1zkp::Secp256k1, pk: &secp256k1zkp::key::PublicKey) -> Result<[u8; 33], String> {
    pk.serialize_vec(secp, true).as_slice()[..33]
        .try_into()
        .map_err(|_| "pk33: unexpected length".to_string())
}

fn mweb_write_varint(buf: &mut Vec<u8>, n: usize) {
    match n {
        0..=0xfc => buf.push(n as u8),
        0xfd..=0xffff => {
            buf.push(0xfd);
            buf.extend_from_slice(&(n as u16).to_le_bytes());
        }
        _ => {
            buf.push(0xfe);
            buf.extend_from_slice(&(n as u32).to_le_bytes());
        }
    }
}
