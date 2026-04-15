//! Bitcoin HD multi-address helpers.
//!
//! This module lets Swift pass in a BIP32 extended public key (xpub, ypub,
//! or zpub) and get back a derived address list plus aggregated UTXO/balance
//! info, without the Rust layer ever seeing a private key. It replaces the
//! Swift-side dependency on `blockchain.info/multiaddr` and the Blockchair
//! xpub dashboard, which are rate-limited and inconsistent.
//!
//! ## Input formats
//!
//! - `xpub…` — BIP44 legacy P2PKH (version bytes `04 88 B2 1E`)
//! - `ypub…` — BIP49 P2SH-nested-P2WPKH (version bytes `04 9D 7C B2`)
//! - `zpub…` — BIP84 native SegWit P2WPKH (version bytes `04 B2 47 46`)
//!
//! We normalize y/zpub prefixes down to the canonical xpub version bytes
//! before parsing. The script type is inferred from the original prefix so
//! address formatting still picks the right encoder.
//!
//! ## Derivation
//!
//! Given an account-level xpub, receive addresses live at `0/i` and change
//! at `1/i`. `derive_children` walks a contiguous index range on the given
//! chain leg and returns `(index, address)` tuples. Aggregation helpers then
//! query Esplora per address and sum the results.

use bip39::Mnemonic;
use secp256k1::{All, Secp256k1};
use serde::{Deserialize, Serialize};

use crate::chains::bitcoin::{BitcoinClient, EsploraUtxo};
use crate::derivation::bitcoin_primitives::{
    encode_p2pkh, encode_p2sh_p2wpkh, encode_p2wpkh, parse_bip32_path, BitcoinNetworkParams,
    ExtendedPrivateKey, ExtendedPublicKey, BTC_MAINNET, BTC_TESTNET, XPUB_VERSION_MAINNET,
    XPUB_VERSION_TESTNET,
};

// ----------------------------------------------------------------
// Script type inferred from the xpub prefix
// ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdScriptType {
    /// BIP44 legacy P2PKH (xpub).
    P2pkh,
    /// BIP49 nested SegWit P2SH-P2WPKH (ypub).
    P2shP2wpkh,
    /// BIP84 native SegWit P2WPKH (zpub).
    P2wpkh,
}

impl HdScriptType {
    pub fn from_prefix(prefix: &str) -> Option<HdScriptType> {
        match prefix {
            "xpub" | "tpub" => Some(HdScriptType::P2pkh),
            "ypub" | "upub" => Some(HdScriptType::P2shP2wpkh),
            "zpub" | "vpub" => Some(HdScriptType::P2wpkh),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdNetwork {
    Mainnet,
    Testnet,
}

impl HdNetwork {
    fn params(self) -> BitcoinNetworkParams {
        match self {
            HdNetwork::Mainnet => BTC_MAINNET,
            HdNetwork::Testnet => BTC_TESTNET,
        }
    }

    fn xpub_version(self) -> [u8; 4] {
        match self {
            HdNetwork::Mainnet => XPUB_VERSION_MAINNET,
            HdNetwork::Testnet => XPUB_VERSION_TESTNET,
        }
    }
}

// ----------------------------------------------------------------
// Xpub normalization
// ----------------------------------------------------------------

/// Normalize a `y/zpub` (or their testnet counterparts) into an `x/tpub` by
/// swapping the 4-byte serialization version prefix. The payload bytes
/// (depth, parent fingerprint, child number, chain code, pubkey) remain
/// untouched. Base58Check is re-encoded after the swap.
pub fn normalize_xpub(input: &str) -> Result<(String, HdScriptType, HdNetwork), String> {
    let prefix = input.get(..4).unwrap_or("");
    let script_type = HdScriptType::from_prefix(prefix)
        .ok_or_else(|| format!("unsupported xpub prefix: {prefix}"))?;
    let is_testnet = matches!(prefix, "tpub" | "upub" | "vpub");
    let network = if is_testnet {
        HdNetwork::Testnet
    } else {
        HdNetwork::Mainnet
    };

    // If already a canonical xpub/tpub, skip the base58 round trip.
    if matches!(prefix, "xpub" | "tpub") {
        return Ok((input.to_string(), script_type, network));
    }

    // Decode base58check (4-byte checksum appended by bitcoin-encoded xpubs).
    let raw = bs58::decode(input)
        .with_check(None)
        .into_vec()
        .map_err(|e| format!("bad xpub base58: {e}"))?;
    if raw.len() < 4 {
        return Err("xpub too short".to_string());
    }
    let mut swapped = Vec::with_capacity(raw.len());
    swapped.extend_from_slice(&network.xpub_version());
    swapped.extend_from_slice(&raw[4..]);
    let encoded = bs58::encode(&swapped).with_check().into_string();
    Ok((encoded, script_type, network))
}

// ----------------------------------------------------------------
// Child derivation
// ----------------------------------------------------------------

/// One derived child address with its `change/index` position on the chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HdChildAddress {
    pub index: u32,
    /// 0 = external (receive), 1 = internal (change).
    pub change: u32,
    pub address: String,
}

/// Derive a contiguous range of child addresses from an account-level xpub.
///
/// The xpub is expected to already sit at the BIP44/49/84 account node
/// (e.g. `m/84'/0'/0'`), so only the final two unhardened children
/// (`change/index`) are appended inside this function.
pub fn derive_children(
    xpub_input: &str,
    change: u32,
    start_index: u32,
    count: u32,
) -> Result<Vec<HdChildAddress>, String> {
    if count == 0 {
        return Ok(Vec::new());
    }

    let (canon, script_type, network) = normalize_xpub(xpub_input)?;
    let (xpub, _version) = ExtendedPublicKey::from_xpub_string(&canon)
        .map_err(|e| format!("bad xpub: {e}"))?;
    let secp = Secp256k1::<All>::new();

    let leg_xpub = xpub
        .derive_child(&secp, change)
        .map_err(|e| format!("derive change leg: {e}"))?;

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let idx = start_index.saturating_add(i);
        let child = leg_xpub
            .derive_child(&secp, idx)
            .map_err(|e| format!("derive index {idx}: {e}"))?;
        let address = address_from_pubkey(&child, script_type, network)?;
        out.push(HdChildAddress {
            index: idx,
            change,
            address,
        });
    }
    Ok(out)
}

fn address_from_pubkey(
    child: &ExtendedPublicKey,
    script_type: HdScriptType,
    network: HdNetwork,
) -> Result<String, String> {
    let compressed = child.public_key.serialize();
    let params = network.params();
    match script_type {
        HdScriptType::P2pkh => Ok(encode_p2pkh(&params, &compressed)),
        HdScriptType::P2shP2wpkh => Ok(encode_p2sh_p2wpkh(&params, &compressed)),
        HdScriptType::P2wpkh => encode_p2wpkh(&params, &compressed),
    }
}

// ----------------------------------------------------------------
// Aggregated balance / UTXO fetch
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HdXpubBalance {
    /// Total confirmed satoshis across all scanned addresses.
    pub confirmed_sats: u64,
    /// Total unconfirmed delta across all scanned addresses.
    pub unconfirmed_sats: i64,
    /// Total UTXO count across all scanned addresses.
    pub utxo_count: usize,
    /// Addresses that were scanned (receive + change).
    pub scanned_addresses: Vec<HdChildAddress>,
    /// UTXOs keyed to the address that owns them.
    pub utxos: Vec<HdUtxo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HdUtxo {
    pub address: String,
    pub change: u32,
    pub index: u32,
    pub txid: String,
    pub vout: u32,
    pub value_sats: u64,
    pub confirmed: bool,
}

/// Scan `receive_count` external + `change_count` internal addresses and
/// return an aggregated balance plus per-UTXO breakdown. `client` must
/// already be configured with Esplora endpoints for the target network.
pub async fn fetch_xpub_balance(
    client: &BitcoinClient,
    xpub_input: &str,
    receive_count: u32,
    change_count: u32,
) -> Result<HdXpubBalance, String> {
    let receive = derive_children(xpub_input, 0, 0, receive_count)?;
    let change = derive_children(xpub_input, 1, 0, change_count)?;
    let mut all = Vec::with_capacity(receive.len() + change.len());
    all.extend(receive);
    all.extend(change);

    let mut confirmed_sats: u64 = 0;
    let mut unconfirmed_sats: i64 = 0;
    let mut utxo_count: usize = 0;
    let mut utxos_out: Vec<HdUtxo> = Vec::new();

    // Sequential scan — Esplora rate limits make parallel fanouts risky, and
    // most HD wallets only scan 20-40 addresses at a time.
    for addr in &all {
        let bal = client.fetch_balance(&addr.address).await?;
        confirmed_sats = confirmed_sats.saturating_add(bal.confirmed_sats);
        unconfirmed_sats = unconfirmed_sats.saturating_add(bal.unconfirmed_sats);
        utxo_count = utxo_count.saturating_add(bal.utxo_count);

        if bal.confirmed_sats > 0 || bal.unconfirmed_sats > 0 {
            let per_addr = client.fetch_utxos(&addr.address).await?;
            for u in per_addr {
                utxos_out.push(from_esplora_utxo(&u, addr));
            }
        }
    }

    Ok(HdXpubBalance {
        confirmed_sats,
        unconfirmed_sats,
        utxo_count,
        scanned_addresses: all,
        utxos: utxos_out,
    })
}

fn from_esplora_utxo(u: &EsploraUtxo, addr: &HdChildAddress) -> HdUtxo {
    HdUtxo {
        address: addr.address.clone(),
        change: addr.change,
        index: addr.index,
        txid: u.txid.clone(),
        vout: u.vout,
        value_sats: u.value,
        confirmed: u.status.confirmed,
    }
}

// ----------------------------------------------------------------
// Next-unused address (receive/change discovery)
// ----------------------------------------------------------------

/// Scan forward on the `change` leg until the first address that has zero
/// historical transactions, respecting a gap limit. Returns `Ok(None)` if
/// the entire scan window is used (caller should widen the gap limit or
/// signal the wallet is nearly out of fresh addresses).
pub async fn fetch_next_unused_address(
    client: &BitcoinClient,
    xpub_input: &str,
    change: u32,
    gap_limit: u32,
) -> Result<Option<HdChildAddress>, String> {
    let batch = derive_children(xpub_input, change, 0, gap_limit.max(1))?;
    for candidate in batch {
        let bal = client.fetch_balance(&candidate.address).await?;
        if bal.utxo_count == 0 && bal.confirmed_sats == 0 && bal.unconfirmed_sats == 0 {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

// ----------------------------------------------------------------
// Seed phrase → account-level xpub
// ----------------------------------------------------------------

/// Derive the account-level extended public key (xpub) from a BIP39 mnemonic.
///
/// `account_path` must be a hardened account path such as `"m/84'/0'/0'"`.
/// The returned string is always encoded as a canonical `xpub` (mainnet).
/// Callers that want ypub/zpub formatting can re-encode the bytes as needed.
///
/// Standard account paths:
/// - BIP44 legacy P2PKH:       `m/44'/0'/0'`
/// - BIP49 P2SH-P2WPKH:        `m/49'/0'/0'`
/// - BIP84 native SegWit:      `m/84'/0'/0'`
/// - BIP86 Taproot:            `m/86'/0'/0'`
pub fn derive_account_xpub(
    mnemonic_phrase: &str,
    passphrase: &str,
    account_path: &str,
) -> Result<String, String> {
    let mnemonic: Mnemonic = mnemonic_phrase
        .trim()
        .parse()
        .map_err(|e| format!("invalid mnemonic: {e}"))?;
    let seed = mnemonic.to_seed(passphrase);

    let secp = Secp256k1::<All>::new();
    let master = ExtendedPrivateKey::master_from_seed(b"Bitcoin seed", &seed)
        .map_err(|e| format!("master key: {e}"))?;
    let path = parse_bip32_path(account_path)
        .map_err(|e| format!("invalid derivation path: {e}"))?;
    let account_xpriv = master
        .derive_path(&secp, &path)
        .map_err(|e| format!("derive priv: {e}"))?;
    let account_xpub = account_xpriv.to_neutered(&secp);
    Ok(account_xpub.to_xpub_string(XPUB_VERSION_MAINNET))
}
