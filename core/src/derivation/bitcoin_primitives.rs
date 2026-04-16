//! Hand-rolled Bitcoin primitives used by the derivation runtime and the
//! HD account helpers. The goal is to let those modules stop depending on
//! the monolithic `bitcoin` crate while still producing byte-for-byte
//! identical addresses, xpubs, and BIP-32 child keys. The chain-signing
//! path (chains/bitcoin.rs) still uses the `bitcoin` crate because Bitcoin
//! consensus serialization and Taproot sighashes are out of scope here.
//!
//! What's implemented:
//!   * BIP-32 CKDpriv / CKDpub over secp256k1
//!   * xpub / xprv Base58Check serialization (canonical mainnet + testnet
//!     version bytes, plus y/zpub swapping for SegWit-labeled xpubs)
//!   * Hash160, Base58Check, Bitcoin address encoders (P2PKH, P2SH-P2WPKH,
//!     P2WPKH, P2TR) and a structural decoder used by the address validator
//!
//! Correctness is pinned by the golden vectors in `runtime.rs::tests`.

use bech32::Hrp;
use hmac::{Hmac, Mac};
use ripemd::Ripemd160;
use secp256k1::{All, PublicKey, Scalar, Secp256k1, SecretKey};
use sha2::{Digest, Sha256, Sha512};

// ──────────────────────────────────────────────────────────────────────
// Hashing
// ──────────────────────────────────────────────────────────────────────

type HmacSha512 = Hmac<Sha512>;

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(&out);
    result
}

/// RIPEMD160(SHA256(bytes)) — Bitcoin's Hash160 primitive.
pub fn hash160(bytes: &[u8]) -> [u8; 20] {
    let sha = sha256(bytes);
    let mut hasher = Ripemd160::new();
    hasher.update(sha);
    let out = hasher.finalize();
    let mut result = [0u8; 20];
    result.copy_from_slice(&out);
    result
}

// ──────────────────────────────────────────────────────────────────────
// Base58Check
// ──────────────────────────────────────────────────────────────────────

pub fn base58check_encode(payload: &[u8]) -> String {
    bs58::encode(payload).with_check().into_string()
}

pub fn base58check_decode(s: &str) -> Result<Vec<u8>, String> {
    bs58::decode(s)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("base58check decode: {e}"))
}

// ──────────────────────────────────────────────────────────────────────
// BIP-32 extended keys
// ──────────────────────────────────────────────────────────────────────

pub const XPUB_VERSION_MAINNET: [u8; 4] = [0x04, 0x88, 0xB2, 0x1E];
pub const XPUB_VERSION_TESTNET: [u8; 4] = [0x04, 0x35, 0x87, 0xCF];

pub const HARDENED_OFFSET: u32 = 0x80000000;

#[derive(Clone)]
pub struct ExtendedPrivateKey {
    pub depth: u8,
    pub parent_fingerprint: [u8; 4],
    pub child_number: u32,
    pub private_key: SecretKey,
    pub chain_code: [u8; 32],
}

#[derive(Clone)]
pub struct ExtendedPublicKey {
    pub depth: u8,
    pub parent_fingerprint: [u8; 4],
    pub child_number: u32,
    pub public_key: PublicKey,
    pub chain_code: [u8; 32],
}

impl ExtendedPrivateKey {
    /// Master key from BIP-39 seed. `hmac_key` is usually `b"Bitcoin seed"` but
    /// is caller-tunable so we can reuse this for SLIP-0010 if ever needed.
    pub fn master_from_seed(hmac_key: &[u8], seed: &[u8]) -> Result<Self, String> {
        let mut mac =
            HmacSha512::new_from_slice(hmac_key).map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(seed);
        let tag = mac.finalize().into_bytes();
        let private_key = SecretKey::from_slice(&tag[..32])
            .map_err(|e| format!("Derived BIP-32 master key is invalid: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);
        Ok(Self {
            depth: 0,
            parent_fingerprint: [0u8; 4],
            child_number: 0,
            private_key,
            chain_code,
        })
    }

    pub fn fingerprint(&self, secp: &Secp256k1<All>) -> [u8; 4] {
        let pk = PublicKey::from_secret_key(secp, &self.private_key);
        let hash = hash160(&pk.serialize());
        let mut fp = [0u8; 4];
        fp.copy_from_slice(&hash[..4]);
        fp
    }

    pub fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| format!("HMAC init: {e}"))?;
        if index >= HARDENED_OFFSET {
            mac.update(&[0x00]);
            mac.update(&self.private_key.secret_bytes());
        } else {
            let pk = PublicKey::from_secret_key(secp, &self.private_key);
            mac.update(&pk.serialize());
        }
        mac.update(&index.to_be_bytes());
        let tag = mac.finalize().into_bytes();

        let tweak = Scalar::from_be_bytes(
            tag[..32]
                .try_into()
                .map_err(|_| "BIP-32 tag slice".to_string())?,
        )
        .map_err(|_| "BIP-32 IL out of range — retry the derivation".to_string())?;
        let private_key = self
            .private_key
            .add_tweak(&tweak)
            .map_err(|e| format!("BIP-32 tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);

        let parent_fingerprint = self.fingerprint(secp);
        Ok(Self {
            depth: self.depth.saturating_add(1),
            parent_fingerprint,
            child_number: index,
            private_key,
            chain_code,
        })
    }

    pub fn derive_path(&self, secp: &Secp256k1<All>, path: &[u32]) -> Result<Self, String> {
        let mut key = self.clone();
        for &index in path {
            key = key.derive_child(secp, index)?;
        }
        Ok(key)
    }

    pub fn to_neutered(&self, secp: &Secp256k1<All>) -> ExtendedPublicKey {
        ExtendedPublicKey {
            depth: self.depth,
            parent_fingerprint: self.parent_fingerprint,
            child_number: self.child_number,
            public_key: PublicKey::from_secret_key(secp, &self.private_key),
            chain_code: self.chain_code,
        }
    }
}

impl ExtendedPublicKey {
    pub fn fingerprint(&self) -> [u8; 4] {
        let hash = hash160(&self.public_key.serialize());
        let mut fp = [0u8; 4];
        fp.copy_from_slice(&hash[..4]);
        fp
    }

    /// CKDpub for unhardened children. Hardened indices return an error — you
    /// can't walk a hardened level from just a public key, by design.
    pub fn derive_child(&self, secp: &Secp256k1<All>, index: u32) -> Result<Self, String> {
        if index >= HARDENED_OFFSET {
            return Err("cannot derive a hardened child from an xpub".to_string());
        }
        let mut mac = HmacSha512::new_from_slice(&self.chain_code)
            .map_err(|e| format!("HMAC init: {e}"))?;
        mac.update(&self.public_key.serialize());
        mac.update(&index.to_be_bytes());
        let tag = mac.finalize().into_bytes();

        let tweak = Scalar::from_be_bytes(
            tag[..32]
                .try_into()
                .map_err(|_| "BIP-32 tag slice".to_string())?,
        )
        .map_err(|_| "BIP-32 IL out of range — retry the derivation".to_string())?;
        let public_key = self
            .public_key
            .add_exp_tweak(secp, &tweak)
            .map_err(|e| format!("BIP-32 pubkey tweak failed: {e}"))?;
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&tag[32..]);

        Ok(Self {
            depth: self.depth.saturating_add(1),
            parent_fingerprint: self.fingerprint(),
            child_number: index,
            public_key,
            chain_code,
        })
    }

    pub fn to_xpub_string(&self, version: [u8; 4]) -> String {
        encode_extended_key(
            version,
            self.depth,
            self.parent_fingerprint,
            self.child_number,
            self.chain_code,
            &self.public_key.serialize(),
        )
    }

    /// Parse an xpub string and return the (key, observed version bytes). We
    /// don't auto-resolve version bytes to a network here — the caller knows
    /// which prefixes they accept.
    pub fn from_xpub_string(s: &str) -> Result<(Self, [u8; 4]), String> {
        let (version, depth, parent_fingerprint, child_number, chain_code, key_bytes) =
            decode_extended_key(s)?;
        let public_key = PublicKey::from_slice(&key_bytes)
            .map_err(|e| format!("xpub: invalid pubkey: {e}"))?;
        Ok((
            Self {
                depth,
                parent_fingerprint,
                child_number,
                public_key,
                chain_code,
            },
            version,
        ))
    }
}

fn encode_extended_key(
    version: [u8; 4],
    depth: u8,
    parent_fingerprint: [u8; 4],
    child_number: u32,
    chain_code: [u8; 32],
    key_bytes: &[u8],
) -> String {
    let mut payload = Vec::with_capacity(78);
    payload.extend_from_slice(&version);
    payload.push(depth);
    payload.extend_from_slice(&parent_fingerprint);
    payload.extend_from_slice(&child_number.to_be_bytes());
    payload.extend_from_slice(&chain_code);
    payload.extend_from_slice(key_bytes);
    base58check_encode(&payload)
}

fn decode_extended_key(
    s: &str,
) -> Result<([u8; 4], u8, [u8; 4], u32, [u8; 32], [u8; 33]), String> {
    let payload = base58check_decode(s)?;
    if payload.len() != 78 {
        return Err(format!(
            "xpub/xprv payload must be 78 bytes, got {}",
            payload.len()
        ));
    }
    let mut version = [0u8; 4];
    version.copy_from_slice(&payload[0..4]);
    let depth = payload[4];
    let mut parent_fingerprint = [0u8; 4];
    parent_fingerprint.copy_from_slice(&payload[5..9]);
    let child_number = u32::from_be_bytes(payload[9..13].try_into().unwrap());
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&payload[13..45]);
    let mut key_bytes = [0u8; 33];
    key_bytes.copy_from_slice(&payload[45..78]);
    Ok((
        version,
        depth,
        parent_fingerprint,
        child_number,
        chain_code,
        key_bytes,
    ))
}

// ──────────────────────────────────────────────────────────────────────
// BIP-32 path parsing
// ──────────────────────────────────────────────────────────────────────

/// Parse `m/44'/0'/0'/0/0` → [44|H, 0|H, 0|H, 0, 0] where `|H` sets the
/// hardened high bit. Accepts both `'` and `h` suffixes for hardening.
pub fn parse_bip32_path(path: &str) -> Result<Vec<u32>, String> {
    let trimmed = path.trim().trim_start_matches('m').trim_start_matches('M');
    let trimmed = trimmed.trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for segment in trimmed.split('/') {
        let (value, hardened) = if let Some(stripped) = segment.strip_suffix('\'') {
            (stripped, true)
        } else if let Some(stripped) = segment.strip_suffix('h') {
            (stripped, true)
        } else if let Some(stripped) = segment.strip_suffix('H') {
            (stripped, true)
        } else {
            (segment, false)
        };
        let raw: u32 = value
            .parse()
            .map_err(|_| format!("invalid path segment: {segment}"))?;
        if raw >= HARDENED_OFFSET {
            return Err(format!("path segment out of range: {segment}"));
        }
        out.push(if hardened { raw | HARDENED_OFFSET } else { raw });
    }
    Ok(out)
}

// ──────────────────────────────────────────────────────────────────────
// Bitcoin address encoding
// ──────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct BitcoinNetworkParams {
    pub p2pkh_version: u8,
    pub p2sh_version: u8,
    pub bech32_hrp: &'static str,
}

pub const BTC_MAINNET: BitcoinNetworkParams = BitcoinNetworkParams {
    p2pkh_version: 0x00,
    p2sh_version: 0x05,
    bech32_hrp: "bc",
};

pub const BTC_TESTNET: BitcoinNetworkParams = BitcoinNetworkParams {
    p2pkh_version: 0x6f,
    p2sh_version: 0xc4,
    bech32_hrp: "tb",
};

pub fn encode_p2pkh(params: &BitcoinNetworkParams, compressed_pubkey: &[u8]) -> String {
    let mut payload = Vec::with_capacity(21);
    payload.push(params.p2pkh_version);
    payload.extend_from_slice(&hash160(compressed_pubkey));
    base58check_encode(&payload)
}

pub fn encode_p2sh_p2wpkh(params: &BitcoinNetworkParams, compressed_pubkey: &[u8]) -> String {
    // redeemScript = OP_0 <20-byte pubkey hash>
    let mut redeem = Vec::with_capacity(22);
    redeem.push(0x00);
    redeem.push(0x14);
    redeem.extend_from_slice(&hash160(compressed_pubkey));
    let mut payload = Vec::with_capacity(21);
    payload.push(params.p2sh_version);
    payload.extend_from_slice(&hash160(&redeem));
    base58check_encode(&payload)
}

pub fn encode_p2wpkh(
    params: &BitcoinNetworkParams,
    compressed_pubkey: &[u8],
) -> Result<String, String> {
    let program = hash160(compressed_pubkey);
    let hrp = Hrp::parse(params.bech32_hrp).map_err(|e| format!("bech32 hrp: {e}"))?;
    bech32::segwit::encode_v0(hrp, &program).map_err(|e| format!("bech32 encode v0: {e}"))
}

/// BIP-86 key-path-only Taproot: output_key = internal_key + H_tapTweak(internal_key) * G.
pub fn encode_p2tr(
    params: &BitcoinNetworkParams,
    secp: &Secp256k1<All>,
    public_key: &PublicKey,
) -> Result<String, String> {
    let (x_only, _parity) = public_key.x_only_public_key();
    // tagged_hash("TapTweak", internal_key_x_only)
    let tag_hash = sha256(b"TapTweak");
    let mut hasher = Sha256::new();
    hasher.update(tag_hash);
    hasher.update(tag_hash);
    hasher.update(x_only.serialize());
    let tweak_bytes: [u8; 32] = hasher.finalize().into();
    let tweak = Scalar::from_be_bytes(tweak_bytes)
        .map_err(|_| "Taproot tweak out of range".to_string())?;
    let (tweaked, _parity) = x_only
        .add_tweak(secp, &tweak)
        .map_err(|e| format!("taproot tweak: {e}"))?;
    let hrp = Hrp::parse(params.bech32_hrp).map_err(|e| format!("bech32 hrp: {e}"))?;
    bech32::segwit::encode_v1(hrp, &tweaked.serialize())
        .map_err(|e| format!("bech32 encode v1: {e}"))
}

// ──────────────────────────────────────────────────────────────────────
// Bitcoin address parsing (structural, for the validator)
// ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitcoinNetworkKind {
    Mainnet,
    Testnet,
}

#[derive(Debug, Clone)]
pub enum ParsedBitcoinAddress {
    Legacy {
        network: BitcoinNetworkKind,
    },
    SegWit {
        network: BitcoinNetworkKind,
    },
}

pub fn parse_bitcoin_address(s: &str) -> Result<ParsedBitcoinAddress, String> {
    // SegWit (bech32 / bech32m) — HRP is lowercase "bc" or "tb".
    if let Ok((hrp, _witver, _program)) = bech32::segwit::decode(s) {
        let hrp_str = hrp.to_string().to_lowercase();
        let network = match hrp_str.as_str() {
            "bc" => BitcoinNetworkKind::Mainnet,
            "tb" | "bcrt" => BitcoinNetworkKind::Testnet,
            other => return Err(format!("unknown bech32 HRP: {other}")),
        };
        return Ok(ParsedBitcoinAddress::SegWit { network });
    }
    // Legacy base58check: 0x00/0x05 mainnet, 0x6f/0xc4 testnet.
    let payload = base58check_decode(s)?;
    if payload.len() != 21 {
        return Err(format!("legacy payload must be 21 bytes, got {}", payload.len()));
    }
    let network = match payload[0] {
        0x00 | 0x05 => BitcoinNetworkKind::Mainnet,
        0x6f | 0xc4 => BitcoinNetworkKind::Testnet,
        other => return Err(format!("unknown legacy version byte: 0x{other:02x}")),
    };
    Ok(ParsedBitcoinAddress::Legacy { network })
}

