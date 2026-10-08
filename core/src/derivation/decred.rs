//! Decred: address validation, BIP-32 derivation, P2PKH (Ds…) BLAKE-256
//! base58check encoding
//!
//! Decred uses BLAKE-256 (SHA-3 finalist family — NOT BLAKE2) wherever
//! Bitcoin uses double-SHA256:
//!   * `Hash160(pub) = RIPEMD-160(BLAKE-256(pub))` — address hash
//!   * Base58Check checksum = first 4 bytes of `BLAKE-256(BLAKE-256(payload))`
//!
//! Mainnet P2PKH addresses use the 2-byte version prefix `0x073F` (the
//! `Ds…` family); testnet uses `Ts…`. Simnet is out of scope.

use crate::derivation::error::DerivationError;
use crate::derivation::utxo_address::ParsedUtxoAddress;
use crate::registry::Chain;

use crate::derivation::primitives::derive_bip39_seed;
use ripemd::{Digest as RipemdDigest, Ripemd160};
use secp256k1::{PublicKey, Secp256k1};

// ── BLAKE-256 (SHA-3 finalist BLAKE-1 family) ─────────────────────────────

const BLAKE256_IV: [u32; 8] = [
    0x6A09_E667,
    0xBB67_AE85,
    0x3C6E_F372,
    0xA54F_F53A,
    0x510E_527F,
    0x9B05_688C,
    0x1F83_D9AB,
    0x5BE0_CD19,
];

const BLAKE256_C: [u32; 16] = [
    0x243F_6A88,
    0x85A3_08D3,
    0x1319_8A2E,
    0x0370_7344,
    0xA409_3822,
    0x299F_31D0,
    0x082E_FA98,
    0xEC4E_6C89,
    0x4528_21E6,
    0x38D0_1377,
    0xBE54_66CF,
    0x34E9_0C6C,
    0xC0AC_29B7,
    0xC97C_50DD,
    0x3F84_D5B5,
    0xB547_0917,
];

const BLAKE256_SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

#[inline(always)]
// BLAKE-256 G mixing function: one quarter-round of the BLAKE internal permutation.
fn g_mix(
    state: &mut [u32; 16],
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    m: &[u32; 16],
    r: usize,
    e: usize,
) {
    let row = &BLAKE256_SIGMA[r % 10];
    state[a] = state[a]
        .wrapping_add(state[b])
        .wrapping_add(m[row[2 * e]] ^ BLAKE256_C[row[2 * e + 1]]);
    state[d] = (state[d] ^ state[a]).rotate_right(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(12);
    state[a] = state[a]
        .wrapping_add(state[b])
        .wrapping_add(m[row[2 * e + 1]] ^ BLAKE256_C[row[2 * e]]);
    state[d] = (state[d] ^ state[a]).rotate_right(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_right(7);
}

// BLAKE-256 block compression: update chaining state h with one 512-bit message block.
fn blake256_compress(h: &mut [u32; 8], block: &[u8; 64], t0: u32, t1: u32) {
    let mut m = [0u32; 16];
    for i in 0..16 {
        m[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    let mut v = [0u32; 16];
    v[..8].copy_from_slice(h);
    v[8] = BLAKE256_C[0];
    v[9] = BLAKE256_C[1];
    v[10] = BLAKE256_C[2];
    v[11] = BLAKE256_C[3];
    v[12] = t0 ^ BLAKE256_C[4];
    v[13] = t0 ^ BLAKE256_C[5];
    v[14] = t1 ^ BLAKE256_C[6];
    v[15] = t1 ^ BLAKE256_C[7];

    for r in 0..14 {
        g_mix(&mut v, 0, 4, 8, 12, &m, r, 0);
        g_mix(&mut v, 1, 5, 9, 13, &m, r, 1);
        g_mix(&mut v, 2, 6, 10, 14, &m, r, 2);
        g_mix(&mut v, 3, 7, 11, 15, &m, r, 3);
        g_mix(&mut v, 0, 5, 10, 15, &m, r, 4);
        g_mix(&mut v, 1, 6, 11, 12, &m, r, 5);
        g_mix(&mut v, 2, 7, 8, 13, &m, r, 6);
        g_mix(&mut v, 3, 4, 9, 14, &m, r, 7);
    }

    for i in 0..8 {
        h[i] ^= v[i] ^ v[i + 8];
    }
}

/// Compute BLAKE-256 of `data`. Decred uses zero salt; we fold that constant
/// into the compression function to keep the public surface trivial.
pub(crate) fn blake256(data: &[u8]) -> [u8; 32] {
    let mut h = BLAKE256_IV;
    let total_bits: u64 = (data.len() as u64).wrapping_mul(8);
    let full_blocks = data.len() / 64;
    let mut t0: u32 = 0;
    let mut t1: u32 = 0;

    // Full-block compression. `t` tracks total bits consumed *including* the
    // current block.
    for i in 0..full_blocks {
        let mut block = [0u8; 64];
        block.copy_from_slice(&data[i * 64..(i + 1) * 64]);
        let (new_t0, carry) = t0.overflowing_add(512);
        t0 = new_t0;
        if carry {
            t1 = t1.wrapping_add(1);
        }
        blake256_compress(&mut h, &block, t0, t1);
    }

    // Padding rule (BLAKE-256, big-endian length suffix):
    //   message ++ 0x80 ++ zeros ++ 0x01 ++ length(8 bytes BE) so total ≡ 0 mod 512 bits
    let remaining = &data[full_blocks * 64..];
    let rem_len = remaining.len();
    let rem_bits = (rem_len as u32) * 8;

    let mut last = [0u8; 64];
    last[..rem_len].copy_from_slice(remaining);
    last[rem_len] = 0x80;

    if rem_len < 55 {
        // Single padding block. The bit counter for this block is the
        // total-message-bits-after-prefix, i.e., previous t plus rem_bits —
        // BUT if rem_len == 0 (no message bits in this block) we set t=0.
        let (final_t0, final_t1) = if rem_len == 0 {
            (0, 0)
        } else {
            let (nt0, c) = t0.overflowing_add(rem_bits);
            (nt0, if c { t1.wrapping_add(1) } else { t1 })
        };
        last[55] |= 0x01;
        last[56..64].copy_from_slice(&total_bits.to_be_bytes());
        blake256_compress(&mut h, &last, final_t0, final_t1);
    } else {
        // Two padding blocks. First block carries the message-bit accounting,
        // second block is length-only with t = 0.
        let (nt0, c) = t0.overflowing_add(rem_bits);
        let first_t0 = nt0;
        let first_t1 = if c { t1.wrapping_add(1) } else { t1 };
        blake256_compress(&mut h, &last, first_t0, first_t1);

        let mut tail = [0u8; 64];
        tail[55] |= 0x01;
        tail[56..64].copy_from_slice(&total_bits.to_be_bytes());
        blake256_compress(&mut h, &tail, 0, 0);
    }

    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// `RIPEMD-160(BLAKE-256(data))` — Decred's hash160 primitive.
pub(crate) fn dcr_hash160(data: &[u8]) -> [u8; 20] {
    let inner = blake256(data);
    let mut hasher = Ripemd160::new();
    RipemdDigest::update(&mut hasher, inner);
    let out = hasher.finalize();
    let mut hash = [0u8; 20];
    hash.copy_from_slice(&out);
    hash
}

/// Decred-flavoured base58check: 4-byte checksum is `BLAKE-256(BLAKE-256(payload))[..4]`,
/// not Bitcoin's `SHA-256(SHA-256(...))`.
pub(crate) fn dcr_base58check_encode(payload: &[u8]) -> String {
    let mut full = Vec::with_capacity(payload.len() + 4);
    full.extend_from_slice(payload);
    let hh = blake256(&blake256(payload));
    full.extend_from_slice(&hh[..4]);
    bs58::encode(full).into_string()
}

// Decode a Decred base58check string: strip the 4-byte BLAKE-256² checksum and return the payload.
pub(crate) fn dcr_base58check_decode(input: &str) -> Result<Vec<u8>, DerivationError> {
    let raw = bs58::decode(input)
        .into_vec()
        .map_err(|e| DerivationError::Invalid(format!("dcr base58 decode: {e}").into()))?;
    if raw.len() < 5 {
        return Err(DerivationError::Invalid(
            "dcr base58check payload too short".into(),
        ));
    }
    let split = raw.len() - 4;
    let payload = &raw[..split];
    let checksum = &raw[split..];
    let expected = blake256(&blake256(payload));
    if &expected[..4] != checksum {
        return Err(DerivationError::Invalid(
            "dcr base58check checksum mismatch".into(),
        ));
    }
    Ok(payload.to_vec())
}

/// Encode the P2PKH address of a 20-byte pubkey hash on `chain`'s network:
/// `Ds…` on mainnet, `Ts…` on testnet.
pub(crate) fn encode_decred_p2pkh(
    chain: Chain,
    pubkey_hash: &[u8; 20],
) -> Result<String, DerivationError> {
    let (p2pkh, _) = chain.decred_address_versions()?;
    let mut payload = Vec::with_capacity(22);
    payload.extend_from_slice(&p2pkh);
    payload.extend_from_slice(pubkey_hash);
    Ok(dcr_base58check_encode(&payload))
}

/// Decode a Decred address on `chain`'s network into the script it pays.
///
/// Returned only a hash, for `Ds…` and `Dc…` alike, so every send paid a
/// P2PKH output whatever the address named: a payment to a script hash went
/// to a key hash nobody holds. Only secp256k1 ECDSA pubkey hash and script
/// hash are payable here; Decred's other forms — pay-to-pubkey (`Dk…`),
/// Ed25519 (`De…`) and Schnorr (`DS…`) pubkey hash — and every address of
/// another network are refused.
pub(crate) fn parse_decred_address(
    chain: Chain,
    address: &str,
) -> Result<ParsedUtxoAddress, DerivationError> {
    let (p2pkh, p2sh) = chain.decred_address_versions()?;
    let refused =
        || DerivationError::invalid("Not a Decred P2PKH or P2SH address on the selected network");
    let payload = dcr_base58check_decode(address.trim()).map_err(|_| refused())?;
    let (version, hash) = payload.split_first_chunk::<2>().ok_or_else(refused)?;
    let hash: [u8; 20] = hash.try_into().map_err(|_| refused())?;
    if *version == p2pkh {
        Ok(ParsedUtxoAddress::P2pkh(hash))
    } else if *version == p2sh {
        Ok(ParsedUtxoAddress::P2sh(hash))
    } else {
        Err(refused())
    }
}

// ── BIP-32 ───────────────────────────────────────────────────────────────

use crate::derivation::primitives::parse_bip32_path;

// BIP-39 → BIP-32 path walk → (compressed secp256k1 pubkey, raw 32-byte private key).
fn derive_secp_keypair(
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
) -> Result<(PublicKey, [u8; 32]), DerivationError> {
    let secp = Secp256k1::new();
    let seed = derive_bip39_seed(seed_phrase, passphrase.unwrap_or(""), 0, None, None)?;
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", seed.as_ref())?;
    let path = parse_bip32_path(derivation_path)?;
    let xpriv = master.derive_path(&secp, &path)?;
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    Ok((public_key, xpriv.private_key.secret_bytes()))
}

// Derive the Decred P2PKH address on `chain`'s network, public key and
// private key from a mnemonic.
fn derive_from_seed_phrase(
    chain: Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<crate::derivation::primitives::OptionalKeyMaterial, DerivationError> {
    let (public_key, private_bytes) =
        derive_secp_keypair(seed_phrase, derivation_path, passphrase)?;
    let address = if want_address {
        Some(encode_decred_p2pkh(
            chain,
            &dcr_hash160(&public_key.serialize()),
        )?)
    } else {
        None
    };
    Ok((
        address,
        want_public_key.then(|| hex::encode(public_key.serialize())),
        want_private_key.then(|| hex::encode(private_bytes)),
    ))
}

// ── Derivation entry points ────────────────────────────────────────────────────────

use crate::SpectraBridgeError;
use crate::derivation::primitives::ExtendedPrivateKey;
use crate::derivation::types::{DerivationResult, parse_path_metadata};

/// Derive Decred mainnet wallet (Ds… P2PKH address) from a seed phrase.
pub fn derive_decred(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        Chain::Decred,
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account,
        branch,
        index,
    })
}

/// Derive Decred testnet wallet (Ts… P2PKH address) from a seed phrase.
pub fn derive_decred_testnet(
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    let (account, branch, index) = parse_path_metadata(&derivation_path);
    let (address, public_key_hex, private_key_hex) = derive_from_seed_phrase(
        Chain::DecredTestnet,
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
        want_address,
        want_public_key,
        want_private_key,
    )?;
    Ok(DerivationResult {
        address,
        public_key_hex,
        private_key_hex,
        account,
        branch,
        index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake256_known_vectors() {
        // Empty string vector from BLAKE reference: BLAKE-256("") =
        // 716f6e863f744b9ac22c97ec7b76ea5f5908bc5b2f67c61510bfc4751384ea7a
        assert_eq!(
            hex::encode(blake256(b"")),
            "716f6e863f744b9ac22c97ec7b76ea5f5908bc5b2f67c61510bfc4751384ea7a"
        );
        // BLAKE-256("abc") = 1833a9fa7cf4086bd5fda73da32e5a1d75b4c3f89d5c436369f9d78bb2da5c28
        assert_eq!(
            hex::encode(blake256(b"abc")),
            "1833a9fa7cf4086bd5fda73da32e5a1d75b4c3f89d5c436369f9d78bb2da5c28"
        );
    }

    /// dcrd's own address vectors: each address with the script it pays.
    /// https://github.com/decred/dcrd/blob/master/txscript/stdaddr/address_test.go
    const DCRD_PAYABLE: [(Chain, &str, &str); 6] = [
        (
            Chain::Decred,
            "DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJu",
            "76a9142789d58cfa0957d206f025c2af056fc8a77cebb088ac",
        ),
        (
            Chain::Decred,
            "DsU7xcg53nxaKLLcAUSKyRndjG78Z2VZnX9",
            "76a914229ebac30efd6a69eec9c1a48e048b7c975c25f288ac",
        ),
        (
            Chain::DecredTestnet,
            "Tso2MVTUeVrjHTBFedFhiyM7yVTbieqp91h",
            "76a914f15da1cb8d1bcb162c6ab446c95757a6e791c91688ac",
        ),
        (
            Chain::Decred,
            "DcuQKx8BES9wU7C6Q5VmLBjw436r27hayjS",
            "a914f0b4e85100aee1a996f22915eb3c3f764d53779a87",
        ),
        (
            Chain::Decred,
            "DcqgK4N4Ccucu2Sq4VDAdu4wH4LASLhzLVp",
            "a914c7da5095683436f4435fc4e7163dcafda1a2d00787",
        ),
        (
            Chain::DecredTestnet,
            "TccWLgcquqvwrfBocq5mcK5kBiyw8MvyvCi",
            "a91436c1ca10a8a6a4b5d4204ac970853979903aa28487",
        ),
    ];

    #[test]
    fn dcrd_vectors_pay_the_script_the_address_names() {
        for (chain, address, script) in DCRD_PAYABLE {
            let parsed = parse_decred_address(chain, address).unwrap();
            assert_eq!(hex::encode(parsed.script_pubkey()), script, "{address}");
            let other = if chain == Chain::Decred {
                Chain::DecredTestnet
            } else {
                Chain::Decred
            };
            assert!(parse_decred_address(other, address).is_err(), "{address}");
        }
    }

    #[test]
    fn p2pkh_encoding_matches_dcrd() {
        for (chain, address, script) in DCRD_PAYABLE {
            if let Some(hash) = script.strip_prefix("76a914") {
                let hash: [u8; 20] = hex::decode(&hash[..40]).unwrap().try_into().unwrap();
                assert_eq!(encode_decred_p2pkh(chain, &hash).unwrap(), address);
            }
        }
    }

    #[test]
    fn refuses_what_it_cannot_pay() {
        for address in [
            "",
            "not-a-decred-address",
            "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
            // dcrd's pay-to-pubkey, Ed25519 and Schnorr pubkey-hash vectors:
            // valid Decred addresses whose scripts these sends do not build.
            "DkM3ZigNyiwHrsXRjkDQ8t8tW6uKGW9g61qEkG3bMqQPQWYEf5X3J",
            "DeeUhrRoTp4DftsqddVW96yMGMW4sgQFYUE",
            "DSXcZv4oSRiEoWL2a9aD8sgfptRo1YEXNKj",
            // A checksum broken in its last character.
            "DsUZxxoHJSty8DCfwfartwTYbuhmVct7tJv",
        ] {
            assert!(
                parse_decred_address(Chain::Decred, address).is_err(),
                "{address}"
            );
        }
        for address in [
            "TkKmMiY5iDh4U3KkSopYgkU1AzhAcQZiSoVhYhFymZHGMi9LM9Fdt",
            "TeeXvqZJrc7KnFZCT27fHfzcrTTzSF1aSRG",
            "TSr4xSiznUfzxkJcH7F3xuaFCUBdEb5Jfzg",
        ] {
            assert!(
                parse_decred_address(Chain::DecredTestnet, address).is_err(),
                "{address}"
            );
        }
    }
}
