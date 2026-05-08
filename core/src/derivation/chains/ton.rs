//! TON address decode + validation + derivation. Self-contained — see
//! `REFACTOR_NOTES.md`.
//!
//! - `decode_ton_address` / `validate_ton_address`: parse raw or
//!   base64url user-friendly addresses.
//! - `derive_ton_seed`: TON mnemonic (ton-crypto / TonKeeper / Tonhub) PBKDF2.
//! - The v4R2 path computes the wallet's account id from the embedded
//!   wallet code BOC + a freshly-built data cell. Correctness is locked
//!   by `v4r2_code_hash_and_depth`'s self-test against the published
//!   v4R2 code hash.

use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;


type HmacSha512 = Hmac<Sha512>;

fn sha256_bytes(input: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let out = hasher.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

fn hmac_sha512(key: &[u8], chunks: &[&[u8]]) -> Result<Zeroizing<[u8; 64]>, String> {
    let mut mac = HmacSha512::new_from_slice(key)
        .map_err(|error| format!("Invalid HMAC-SHA512 key: {error}"))?;
    for chunk in chunks {
        mac.update(chunk);
    }
    let tag = mac.finalize().into_bytes();
    let mut out = Zeroizing::new([0u8; 64]);
    out.copy_from_slice(&tag);
    Ok(out)
}

pub(crate) fn decode_ton_address(address: &str) -> Result<(i8, [u8; 32]), String> {
    // TON addresses can be in raw form (workchain:hex) or user-friendly base64url.
    if address.contains(':') {
        let parts: Vec<&str> = address.splitn(2, ':').collect();
        let workchain: i8 = parts[0].parse().map_err(|e| format!("wc: {e}"))?;
        let bytes = hex::decode(parts[1]).map_err(|e| format!("addr hex: {e}"))?;
        if bytes.len() != 32 {
            return Err("addr wrong len".to_string());
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        return Ok((workchain, arr));
    }

    // User-friendly: 36 bytes base64url = [flags(1)] + [wc(1)] + [addr(32)] + [crc(2)]
    let normalized = address.replace('-', "+").replace('_', "/");
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&normalized)
        .map_err(|e| format!("base64 decode: {e}"))?;
    if decoded.len() != 36 {
        return Err(format!("TON address wrong length: {}", decoded.len()));
    }
    let workchain = decoded[1] as i8;
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&decoded[2..34]);
    Ok((workchain, arr))
}

// ── TON mnemonic seed expansion ──────────────────────────────────────────

pub(crate) fn derive_ton_seed(
    mnemonic: &str,
    passphrase: &str,
    salt_prefix: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 64]>, String> {
    // TON mnemonic scheme (ton-crypto / TonKeeper / Tonhub):
    //   entropy = HMAC-SHA512(key = mnemonic_string, data = passphrase_bytes)
    //   seed    = PBKDF2-HMAC-SHA512(entropy, salt = "TON default seed",
    //                                 iterations = 100_000, dklen = 64)
    //   priv    = seed[0..32]
    //
    // `salt_prefix` and `iteration_count` are honored as customization
    // points — defaults match the ton-crypto reference implementation.
    let entropy = hmac_sha512(mnemonic.as_bytes(), &[passphrase.as_bytes()])?;
    let iterations = if iteration_count == 0 { 100_000 } else { iteration_count };
    let salt = salt_prefix.unwrap_or("TON default seed");
    let mut seed = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(&*entropy, salt.as_bytes(), iterations, &mut *seed);
    Ok(seed)
}

// ── v4R2 wallet contract code (embedded BOC) ─────────────────────────────
// The wallet contract bytes-of-code, copied from ton-core's
// `WalletContractV4R2.js`. Correctness is locked by a self-test that
// asserts the recomputed root cell hash matches the well-known public
// constant `feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0`.

const V4R2_CODE_BOC_HEX: &str = "b5ee9c7241021401000\
2d4000114ff00f4a413f4bcf2c80b010201200203020148040504f8f28308d71820\
d31fd31fd31f02f823bbf264ed44d0d31fd31fd3fff404d15143baf2a15151baf2a\
205f901541064f910f2a3f80024a4c8cb1f5240cb1f5230cbff5210f400c9ed54f8\
0f01d30721c0009f6c519320d74a96d307d402fb00e830e021c001e30021c002e30\
001c0039130e30d03a4c8cb1f12cb1fcbff1011121302e6d001d0d3032171b0925f\
04e022d749c120925f04e002d31f218210706c7567bd22821064737472bdb0925f0\
5e003fa403020fa4401c8ca07cbffc9d0ed44d0810140d721f404305c810108f40a\
6fa131b3925f07e005d33fc8258210706c7567ba923830e30d03821064737472ba9\
25f06e30d06070201200809007801fa00f40430f8276f2230500aa121bef2e05082\
10706c7567831eb17080185004cb0526cf1658fa0219f400cb6917cb1f5260cb3f2\
0c98040fb0006008a5004810108f45930ed44d0810140d720c801cf16f400c9ed54\
0172b08e23821064737472831eb17080185005cb055003cf1623fa0213cb6acb1fc\
b3fc98040fb00925f03e20201200a0b0059bd242b6f6a2684080a06b90fa0218470\
d4080847a4937d29910ce6903e9ff9837812801b7810148987159f31840201580c0\
d0011b8c97ed44d0d70b1f8003db29dfb513420405035c87d010c00b23281f2fff2\
74006040423d029be84c600201200e0f0019adce76a26840206b90eb85ffc00019a\
f1df6a26840106b90eb858fc0006ed207fa00d4d422f90005c8ca0715cbffc9d077\
748018c8cb05cb0222cf165005fa0214cb6b12ccccc973fb00c84014810108f451f\
2a7020070810108d718fa00d33fc8542047810108f451f2a782106e6f7465707480\
18c8cb05cb025006cf165004fa0214cb6a12cb1fcb3fc973fb0002006c810108d71\
8fa00d33f305224810108f459f2a782106473747270748018c8cb05cb025005cf16\
5003fa0213cb6acb1f12cb3fc973fb00000af400c9ed54696225e5";

const V4R2_KNOWN_CODE_HASH: [u8; 32] = [
    0xfe, 0xb5, 0xff, 0x68, 0x20, 0xe2, 0xff, 0x0d, 0x94, 0x83, 0xe7, 0xe0,
    0xd6, 0x2c, 0x81, 0x7d, 0x84, 0x67, 0x89, 0xfb, 0x4a, 0xe5, 0x80, 0xc8,
    0x78, 0x86, 0x6d, 0x95, 0x9d, 0xab, 0xd5, 0xc0,
];

// Default subwallet id for v4 on the basic workchain (0). Hardcoded in
// every popular wallet (tonkeeper, tonhub, tonweb) so anyone generating
// a v4R2 address from the same mnemonic produces the same address.
const V4R2_DEFAULT_WALLET_ID: u32 = 698983191;

#[derive(Clone)]
struct ParsedCell {
    d1: u8,
    d2: u8,
    data: Vec<u8>,
    refs: Vec<usize>,
}

/// Minimal parser for BOC v0 (`b5ee9c72`) carrying ordinary (non-exotic,
/// level-0) cells. Supports the index and crc32c flags but validates
/// neither; the parser's correctness is instead locked by a cell-hash
/// self-test.
fn parse_boc(bytes: &[u8]) -> Result<(Vec<ParsedCell>, usize), String> {
    if bytes.len() < 6 || bytes[0..4] != [0xb5, 0xee, 0x9c, 0x72] {
        return Err("TON BOC: missing magic".to_string());
    }
    let flags = bytes[4];
    let has_idx = (flags & 0x80) != 0;
    let _has_crc32c = (flags & 0x40) != 0;
    let ref_size = (flags & 0x07) as usize;
    if ref_size == 0 || ref_size > 4 {
        return Err(format!("TON BOC: invalid ref size {ref_size}"));
    }
    let off_size = bytes[5] as usize;
    if off_size == 0 || off_size > 8 {
        return Err(format!("TON BOC: invalid offset size {off_size}"));
    }
    let mut cursor = 6usize;
    let read_uint = |buf: &[u8], off: usize, n: usize| -> Result<u64, String> {
        if off + n > buf.len() {
            return Err("TON BOC: unexpected EOF".to_string());
        }
        let mut v = 0u64;
        for &b in &buf[off..off + n] {
            v = (v << 8) | u64::from(b);
        }
        Ok(v)
    };
    let cell_count = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size;
    let root_count = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size;
    let _absent = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size;
    let _tot_cell_size = read_uint(bytes, cursor, off_size)? as usize;
    cursor += off_size;
    if root_count == 0 {
        return Err("TON BOC: no roots".to_string());
    }
    let root_idx = read_uint(bytes, cursor, ref_size)? as usize;
    cursor += ref_size * root_count;
    if has_idx {
        cursor += cell_count * off_size;
    }
    let mut cells = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        if cursor + 2 > bytes.len() {
            return Err("TON BOC: cell header EOF".to_string());
        }
        let d1 = bytes[cursor];
        let d2 = bytes[cursor + 1];
        cursor += 2;
        let refs_count = (d1 & 0x07) as usize;
        let exotic = (d1 & 0x08) != 0;
        let level = (d1 >> 5) & 0x03;
        if exotic || level != 0 {
            return Err("TON BOC: exotic or leveled cells not supported".to_string());
        }
        let data_len = (d2 as usize).div_ceil(2);
        if cursor + data_len > bytes.len() {
            return Err("TON BOC: cell data EOF".to_string());
        }
        let data = bytes[cursor..cursor + data_len].to_vec();
        cursor += data_len;
        let mut refs = Vec::with_capacity(refs_count);
        for _ in 0..refs_count {
            refs.push(read_uint(bytes, cursor, ref_size)? as usize);
            cursor += ref_size;
        }
        cells.push(ParsedCell { d1, d2, data, refs });
    }
    Ok((cells, root_idx))
}

/// Recursively compute SHA-256 cell hashes and depths for every cell,
/// bottom-up. BOC v0 orders cells such that every ref points to a higher
/// index, so iterating from the tail means every ref is resolved by the
/// time we reach the cell that uses it.
fn compute_cell_hashes(cells: &[ParsedCell]) -> Vec<([u8; 32], u16)> {
    let mut out = vec![([0u8; 32], 0u16); cells.len()];
    for i in (0..cells.len()).rev() {
        let cell = &cells[i];
        let mut repr = Vec::with_capacity(2 + cell.data.len() + cell.refs.len() * 34);
        repr.push(cell.d1);
        repr.push(cell.d2);
        repr.extend_from_slice(&cell.data);
        let mut depth = 0u16;
        for &r in &cell.refs {
            repr.extend_from_slice(&out[r].1.to_be_bytes());
            depth = depth.max(out[r].1.saturating_add(1));
        }
        for &r in &cell.refs {
            repr.extend_from_slice(&out[r].0);
        }
        let hash = sha256_bytes(&repr);
        out[i] = (hash, depth);
    }
    out
}

/// Returns (code_hash, code_depth) for the embedded v4R2 wallet code
/// cell, computed once per process.
pub(crate) fn v4r2_code_hash_and_depth() -> Result<([u8; 32], u16), String> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Result<([u8; 32], u16), String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let boc = hex::decode(V4R2_CODE_BOC_HEX)
                .map_err(|e| format!("TON v4R2: invalid embedded BOC hex: {e}"))?;
            let (cells, root) = parse_boc(&boc)?;
            let hashes = compute_cell_hashes(&cells);
            let (hash, depth) = hashes[root];
            if hash != V4R2_KNOWN_CODE_HASH {
                return Err(format!(
                    "TON v4R2: computed code hash {} does not match known constant",
                    hex::encode(hash)
                ));
            }
            Ok((hash, depth))
        })
        .clone()
}

/// Build the v4R2 data cell (321 bits, no refs) carrying the initial
/// seqno, subwallet id, public key, and empty plugin dict, and return its
/// cell hash and depth.
fn v4r2_data_cell_hash(public_key: &[u8; 32]) -> ([u8; 32], u16) {
    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&V4R2_DEFAULT_WALLET_ID.to_be_bytes());
    data.extend_from_slice(public_key);
    data.push(0x40);
    let mut repr = Vec::with_capacity(2 + data.len());
    repr.push(0x00);
    repr.push(81);
    repr.extend_from_slice(&data);
    let hash = sha256_bytes(&repr);
    (hash, 0)
}

/// Build the state_init cell for v4R2 (5 header bits + 2 refs: code,
/// data) and return its cell hash — which is the TON account id.
fn v4r2_state_init_account_id(public_key: &[u8; 32]) -> Result<[u8; 32], String> {
    let (code_hash, code_depth) = v4r2_code_hash_and_depth()?;
    let (data_hash, data_depth) = v4r2_data_cell_hash(public_key);
    let header_byte: u8 = 0x34;
    let mut repr = Vec::with_capacity(2 + 1 + 2 * (2 + 32));
    repr.push(0x02);
    repr.push(0x01);
    repr.push(header_byte);
    repr.extend_from_slice(&code_depth.to_be_bytes());
    repr.extend_from_slice(&data_depth.to_be_bytes());
    repr.extend_from_slice(&code_hash);
    repr.extend_from_slice(&data_hash);
    Ok(sha256_bytes(&repr))
}

/// CRC-16/XMODEM (poly=0x1021, init=0x0000, no reflection, no xor-out),
/// as required by TON user-friendly address checksums.
pub(crate) fn crc16_xmodem(bytes: &[u8]) -> u16 {
    const CRC: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
    CRC.checksum(bytes)
}

fn derive_ton_v4r2_address(public_key: &[u8; 32]) -> Result<String, String> {
    let account_id = v4r2_state_init_account_id(public_key)?;
    // tag 0x11 = bounceable, not-test; workchain 0x00 = basic workchain.
    let mut buf = [0u8; 36];
    buf[0] = 0x11;
    buf[1] = 0x00;
    buf[2..34].copy_from_slice(&account_id);
    let crc = crc16_xmodem(&buf[..34]);
    buf[34..36].copy_from_slice(&crc.to_be_bytes());
    use base64::Engine;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

/// Derive a TON V4R2 bounceable mainnet address from a TON-style mnemonic.
pub(crate) fn derive_ton_standard(
    seed_phrase: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let seed = derive_ton_seed(seed_phrase, passphrase.unwrap_or(""), None, 0)?;
    let mut private_key = [0u8; 32];
    private_key.copy_from_slice(&seed[..32]);
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = signing_key.verifying_key().to_bytes();

    let address = if want_address {
        Some(derive_ton_v4r2_address(&public_key)?)
    } else {
        None
    };

    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key)),
        want_private_key.then(|| hex::encode(private_key)),
    ))
}

// ── UniFFI exports ────────────────────────────────────────────────────────

use crate::derivation::types::DerivationResult;
use crate::SpectraBridgeError;

fn ton_internal(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (address, public_key_hex, private_key_hex) = derive_ton_standard(
        &seed_phrase, passphrase.as_deref(),
        want_address, want_public_key, want_private_key,
    )?;
    Ok(DerivationResult { address, public_key_hex, private_key_hex, account: 0, branch: 0, index: 0 })
}

#[uniffi::export]
pub fn derive_ton(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    ton_internal(seed_phrase, passphrase, want_address, want_public_key, want_private_key)
}

#[uniffi::export]
pub fn derive_ton_testnet(
    seed_phrase: String, passphrase: Option<String>,
    want_address: bool, want_public_key: bool, want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    ton_internal(seed_phrase, passphrase, want_address, want_public_key, want_private_key)
}
