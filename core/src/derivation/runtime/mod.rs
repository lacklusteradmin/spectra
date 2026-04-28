use bip39::{Language, Mnemonic};
use ed25519_dalek::SigningKey;
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Sha512;
use std::fmt::Display;
use tiny_keccak::{Hasher, Keccak};
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};

use super::bitcoin_primitives::{
    encode_p2pkh, encode_p2sh_p2wpkh, encode_p2tr, encode_p2wpkh, hash160 as hash160_bytes,
    parse_bip32_path, sha256 as sha256_bytes, BitcoinNetworkParams, ExtendedPrivateKey,
    BTC_MAINNET, BTC_TESTNET,
};

type HmacSha512 = Hmac<Sha512>;

// Bitflags describing which output fields the caller wants back.
const OUTPUT_ADDRESS: u32 = 1 << 0;
const OUTPUT_PUBLIC_KEY: u32 = 1 << 1;
const OUTPUT_PRIVATE_KEY: u32 = 1 << 2;

pub(super) const CHAIN_BITCOIN: u32 = 0;
pub(super) const CHAIN_ETHEREUM: u32 = 1;
pub(super) const CHAIN_SOLANA: u32 = 2;
pub(super) const CHAIN_BITCOIN_CASH: u32 = 3;
pub(super) const CHAIN_BITCOIN_SV: u32 = 4;
pub(super) const CHAIN_LITECOIN: u32 = 5;
pub(super) const CHAIN_DOGECOIN: u32 = 6;
pub(super) const CHAIN_ETHEREUM_CLASSIC: u32 = 7;
pub(super) const CHAIN_ARBITRUM: u32 = 8;
pub(super) const CHAIN_OPTIMISM: u32 = 9;
pub(super) const CHAIN_AVALANCHE: u32 = 10;
pub(super) const CHAIN_HYPERLIQUID: u32 = 11;
pub(super) const CHAIN_TRON: u32 = 12;
pub(super) const CHAIN_STELLAR: u32 = 13;
pub(super) const CHAIN_XRP: u32 = 14;
pub(super) const CHAIN_CARDANO: u32 = 15;
pub(super) const CHAIN_SUI: u32 = 16;
pub(super) const CHAIN_APTOS: u32 = 17;
pub(super) const CHAIN_TON: u32 = 18;
pub(super) const CHAIN_INTERNET_COMPUTER: u32 = 19;
pub(super) const CHAIN_NEAR: u32 = 20;
pub(super) const CHAIN_POLKADOT: u32 = 21;
pub(super) const CHAIN_MONERO: u32 = 22;
pub(super) const CHAIN_ZCASH: u32 = 23;
pub(super) const CHAIN_BITCOIN_GOLD: u32 = 24;
pub(super) const CHAIN_DECRED: u32 = 25;
pub(super) const CHAIN_KASPA: u32 = 26;
pub(super) const CHAIN_DASH: u32 = 27;
pub(super) const CHAIN_BITTENSOR: u32 = 28;

pub(super) const NETWORK_MAINNET: u32 = 0;
pub(super) const NETWORK_TESTNET: u32 = 1;
pub(super) const NETWORK_TESTNET4: u32 = 2;
pub(super) const NETWORK_SIGNET: u32 = 3;

pub(super) const CURVE_SECP256K1: u32 = 0;
pub(super) const CURVE_ED25519: u32 = 1;
pub(super) const CURVE_SR25519: u32 = 2;

const DERIVATION_AUTO: u32 = 0;
pub(super) const DERIVATION_BIP32_SECP256K1: u32 = 1;
pub(super) const DERIVATION_SLIP10_ED25519: u32 = 2;
pub(super) const DERIVATION_DIRECT_SEED_ED25519: u32 = 3;
pub(super) const DERIVATION_TON_MNEMONIC: u32 = 4;
pub(super) const DERIVATION_BIP32_ED25519_ICARUS: u32 = 5;
pub(super) const DERIVATION_SUBSTRATE_BIP39: u32 = 6;
pub(super) const DERIVATION_MONERO_BIP39: u32 = 7;

const ADDRESS_AUTO: u32 = 0;
pub(super) const ADDRESS_BITCOIN: u32 = 1;
pub(super) const ADDRESS_EVM: u32 = 2;
pub(super) const ADDRESS_SOLANA: u32 = 3;
pub(super) const ADDRESS_NEAR_HEX: u32 = 4;
pub(super) const ADDRESS_TON_RAW_ACCOUNT_ID: u32 = 5;
pub(super) const ADDRESS_CARDANO_SHELLEY_ENTERPRISE: u32 = 6;
pub(super) const ADDRESS_SS58: u32 = 7;
pub(super) const ADDRESS_MONERO_MAIN: u32 = 8;
pub(super) const ADDRESS_TON_V4R2: u32 = 9;
pub(super) const ADDRESS_LITECOIN: u32 = 10;
pub(super) const ADDRESS_DOGECOIN: u32 = 11;
pub(super) const ADDRESS_BITCOIN_CASH_LEGACY: u32 = 12;
pub(super) const ADDRESS_BITCOIN_SV_LEGACY: u32 = 13;
pub(super) const ADDRESS_TRON_BASE58_CHECK: u32 = 14;
pub(super) const ADDRESS_XRP_BASE58_CHECK: u32 = 15;
pub(super) const ADDRESS_STELLAR_STRKEY: u32 = 16;
pub(super) const ADDRESS_SUI_KECCAK: u32 = 17;
pub(super) const ADDRESS_APTOS_KECCAK: u32 = 18;
pub(super) const ADDRESS_ICP_PRINCIPAL: u32 = 19;
pub(super) const ADDRESS_ZCASH_TRANSPARENT: u32 = 20;
pub(super) const ADDRESS_BITCOIN_GOLD_LEGACY: u32 = 21;
pub(super) const ADDRESS_DECRED_P2PKH: u32 = 22;
pub(super) const ADDRESS_KASPA_SCHNORR: u32 = 23;
pub(super) const ADDRESS_DASH_LEGACY: u32 = 24;
pub(super) const ADDRESS_BITTENSOR_SS58: u32 = 25;

const PUBLIC_KEY_AUTO: u32 = 0;
pub(super) const PUBLIC_KEY_COMPRESSED: u32 = 1;
pub(super) const PUBLIC_KEY_UNCOMPRESSED: u32 = 2;
const PUBLIC_KEY_X_ONLY: u32 = 3;
pub(super) const PUBLIC_KEY_RAW: u32 = 4;

const SCRIPT_AUTO: u32 = 0;
pub(super) const SCRIPT_P2PKH: u32 = 1;
pub(super) const SCRIPT_P2SH_P2WPKH: u32 = 2;
pub(super) const SCRIPT_P2WPKH: u32 = 3;
pub(super) const SCRIPT_P2TR: u32 = 4;
pub(super) const SCRIPT_ACCOUNT: u32 = 5;

struct DerivedOutput {
    address: Option<String>,
    public_key_hex: Option<String>,
    private_key_hex: Option<String>,
}

#[derive(Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIDerivationRequest {
    // Deprecated: chain is now inferred from `address_algorithm` at parse
    // time. Callers may still send it for backwards compatibility, but the
    // value is ignored by the Rust side.
    #[serde(default)]
    pub chain: Option<u32>,
    pub network: u32,
    pub curve: u32,
    pub requested_outputs: u32,
    pub derivation_algorithm: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub seed_phrase: String,
    pub derivation_path: Option<String>,
    pub passphrase: Option<String>,
    pub hmac_key: Option<String>,
    pub mnemonic_wordlist: Option<String>,
    pub iteration_count: u32,
    pub salt_prefix: Option<String>,
}

#[derive(Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIPrivateKeyDerivationRequest {
    // Deprecated: chain is now inferred from `address_algorithm` at parse
    // time. Callers may still send it for backwards compatibility, but the
    // value is ignored by the Rust side.
    #[serde(default)]
    pub chain: Option<u32>,
    pub network: u32,
    pub curve: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub private_key_hex: String,
}

#[derive(Debug, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIDerivationResponse {
    pub address: Option<String>,
    pub public_key_hex: Option<String>,
    pub private_key_hex: Option<String>,
}

#[derive(Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIMaterialRequest {
    // Deprecated: chain is now inferred from `address_algorithm` at parse
    // time. Callers may still send it for backwards compatibility, but the
    // value is ignored by the Rust side.
    #[serde(default)]
    pub chain: Option<u32>,
    pub network: u32,
    pub curve: u32,
    pub derivation_algorithm: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub seed_phrase: String,
    pub derivation_path: String,
    pub passphrase: Option<String>,
    pub hmac_key: Option<String>,
    pub mnemonic_wordlist: Option<String>,
    pub iteration_count: u32,
    pub salt_prefix: Option<String>,
}

#[derive(Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIPrivateKeyMaterialRequest {
    // Deprecated: chain is now inferred from `address_algorithm` at parse
    // time. Callers may still send it for backwards compatibility, but the
    // value is ignored by the Rust side.
    #[serde(default)]
    pub chain: Option<u32>,
    pub network: u32,
    pub curve: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub private_key_hex: String,
    pub derivation_path: String,
}

#[derive(Debug, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIMaterialResponse {
    pub address: String,
    pub private_key_hex: String,
    pub derivation_path: String,
    pub account: u32,
    pub branch: u32,
    pub index: u32,
}

struct ParsedRequest {
    chain: Chain,
    network: NetworkFlavor,
    curve: CurveFamily,
    requested_outputs: u32,
    derivation_algorithm: DerivationAlgorithm,
    address_algorithm: AddressAlgorithm,
    public_key_format: PublicKeyFormat,
    script_type: ScriptType,
    seed_phrase: String,
    derivation_path: Option<String>,
    passphrase: String,
    hmac_key: Option<String>,
    mnemonic_wordlist: Option<String>,
    iteration_count: u32,
    salt_prefix: Option<String>,
}

impl Drop for ParsedRequest {
    fn drop(&mut self) {
        self.seed_phrase.zeroize();
        self.passphrase.zeroize();
        if let Some(hmac_key) = &mut self.hmac_key {
            hmac_key.zeroize();
        }
        if let Some(wordlist) = &mut self.mnemonic_wordlist {
            wordlist.zeroize();
        }
        if let Some(path) = &mut self.derivation_path {
            path.zeroize();
        }
        if let Some(salt_prefix) = &mut self.salt_prefix {
            salt_prefix.zeroize();
        }
    }
}

#[derive(Clone, Copy)]
enum Chain {
    Bitcoin,
    BitcoinCash,
    BitcoinSv,
    Litecoin,
    Dogecoin,
    Ethereum,
    EthereumClassic,
    Arbitrum,
    Optimism,
    Avalanche,
    Hyperliquid,
    Tron,
    Solana,
    Stellar,
    Xrp,
    Cardano,
    Sui,
    Aptos,
    Ton,
    InternetComputer,
    Near,
    Polkadot,
    Monero,
    Zcash,
    BitcoinGold,
    Decred,
    Kaspa,
    Dash,
    Bittensor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NetworkFlavor {
    Mainnet,
    Testnet,
    Testnet4,
    Signet,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CurveFamily {
    Secp256k1,
    Ed25519,
    // Schnorr/Ristretto on Curve25519 — Polkadot/Substrate signing curve.
    // The 32-byte private key is the "mini secret" that schnorrkel expands
    // into a full keypair via SHA-512 (ExpansionMode::Ed25519).
    Sr25519,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DerivationAlgorithm {
    Auto,
    Bip32Secp256k1,
    Slip10Ed25519,
    // Private key = PBKDF2-BIP39 seed[0..32]. Path is ignored. Matches the
    // MyNearWallet / near-seed-phrase convention used across the NEAR ecosystem.
    DirectSeedEd25519,
    // TON mnemonic scheme (ton-crypto): entropy = HMAC-SHA512(key=mnemonic,
    // data=passphrase); seed = PBKDF2(entropy, salt="TON default seed",
    // 100_000, 64); priv = seed[0..32]. Unrelated to BIP-39 PBKDF2.
    TonMnemonic,
    // CIP-3 Icarus: entropy = bip39.to_entropy(mnemonic); xprv = PBKDF2(
    // passphrase, entropy, 4096, 96); clamp per Khovratovich-Law; walk path
    // via BIP-32-Ed25519 CKDpriv.
    Bip32Ed25519Icarus,
    // substrate-bip39: mini_secret = PBKDF2-HMAC-SHA512(password=BIP-39
    // entropy, salt="mnemonic"+passphrase, 2048)[0..32]. Then schnorrkel
    // expands the mini-secret into an sr25519 keypair. Path support is
    // currently limited to the root (empty path); Substrate's //hard /soft
    // junctions are deferred.
    SubstrateBip39,
    // Monero (BIP-39 variant): private spend key = sc_reduce32(BIP-39 seed[
    // 0..32]); private view key = sc_reduce32(Keccak256(spend)). NOTE: this
    // does NOT match Monero's native Electrum-style 25-word seed used by
    // Cake/Monerujo; it is for cross-chain BIP-39 wallets only.
    MoneroBip39,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AddressAlgorithm {
    Auto,
    Bitcoin,
    Evm,
    Solana,
    NearHex,
    // TON "raw account id": "<workchain>:<hex>". Workchain defaults to 0.
    // NOTE: this is NOT the user-friendly base64url address; user-friendly
    // addresses require state-init (BOC) hashing of a specific wallet version
    // and are pending a full BOC implementation.
    TonRawAccountId,
    // Shelley enterprise address (CIP-19 header type 6, payment-key hash),
    // bech32 encoded under HRP "addr" (mainnet) or "addr_test" (testnet).
    CardanoShelleyEnterprise,
    // SS58 (Substrate) address: prefix_byte(s) || pubkey || blake2b_512(
    // "SS58PRE" || prefix || pubkey)[0..2], base58-encoded. Network prefix
    // = 0 (Polkadot mainnet) by default.
    Ss58,
    // Monero standard mainnet address: 0x12 || public_spend (32) ||
    // public_view (32) || keccak256(prev)[0..4], encoded with Monero's
    // chunked Base58 (8-byte blocks → 11 chars).
    MoneroMain,
    // TON wallet v4R2 user-friendly bounceable address: computed by
    // building a state_init cell with (v4R2 code cell, fresh data cell)
    // and taking its SHA-256 cell hash as the 32-byte account id, then
    // formatting tag(0x11)||workchain||account_id||crc16_xmodem as
    // base64url. Wallet code BOC is embedded and parsed at first use;
    // the resulting root hash is locked to the public v4R2 code hash
    // via a self-test.
    TonV4R2,
    // Bitcoin-family Base58Check/Bech32 variants that differ from
    // `Bitcoin` only in their network version bytes / HRP. Splitting
    // them into discrete variants is what makes address_algorithm
    // sufficient to describe a derivation without a `chain` hint.
    Litecoin,
    Dogecoin,
    BitcoinCashLegacy,
    BitcoinSvLegacy,
    // Zcash transparent (t1...): same hash160 + base58check structure as
    // Bitcoin P2PKH but with a 2-byte version prefix `0x1CB8` instead of
    // BTC's `0x00`. Shielded addresses are out of scope.
    ZcashTransparent,
    // Bitcoin Gold (G…): BCH/BTC-style P2PKH with version byte `0x26`.
    BitcoinGoldLegacy,
    // Decred (Ds…): RIPEMD-160(BLAKE-256(pub)) || base58check with BLAKE-256
    // checksum and 2-byte version `0x073F`.
    DecredP2pkh,
    // Kaspa: CashAddr-variant bech32 with HRP "kaspa", Schnorr P2PK encoded
    // with version byte 0x00 and a 32-byte x-only secp256k1 public key.
    KaspaSchnorr,
    // Dash (X…): BTC-style P2PKH with version byte `0x4C` (76 decimal).
    DashLegacy,
    // Bittensor (5…): SS58 with substrate-generic prefix 42, sr25519 curve
    // (substrate-bip39). Same wire/codec as Polkadot, different network prefix.
    BittensorSs58,
    // Per-chain variants for accounts whose address format is distinct
    // from EVM/Solana/Bitcoin. These exist so `address_algorithm` is
    // enough to pick the right derivation path without a chain hint.
    // Internally they still dispatch via `Chain::Tron`/`::Xrp`/etc.
    TronBase58Check,
    XrpBase58Check,
    StellarStrKey,
    SuiKeccak,
    AptosKeccak,
    IcpPrincipal,
}

#[derive(Clone, Copy)]
enum PublicKeyFormat {
    Auto,
    Compressed,
    Uncompressed,
    XOnly,
    Raw,
}

#[derive(Clone, Copy)]
enum ScriptType {
    Auto,
    P2pkh,
    P2shP2wpkh,
    P2wpkh,
    P2tr,
    Account,
}

#[uniffi::export]
pub fn derivation_derive(
    request: UniFFIDerivationRequest,
) -> Result<UniFFIDerivationResponse, crate::SpectraBridgeError> {
    let parsed = parse_uniffi_request(request)?;
    let result = derive(parsed)?;
    Ok(UniFFIDerivationResponse {
        address: result.address,
        public_key_hex: result.public_key_hex,
        private_key_hex: result.private_key_hex,
    })
}

#[uniffi::export]
pub fn derivation_derive_from_private_key(
    request: UniFFIPrivateKeyDerivationRequest,
) -> Result<UniFFIDerivationResponse, crate::SpectraBridgeError> {
    let parsed = parse_uniffi_private_key_request(request)?;
    let result = derive_from_private_key(parsed)?;
    Ok(UniFFIDerivationResponse {
        address: result.address,
        public_key_hex: result.public_key_hex,
        private_key_hex: result.private_key_hex,
    })
}

#[uniffi::export]
pub fn derivation_build_material(
    request: UniFFIMaterialRequest,
) -> Result<UniFFIMaterialResponse, crate::SpectraBridgeError> {
    let parsed = parse_uniffi_material_request(request)?;
    let result = build_material(parsed)?;
    Ok(UniFFIMaterialResponse {
        address: result.address,
        private_key_hex: result.private_key_hex,
        derivation_path: result.derivation_path,
        account: result.account,
        branch: result.branch,
        index: result.index,
    })
}

#[uniffi::export]
pub fn derivation_build_material_from_private_key(
    request: UniFFIPrivateKeyMaterialRequest,
) -> Result<UniFFIMaterialResponse, crate::SpectraBridgeError> {
    let parsed = parse_uniffi_private_key_material_request(request)?;
    let result = build_material_from_private_key(parsed)?;
    Ok(UniFFIMaterialResponse {
        address: result.address,
        private_key_hex: result.private_key_hex,
        derivation_path: result.derivation_path,
        account: result.account,
        branch: result.branch,
        index: result.index,
    })
}

#[uniffi::export]
pub fn derivation_derive_all_addresses(
    seed_phrase: String,
    chain_paths: std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, crate::SpectraBridgeError> {
    let mut results = std::collections::HashMap::new();
    for (chain_name, path) in &chain_paths {
        if let Some(address) = derive_address_for_chain(&seed_phrase, chain_name, path)
            .ok()
            .flatten()
        {
            results.insert(chain_name.clone(), address);
        }
    }
    Ok(results)
}

/// Derive a single address from a seed phrase, chain name, and derivation path,
/// using the canonical per-chain algorithm defaults.
fn derive_address_for_chain(
    seed_phrase: &str,
    chain_name: &str,
    path: &str,
) -> Result<Option<String>, crate::SpectraBridgeError> {
    let (chain_id, curve, deriv_alg, addr_alg, pubkey_fmt, script_opt) =
        match chain_defaults_from_name(chain_name) {
            Some(defaults) => defaults,
            None => return Ok(None), // unknown chain — skip silently
        };

    // For Bitcoin, script type depends on the purpose level in the path (44/49/84/86).
    // All other chains use a fixed script type supplied by chain_defaults_from_name.
    let script = script_opt.unwrap_or_else(|| script_type_from_purpose(path));

    let _ = chain_id; // chain is inferred from address_algorithm at parse time
    let request = UniFFIDerivationRequest {
        chain: None,
        network: NETWORK_MAINNET,
        curve,
        requested_outputs: OUTPUT_ADDRESS,
        derivation_algorithm: deriv_alg,
        address_algorithm: addr_alg,
        public_key_format: pubkey_fmt,
        script_type: script,
        seed_phrase: seed_phrase.to_string(),
        derivation_path: Some(path.to_string()),
        passphrase: None,
        hmac_key: None,
        mnemonic_wordlist: None,
        iteration_count: 0,
        salt_prefix: None,
    };

    let parsed = parse_uniffi_request(request)?;
    let output = derive(parsed)?;
    Ok(output.address)
}

/// Derive full key material (address, public key, private key) for a chain,
/// applying optional power-user overrides (passphrase, wordlist, custom
/// algorithm/curve/address overrides, etc.). Each override that is `Some`
/// replaces the chain preset default; fields that are `None` fall back to
/// the preset. Requests all three outputs so the caller can use the keys
/// for signing.
pub(crate) fn derive_key_material_for_chain_with_overrides(
    seed_phrase: &str,
    chain_name: &str,
    path: &str,
    overrides: Option<&crate::store::wallet_domain::CoreWalletDerivationOverrides>,
) -> Result<(String, String, String), crate::SpectraBridgeError> {
    let (chain_id, preset_curve, preset_deriv_alg, preset_addr_alg, preset_pubkey_fmt, script_opt) =
        chain_defaults_from_name(chain_name)
            .ok_or_else(|| format!("unsupported chain for derivation: {chain_name}"))?;
    let _ = chain_id;

    // Apply overrides where provided; otherwise fall back to chain preset.
    let (
        curve,
        deriv_alg,
        addr_alg,
        pubkey_fmt,
        script_type,
        passphrase,
        hmac_key,
        mnemonic_wordlist,
        iteration_count,
        salt_prefix,
    ) = if let Some(o) = overrides {
        let curve = o
            .curve
            .as_deref()
            .map(super::presets::curve_wire_value)
            .transpose()
            .map_err(crate::SpectraBridgeError::from)?
            .unwrap_or(preset_curve);
        let deriv_alg = o
            .derivation_algorithm
            .as_deref()
            .map(super::presets::derivation_algorithm_wire_value)
            .transpose()
            .map_err(crate::SpectraBridgeError::from)?
            .unwrap_or(preset_deriv_alg);
        let addr_alg = o
            .address_algorithm
            .as_deref()
            .map(super::presets::address_algorithm_wire_value)
            .transpose()
            .map_err(crate::SpectraBridgeError::from)?
            .unwrap_or(preset_addr_alg);
        let pubkey_fmt = o
            .public_key_format
            .as_deref()
            .map(super::presets::public_key_format_wire_value)
            .transpose()
            .map_err(crate::SpectraBridgeError::from)?
            .unwrap_or(preset_pubkey_fmt);
        let script_override = o
            .script_type
            .as_deref()
            .map(super::presets::script_type_wire_value)
            .transpose()
            .map_err(crate::SpectraBridgeError::from)?;
        let script = script_override
            .or(script_opt)
            .unwrap_or_else(|| script_type_from_purpose(path));
        (
            curve,
            deriv_alg,
            addr_alg,
            pubkey_fmt,
            script,
            o.passphrase.clone(),
            o.hmac_key.clone(),
            o.mnemonic_wordlist.clone(),
            o.iteration_count.unwrap_or(0),
            o.salt_prefix.clone(),
        )
    } else {
        let script = script_opt.unwrap_or_else(|| script_type_from_purpose(path));
        (
            preset_curve,
            preset_deriv_alg,
            preset_addr_alg,
            preset_pubkey_fmt,
            script,
            None,
            None,
            None,
            0,
            None,
        )
    };

    let request = UniFFIDerivationRequest {
        chain: None,
        network: NETWORK_MAINNET,
        curve,
        requested_outputs: OUTPUT_ADDRESS | OUTPUT_PUBLIC_KEY | OUTPUT_PRIVATE_KEY,
        derivation_algorithm: deriv_alg,
        address_algorithm: addr_alg,
        public_key_format: pubkey_fmt,
        script_type,
        seed_phrase: seed_phrase.to_string(),
        derivation_path: Some(path.to_string()),
        passphrase,
        hmac_key,
        mnemonic_wordlist,
        iteration_count,
        salt_prefix,
    };

    let parsed = parse_uniffi_request(request)?;
    let output = derive(parsed)?;

    let address = output.address.ok_or("derivation did not produce address")?;
    let pub_hex = output.public_key_hex.ok_or("derivation did not produce public key")?;
    let priv_hex = output.private_key_hex.ok_or("derivation did not produce private key")?;

    Ok((address, priv_hex, pub_hex))
}

/// Map a chain display name to its canonical algorithm defaults.
///
/// Returns `(chain_id, curve, derivation_algorithm, address_algorithm, public_key_format, script_type)`.
/// `script_type = None` means the caller should infer it from the path's purpose level
/// (used for Bitcoin where the address format varies by purpose: 44/49/84/86).
///
/// Data-driven from [core/data/derivation_presets.toml](../../data/derivation_presets.toml);
/// see `derivation/presets.rs`.
fn chain_defaults_from_name(name: &str) -> Option<(u32, u32, u32, u32, u32, Option<u32>)> {
    let preset = super::presets::preset_by_name(name)?;
    Some((
        preset.chain_id,
        preset.curve,
        preset.derivation_algorithm,
        preset.address_algorithm,
        preset.public_key_format,
        preset.script_type,
    ))
}

/// Parse the BIP-32 purpose level from a derivation path and return the matching
/// Bitcoin script type constant.  Defaults to P2PKH when the purpose is unknown.
///
/// Examples: `m/44'/…` → P2PKH, `m/49'/…` → P2SH-P2WPKH,
///           `m/84'/…` → P2WPKH, `m/86'/…` → P2TR.
fn script_type_from_purpose(path: &str) -> u32 {
    let without_prefix = path
        .trim_start_matches('m')
        .trim_start_matches('M')
        .trim_start_matches('/');
    let purpose_segment = without_prefix.split('/').next().unwrap_or("");
    let purpose_str = purpose_segment
        .trim_end_matches('\'')
        .trim_end_matches('h');
    match purpose_str {
        "44" => SCRIPT_P2PKH,
        "49" => SCRIPT_P2SH_P2WPKH,
        "84" => SCRIPT_P2WPKH,
        "86" => SCRIPT_P2TR,
        _ => SCRIPT_P2PKH,
    }
}

fn parse_uniffi_request(
    request: UniFFIDerivationRequest,
) -> Result<ParsedRequest, crate::SpectraBridgeError> {
    let seed_phrase = normalize_seed_phrase(&request.seed_phrase);
    if seed_phrase.is_empty() {
        return Err(crate::SpectraBridgeError::from("Seed phrase is empty."));
    }

    // Avoid the unconditional `.trim().to_string()` allocation: keep the
    // original owned String when no whitespace was present (the common case
    // for paths like "m/44'/60'/0'/0/0", language names like "english", and
    // hex hmac keys). Re-allocate only when trim actually shortens the value.
    fn trim_in_place(value: String) -> Option<String> {
        let trimmed_len = value.trim().len();
        if trimmed_len == 0 {
            None
        } else if trimmed_len == value.len() {
            Some(value)
        } else {
            Some(value.trim().to_string())
        }
    }
    let derivation_path = request.derivation_path.and_then(trim_in_place);
    let passphrase = request.passphrase.unwrap_or_default();
    let hmac_key = request.hmac_key.and_then(trim_in_place);
    let mnemonic_wordlist = request.mnemonic_wordlist.and_then(trim_in_place);
    // `salt_prefix` is intentionally NOT trimmed — callers may legitimately use
    // a prefix that includes or consists entirely of whitespace. Only an
    // explicit `None` falls back to the BIP-39 default of "mnemonic".
    let salt_prefix = request.salt_prefix;

    if request.requested_outputs == 0 {
        return Err(crate::SpectraBridgeError::from(
            "At least one output must be requested.",
        ));
    }
    let known_outputs = OUTPUT_ADDRESS | OUTPUT_PUBLIC_KEY | OUTPUT_PRIVATE_KEY;
    if request.requested_outputs & !known_outputs != 0 {
        return Err(crate::SpectraBridgeError::from(
            "Requested outputs contain unsupported output flags.",
        ));
    }

    let address_algorithm = parse_address_algorithm(request.address_algorithm)?;
    let chain = match request.chain {
        Some(value) => parse_chain(value)?,
        None => chain_from_address_algorithm(address_algorithm)?,
    };
    Ok(ParsedRequest {
        chain,
        network: parse_network(request.network)?,
        curve: parse_curve(request.curve)?,
        requested_outputs: request.requested_outputs,
        derivation_algorithm: parse_derivation_algorithm(request.derivation_algorithm)?,
        address_algorithm,
        public_key_format: parse_public_key_format(request.public_key_format)?,
        script_type: parse_script_type(request.script_type)?,
        seed_phrase,
        derivation_path,
        passphrase,
        hmac_key,
        mnemonic_wordlist,
        iteration_count: request.iteration_count,
        salt_prefix,
    })
}

fn parse_uniffi_private_key_request(
    request: UniFFIPrivateKeyDerivationRequest,
) -> Result<ParsedPrivateKeyRequest, crate::SpectraBridgeError> {
    let address_algorithm = parse_address_algorithm(request.address_algorithm)?;
    let chain = match request.chain {
        Some(value) => parse_chain(value)?,
        None => chain_from_address_algorithm(address_algorithm)?,
    };
    Ok(ParsedPrivateKeyRequest {
        chain,
        network: parse_network(request.network)?,
        curve: parse_curve(request.curve)?,
        address_algorithm,
        public_key_format: parse_public_key_format(request.public_key_format)?,
        script_type: parse_script_type(request.script_type)?,
        private_key: decode_private_key_hex(&request.private_key_hex)?,
    })
}

struct ParsedPrivateKeyRequest {
    chain: Chain,
    network: NetworkFlavor,
    curve: CurveFamily,
    address_algorithm: AddressAlgorithm,
    public_key_format: PublicKeyFormat,
    script_type: ScriptType,
    private_key: [u8; 32],
}

struct ParsedMaterialRequest {
    request: ParsedRequest,
    derivation_path: String,
}

struct ParsedPrivateKeyMaterialRequest {
    request: ParsedPrivateKeyRequest,
    derivation_path: String,
}

struct DerivedMaterial {
    address: String,
    private_key_hex: String,
    derivation_path: String,
    account: u32,
    branch: u32,
    index: u32,
}

fn decode_private_key_hex(raw: &str) -> Result<[u8; 32], String> {
    let value = raw.trim();
    if value.len() != 64 {
        return Err("Private key hex must be exactly 64 hex characters.".to_string());
    }

    let decoded = hex::decode(value).map_err(display_error)?;
    if decoded.len() != 32 {
        return Err("Private key must decode to 32 bytes.".to_string());
    }

    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

fn encode_private_key_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Trim + non-empty validation shared by both material-request parsers.
/// Returns the trimmed path without re-allocating when no whitespace was
/// present (the common case for canonical derivation paths).
fn require_material_path(raw: String) -> Result<String, crate::SpectraBridgeError> {
    let trimmed_len = raw.trim().len();
    if trimmed_len == 0 {
        return Err(crate::SpectraBridgeError::from(
            "Derivation path is required to build signing material.",
        ));
    }
    if trimmed_len == raw.len() {
        Ok(raw)
    } else {
        Ok(raw.trim().to_string())
    }
}

fn parse_uniffi_material_request(
    request: UniFFIMaterialRequest,
) -> Result<ParsedMaterialRequest, crate::SpectraBridgeError> {
    let derivation_path = require_material_path(request.derivation_path)?;
    let parsed = parse_uniffi_request(UniFFIDerivationRequest {
        chain: request.chain,
        network: request.network,
        curve: request.curve,
        requested_outputs: OUTPUT_ADDRESS | OUTPUT_PRIVATE_KEY,
        derivation_algorithm: request.derivation_algorithm,
        address_algorithm: request.address_algorithm,
        public_key_format: request.public_key_format,
        script_type: request.script_type,
        seed_phrase: request.seed_phrase,
        derivation_path: Some(derivation_path.clone()),
        passphrase: request.passphrase,
        hmac_key: request.hmac_key,
        mnemonic_wordlist: request.mnemonic_wordlist,
        iteration_count: request.iteration_count,
        salt_prefix: request.salt_prefix,
    })?;
    Ok(ParsedMaterialRequest {
        request: parsed,
        derivation_path,
    })
}

fn parse_uniffi_private_key_material_request(
    request: UniFFIPrivateKeyMaterialRequest,
) -> Result<ParsedPrivateKeyMaterialRequest, crate::SpectraBridgeError> {
    let derivation_path = require_material_path(request.derivation_path)?;
    let parsed = parse_uniffi_private_key_request(UniFFIPrivateKeyDerivationRequest {
        chain: request.chain,
        network: request.network,
        curve: request.curve,
        address_algorithm: request.address_algorithm,
        public_key_format: request.public_key_format,
        script_type: request.script_type,
        private_key_hex: request.private_key_hex,
    })?;
    Ok(ParsedPrivateKeyMaterialRequest {
        request: parsed,
        derivation_path,
    })
}

fn build_material(
    request: ParsedMaterialRequest,
) -> Result<DerivedMaterial, crate::SpectraBridgeError> {
    let result = derive(request.request)?;
    let address = result.address.ok_or_else(|| {
        crate::SpectraBridgeError::from("Derived material did not contain an address.")
    })?;
    let private_key_hex = result.private_key_hex.ok_or_else(|| {
        crate::SpectraBridgeError::from("Derived material did not contain a private key.")
    })?;
    let (account, branch, index) = parse_account_branch_index(&request.derivation_path);
    Ok(DerivedMaterial {
        address,
        private_key_hex,
        derivation_path: request.derivation_path,
        account,
        branch,
        index,
    })
}

fn build_material_from_private_key(
    request: ParsedPrivateKeyMaterialRequest,
) -> Result<DerivedMaterial, crate::SpectraBridgeError> {
    let ParsedPrivateKeyMaterialRequest {
        request,
        derivation_path,
    } = request;
    let private_key_hex = encode_private_key_hex(&request.private_key);
    let result = derive_from_private_key(request)?;
    let address = result.address.ok_or_else(|| {
        crate::SpectraBridgeError::from("Derived material did not contain an address.")
    })?;
    let (account, branch, index) = parse_account_branch_index(&derivation_path);
    Ok(DerivedMaterial {
        address,
        private_key_hex,
        derivation_path,
        account,
        branch,
        index,
    })
}

fn parse_account_branch_index(path: &str) -> (u32, u32, u32) {
    let trimmed = path.trim();
    let Some(stripped) = trimmed
        .strip_prefix("m/")
        .or_else(|| trimmed.strip_prefix("M/"))
    else {
        return (0, 0, 0);
    };
    let segments = stripped
        .split('/')
        .filter_map(|segment| {
            let cleaned = segment.trim_end_matches('\'');
            cleaned.parse::<u32>().ok()
        })
        .collect::<Vec<_>>();
    let account = segments.get(2).copied().unwrap_or(0);
    let branch = segments
        .get(segments.len().saturating_sub(2))
        .copied()
        .unwrap_or(0);
    let index = segments.last().copied().unwrap_or(0);
    (account, branch, index)
}

fn derive_from_private_key(request: ParsedPrivateKeyRequest) -> Result<DerivedOutput, String> {
    // secp chains use secp256k1 address encodings; ed25519 chains use
    // chain-specific ed25519 address logic.
    if is_secp_chain(request.chain) {
        if request.curve != CurveFamily::Secp256k1 {
            return Err("This chain currently requires secp256k1.".to_string());
        }

        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&request.private_key).map_err(display_error)?;
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);

        let address = derive_address_from_keys(
            request.chain,
            request.network,
            request.address_algorithm,
            request.script_type,
            &public_key,
            &secp,
        )?;

        return Ok(DerivedOutput {
            address: Some(address),
            public_key_hex: Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?)),
            private_key_hex: Some(hex::encode(request.private_key)),
        });
    }

    // Polkadot / Bittensor share sr25519 derivation. Difference is the SS58
    // network prefix used in the human-readable address: 0 for Polkadot, 42
    // for Bittensor (substrate-generic).
    if matches!(request.chain, Chain::Polkadot | Chain::Bittensor) {
        if request.curve != CurveFamily::Sr25519 {
            return Err("Polkadot/Bittensor currently require sr25519.".to_string());
        }
        let mini = schnorrkel::MiniSecretKey::from_bytes(&request.private_key)
            .map_err(|e| format!("Invalid sr25519 mini-secret: {e}"))?;
        let keypair = mini.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
        let public_key = keypair.public.to_bytes();
        let prefix = if matches!(request.chain, Chain::Bittensor) { 42 } else { 0 };
        return Ok(DerivedOutput {
            address: Some(encode_ss58(&public_key, prefix)),
            public_key_hex: Some(hex::encode(public_key)),
            private_key_hex: Some(hex::encode(request.private_key)),
        });
    }

    // Monero: the 32-byte private key is the spend seed; sc_reduce32 + Keccak
    // produce the full spend/view key pair and we encode as Monero main address.
    if matches!(request.chain, Chain::Monero) {
        if request.curve != CurveFamily::Ed25519 {
            return Err("Monero currently requires ed25519.".to_string());
        }
        let (private_spend, public_spend, _private_view, public_view) =
            derive_monero_keys_from_spend_seed(&request.private_key)?;
        let address = encode_monero_main_address(&public_spend, &public_view, request.network)?;
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&public_spend);
        both[32..].copy_from_slice(&public_view);
        return Ok(DerivedOutput {
            address: Some(address),
            public_key_hex: Some(hex::encode(both)),
            private_key_hex: Some(hex::encode(private_spend)),
        });
    }

    if request.curve != CurveFamily::Ed25519 {
        return Err("This chain currently requires ed25519.".to_string());
    }

    let signing_key = SigningKey::from_bytes(&request.private_key);
    let public_key = signing_key.verifying_key().to_bytes();
    let address = derive_ed25519_chain_address(request.chain, request.address_algorithm, &public_key)?;

    Ok(DerivedOutput {
        address: Some(address),
        public_key_hex: Some(hex::encode(public_key)),
        private_key_hex: Some(hex::encode(request.private_key)),
    })
}

fn derive_address_from_keys(
    chain: Chain,
    network: NetworkFlavor,
    address_algorithm: AddressAlgorithm,
    script_type: ScriptType,
    public_key: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    match chain {
        Chain::Bitcoin => {
            if !matches!(address_algorithm, AddressAlgorithm::Bitcoin) {
                return Err("Bitcoin requests require the Bitcoin address algorithm.".to_string());
            }
            derive_bitcoin_address_for_network(
                bitcoin_network_params(network),
                script_type,
                public_key,
                secp,
            )
        }
        Chain::BitcoinCash | Chain::BitcoinSv => {
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::BitcoinGold => {
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![0x26u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Decred => {
            use crate::derivation::chains::decred::{dcr_hash160, encode_dcr_p2pkh};
            let pubkey_hash = dcr_hash160(&public_key.serialize());
            Ok(encode_dcr_p2pkh(&pubkey_hash))
        }
        Chain::Kaspa => {
            use crate::derivation::chains::kaspa::encode_kaspa_schnorr;
            let serialized = public_key.serialize();
            let mut x_only = [0u8; 32];
            x_only.copy_from_slice(&serialized[1..33]);
            Ok(encode_kaspa_schnorr(&x_only))
        }
        Chain::Dash => {
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![0x4Cu8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Litecoin => {
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![0x30u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Zcash => {
            // Mainnet transparent P2PKH: 2-byte version 0x1CB8 || hash160 ||
            // 4-byte sha256d checksum, base58-encoded → "t1..." addresses.
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![0x1Cu8, 0xB8u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Dogecoin => {
            let version = if matches!(network, NetworkFlavor::Testnet) {
                0x71
            } else {
                0x1e
            };
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![version];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Ethereum
        | Chain::EthereumClassic
        | Chain::Arbitrum
        | Chain::Optimism
        | Chain::Avalanche
        | Chain::Hyperliquid => Ok(derive_evm_address(public_key)),
        Chain::Tron => {
            let evm_address = derive_evm_address_bytes(public_key);
            let mut payload = vec![0x41u8];
            payload.extend_from_slice(&evm_address);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Xrp => {
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![0x00u8];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::RIPPLE))
        }
        _ => Err("Unsupported secp256k1 chain for private-key address derivation.".to_string()),
    }
}

fn bitcoin_network_params(network: NetworkFlavor) -> BitcoinNetworkParams {
    match network {
        NetworkFlavor::Mainnet => BTC_MAINNET,
        NetworkFlavor::Testnet | NetworkFlavor::Testnet4 | NetworkFlavor::Signet => BTC_TESTNET,
    }
}

fn derive_ed25519_chain_address(
    chain: Chain,
    address_algorithm: AddressAlgorithm,
    public_key: &[u8; 32],
) -> Result<String, String> {
    match chain {
        Chain::Solana => Ok(bs58::encode(public_key).into_string()),
        Chain::Stellar => {
            let encoded = base32_no_pad(public_key);
            let stellar_address = format!("G{}", &encoded[..55.min(encoded.len())]);
            if stellar_address.len() < 56 {
                Ok(format!(
                    "{}{}",
                    stellar_address,
                    "A".repeat(56 - stellar_address.len())
                ))
            } else {
                Ok(stellar_address)
            }
        }
        Chain::Cardano => {
            derive_cardano_shelley_enterprise_address(public_key, NetworkFlavor::Mainnet)
        }
        Chain::Sui => {
            let mut hasher = Keccak::v256();
            let mut digest = [0u8; 32];
            hasher.update(&[0x00]);
            hasher.update(public_key);
            hasher.finalize(&mut digest);
            Ok(format!("0x{}", hex::encode(digest)))
        }
        Chain::Aptos => {
            let mut hasher = Keccak::v256();
            let mut digest = [0u8; 32];
            hasher.update(public_key);
            hasher.update(&[0x00]);
            hasher.finalize(&mut digest);
            Ok(format!("0x{}", hex::encode(digest)))
        }
        Chain::Ton => format_ton_address(public_key, address_algorithm),
        Chain::InternetComputer => {
            let mut data = Vec::from(*public_key);
            data.extend_from_slice(b"icp");
            let digest = sha256_bytes(&data);
            let digest2 = sha256_bytes(&digest);
            Ok(hex::encode(digest2))
        }
        Chain::Near => Ok(hex::encode(public_key)),
        _ => Err("Unsupported ed25519 chain for private-key address derivation.".to_string()),
    }
}

/// Infer the internal `Chain` dispatch tag from the address algorithm.
/// This is what makes `chain` optional on the public request: every
/// supported `AddressAlgorithm` variant is 1:1 with a concrete chain
/// (or, for EVM, any chain in the EVM family — derivation output is
/// identical across them, so we pick Ethereum as the canonical tag).
fn chain_from_address_algorithm(alg: AddressAlgorithm) -> Result<Chain, String> {
    Ok(match alg {
        AddressAlgorithm::Bitcoin => Chain::Bitcoin,
        AddressAlgorithm::Litecoin => Chain::Litecoin,
        AddressAlgorithm::Dogecoin => Chain::Dogecoin,
        AddressAlgorithm::BitcoinCashLegacy => Chain::BitcoinCash,
        AddressAlgorithm::BitcoinSvLegacy => Chain::BitcoinSv,
        AddressAlgorithm::Evm => Chain::Ethereum,
        AddressAlgorithm::TronBase58Check => Chain::Tron,
        AddressAlgorithm::XrpBase58Check => Chain::Xrp,
        AddressAlgorithm::Solana => Chain::Solana,
        AddressAlgorithm::StellarStrKey => Chain::Stellar,
        AddressAlgorithm::SuiKeccak => Chain::Sui,
        AddressAlgorithm::AptosKeccak => Chain::Aptos,
        AddressAlgorithm::IcpPrincipal => Chain::InternetComputer,
        AddressAlgorithm::NearHex => Chain::Near,
        AddressAlgorithm::TonRawAccountId | AddressAlgorithm::TonV4R2 => Chain::Ton,
        AddressAlgorithm::CardanoShelleyEnterprise => Chain::Cardano,
        AddressAlgorithm::Ss58 => Chain::Polkadot,
        AddressAlgorithm::MoneroMain => Chain::Monero,
        AddressAlgorithm::ZcashTransparent => Chain::Zcash,
        AddressAlgorithm::BitcoinGoldLegacy => Chain::BitcoinGold,
        AddressAlgorithm::DecredP2pkh => Chain::Decred,
        AddressAlgorithm::KaspaSchnorr => Chain::Kaspa,
        AddressAlgorithm::DashLegacy => Chain::Dash,
        AddressAlgorithm::BittensorSs58 => Chain::Bittensor,
        AddressAlgorithm::Auto => {
            return Err(
                "Address algorithm must be explicit to derive chain automatically.".to_string(),
            )
        }
    })
}

fn parse_chain(value: u32) -> Result<Chain, String> {
    match value {
        CHAIN_BITCOIN => Ok(Chain::Bitcoin),
        CHAIN_ETHEREUM => Ok(Chain::Ethereum),
        CHAIN_SOLANA => Ok(Chain::Solana),
        CHAIN_BITCOIN_CASH => Ok(Chain::BitcoinCash),
        CHAIN_BITCOIN_SV => Ok(Chain::BitcoinSv),
        CHAIN_LITECOIN => Ok(Chain::Litecoin),
        CHAIN_DOGECOIN => Ok(Chain::Dogecoin),
        CHAIN_ETHEREUM_CLASSIC => Ok(Chain::EthereumClassic),
        CHAIN_ARBITRUM => Ok(Chain::Arbitrum),
        CHAIN_OPTIMISM => Ok(Chain::Optimism),
        CHAIN_AVALANCHE => Ok(Chain::Avalanche),
        CHAIN_HYPERLIQUID => Ok(Chain::Hyperliquid),
        CHAIN_TRON => Ok(Chain::Tron),
        CHAIN_STELLAR => Ok(Chain::Stellar),
        CHAIN_XRP => Ok(Chain::Xrp),
        CHAIN_CARDANO => Ok(Chain::Cardano),
        CHAIN_SUI => Ok(Chain::Sui),
        CHAIN_APTOS => Ok(Chain::Aptos),
        CHAIN_TON => Ok(Chain::Ton),
        CHAIN_INTERNET_COMPUTER => Ok(Chain::InternetComputer),
        CHAIN_NEAR => Ok(Chain::Near),
        CHAIN_POLKADOT => Ok(Chain::Polkadot),
        CHAIN_MONERO => Ok(Chain::Monero),
        CHAIN_ZCASH => Ok(Chain::Zcash),
        CHAIN_BITCOIN_GOLD => Ok(Chain::BitcoinGold),
        CHAIN_DECRED => Ok(Chain::Decred),
        CHAIN_KASPA => Ok(Chain::Kaspa),
        CHAIN_DASH => Ok(Chain::Dash),
        CHAIN_BITTENSOR => Ok(Chain::Bittensor),
        other => Err(format!("Unsupported chain id: {other}")),
    }
}

fn parse_network(value: u32) -> Result<NetworkFlavor, String> {
    match value {
        NETWORK_MAINNET => Ok(NetworkFlavor::Mainnet),
        NETWORK_TESTNET => Ok(NetworkFlavor::Testnet),
        NETWORK_TESTNET4 => Ok(NetworkFlavor::Testnet4),
        NETWORK_SIGNET => Ok(NetworkFlavor::Signet),
        other => Err(format!("Unsupported network id: {other}")),
    }
}

fn parse_curve(value: u32) -> Result<CurveFamily, String> {
    match value {
        CURVE_SECP256K1 => Ok(CurveFamily::Secp256k1),
        CURVE_ED25519 => Ok(CurveFamily::Ed25519),
        CURVE_SR25519 => Ok(CurveFamily::Sr25519),
        other => Err(format!("Unsupported curve id: {other}")),
    }
}

fn parse_derivation_algorithm(value: u32) -> Result<DerivationAlgorithm, String> {
    match value {
        DERIVATION_AUTO => Ok(DerivationAlgorithm::Auto),
        DERIVATION_BIP32_SECP256K1 => Ok(DerivationAlgorithm::Bip32Secp256k1),
        DERIVATION_SLIP10_ED25519 => Ok(DerivationAlgorithm::Slip10Ed25519),
        DERIVATION_DIRECT_SEED_ED25519 => Ok(DerivationAlgorithm::DirectSeedEd25519),
        DERIVATION_TON_MNEMONIC => Ok(DerivationAlgorithm::TonMnemonic),
        DERIVATION_BIP32_ED25519_ICARUS => Ok(DerivationAlgorithm::Bip32Ed25519Icarus),
        DERIVATION_SUBSTRATE_BIP39 => Ok(DerivationAlgorithm::SubstrateBip39),
        DERIVATION_MONERO_BIP39 => Ok(DerivationAlgorithm::MoneroBip39),
        other => Err(format!("Unsupported derivation algorithm id: {other}")),
    }
}

fn parse_address_algorithm(value: u32) -> Result<AddressAlgorithm, String> {
    match value {
        ADDRESS_AUTO => Ok(AddressAlgorithm::Auto),
        ADDRESS_BITCOIN => Ok(AddressAlgorithm::Bitcoin),
        ADDRESS_EVM => Ok(AddressAlgorithm::Evm),
        ADDRESS_SOLANA => Ok(AddressAlgorithm::Solana),
        ADDRESS_NEAR_HEX => Ok(AddressAlgorithm::NearHex),
        ADDRESS_TON_RAW_ACCOUNT_ID => Ok(AddressAlgorithm::TonRawAccountId),
        ADDRESS_CARDANO_SHELLEY_ENTERPRISE => Ok(AddressAlgorithm::CardanoShelleyEnterprise),
        ADDRESS_SS58 => Ok(AddressAlgorithm::Ss58),
        ADDRESS_MONERO_MAIN => Ok(AddressAlgorithm::MoneroMain),
        ADDRESS_TON_V4R2 => Ok(AddressAlgorithm::TonV4R2),
        ADDRESS_LITECOIN => Ok(AddressAlgorithm::Litecoin),
        ADDRESS_DOGECOIN => Ok(AddressAlgorithm::Dogecoin),
        ADDRESS_BITCOIN_CASH_LEGACY => Ok(AddressAlgorithm::BitcoinCashLegacy),
        ADDRESS_BITCOIN_SV_LEGACY => Ok(AddressAlgorithm::BitcoinSvLegacy),
        ADDRESS_TRON_BASE58_CHECK => Ok(AddressAlgorithm::TronBase58Check),
        ADDRESS_XRP_BASE58_CHECK => Ok(AddressAlgorithm::XrpBase58Check),
        ADDRESS_STELLAR_STRKEY => Ok(AddressAlgorithm::StellarStrKey),
        ADDRESS_SUI_KECCAK => Ok(AddressAlgorithm::SuiKeccak),
        ADDRESS_APTOS_KECCAK => Ok(AddressAlgorithm::AptosKeccak),
        ADDRESS_ICP_PRINCIPAL => Ok(AddressAlgorithm::IcpPrincipal),
        ADDRESS_ZCASH_TRANSPARENT => Ok(AddressAlgorithm::ZcashTransparent),
        ADDRESS_BITCOIN_GOLD_LEGACY => Ok(AddressAlgorithm::BitcoinGoldLegacy),
        ADDRESS_DECRED_P2PKH => Ok(AddressAlgorithm::DecredP2pkh),
        ADDRESS_KASPA_SCHNORR => Ok(AddressAlgorithm::KaspaSchnorr),
        ADDRESS_DASH_LEGACY => Ok(AddressAlgorithm::DashLegacy),
        ADDRESS_BITTENSOR_SS58 => Ok(AddressAlgorithm::BittensorSs58),
        other => Err(format!("Unsupported address algorithm id: {other}")),
    }
}

fn parse_public_key_format(value: u32) -> Result<PublicKeyFormat, String> {
    match value {
        PUBLIC_KEY_AUTO => Ok(PublicKeyFormat::Auto),
        PUBLIC_KEY_COMPRESSED => Ok(PublicKeyFormat::Compressed),
        PUBLIC_KEY_UNCOMPRESSED => Ok(PublicKeyFormat::Uncompressed),
        PUBLIC_KEY_X_ONLY => Ok(PublicKeyFormat::XOnly),
        PUBLIC_KEY_RAW => Ok(PublicKeyFormat::Raw),
        other => Err(format!("Unsupported public key format id: {other}")),
    }
}

fn parse_script_type(value: u32) -> Result<ScriptType, String> {
    match value {
        SCRIPT_AUTO => Ok(ScriptType::Auto),
        SCRIPT_P2PKH => Ok(ScriptType::P2pkh),
        SCRIPT_P2SH_P2WPKH => Ok(ScriptType::P2shP2wpkh),
        SCRIPT_P2WPKH => Ok(ScriptType::P2wpkh),
        SCRIPT_P2TR => Ok(ScriptType::P2tr),
        SCRIPT_ACCOUNT => Ok(ScriptType::Account),
        other => Err(format!("Unsupported script type id: {other}")),
    }
}

fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    // Validate cross-field constraints before dispatching to chain-specific logic.
    validate_request(&request)?;

    // Dispatch by chain family; each branch handles address/key formatting specifics.
    match request.chain {
        Chain::Bitcoin => derive_bitcoin(request),
        Chain::BitcoinCash => derive_bitcoin_legacy_family(request, 0x00),
        Chain::BitcoinSv => derive_bitcoin_legacy_family(request, 0x00),
        Chain::Litecoin => derive_litecoin(request),
        Chain::Dogecoin => derive_dogecoin(request),
        Chain::Ethereum
        | Chain::EthereumClassic
        | Chain::Arbitrum
        | Chain::Optimism
        | Chain::Avalanche
        | Chain::Hyperliquid => derive_evm_family(request),
        Chain::Tron => derive_tron(request),
        Chain::Solana => derive_solana(request),
        Chain::Stellar => derive_stellar(request),
        Chain::Xrp => derive_xrp(request),
        Chain::Cardano => derive_cardano(request),
        Chain::Sui => derive_sui(request),
        Chain::Aptos => derive_aptos(request),
        Chain::Ton => derive_ton(request),
        Chain::InternetComputer => derive_icp(request),
        Chain::Near => derive_near(request),
        Chain::Polkadot => derive_polkadot(request),
        Chain::Bittensor => derive_bittensor(request),
        Chain::Monero => derive_monero(request),
        Chain::Zcash => derive_zcash_transparent(request),
        Chain::BitcoinGold => derive_bitcoin_legacy_family(request, 0x26),
        Chain::Decred => derive_decred(request),
        Chain::Kaspa => derive_kaspa(request),
        Chain::Dash => derive_bitcoin_legacy_family(request, 0x4C),
    }
}

fn validate_request(request: &ParsedRequest) -> Result<(), String> {
    // Enforce curve/algorithm compatibility. Wordlist, salt prefix, iteration
    // count, and HMAC key are user-customizable and resolved downstream.
    if request.iteration_count == 1 {
        return Err("Iteration count must be 0 (default) or >= 2.".to_string());
    }

    // Validate the wordlist identifier up-front so misspellings fail fast.
    let _ = resolve_bip39_language(request.mnemonic_wordlist.as_deref())?;

    if is_secp_chain(request.chain) {
        if request.curve != CurveFamily::Secp256k1 {
            return Err("This chain currently requires secp256k1.".to_string());
        }
        if matches!(request.derivation_algorithm, DerivationAlgorithm::Auto) {
            return Err("Derivation algorithm must be explicit for secp256k1 chains.".to_string());
        }
        if matches!(
            request.derivation_algorithm,
            DerivationAlgorithm::Slip10Ed25519
        ) {
            return Err("This chain does not support SLIP-0010 ed25519 derivation.".to_string());
        }
    } else if matches!(request.chain, Chain::Polkadot | Chain::Bittensor) {
        if request.curve != CurveFamily::Sr25519 {
            return Err("Polkadot/Bittensor currently require sr25519.".to_string());
        }
        if !matches!(
            request.derivation_algorithm,
            DerivationAlgorithm::SubstrateBip39
        ) {
            return Err("Polkadot/Bittensor derivation algorithm must be substrate-bip39.".to_string());
        }
    } else {
        if request.curve != CurveFamily::Ed25519 {
            return Err("This chain currently requires ed25519.".to_string());
        }
        if matches!(request.derivation_algorithm, DerivationAlgorithm::Auto) {
            return Err("Derivation algorithm must be explicit for ed25519 chains.".to_string());
        }
        if matches!(
            request.derivation_algorithm,
            DerivationAlgorithm::Bip32Secp256k1
        ) {
            return Err("This chain does not support BIP-32 secp256k1 derivation.".to_string());
        }
        if matches!(
            request.derivation_algorithm,
            DerivationAlgorithm::SubstrateBip39
        ) {
            return Err("Substrate BIP-39 derivation is reserved for sr25519 chains.".to_string());
        }
        // DirectSeedEd25519, TonMnemonic, Slip10Ed25519, Bip32Ed25519Icarus,
        // MoneroBip39 are all valid for ed25519 chains; pipeline selection
        // happens in `derive_ed25519_material` (or in chain-specific code).
    }

    if !is_network_supported(request.chain, request.network) {
        return Err("Network is not supported for this chain.".to_string());
    }

    validate_request_algorithms(request)?;

    Ok(())
}

fn validate_request_algorithms(request: &ParsedRequest) -> Result<(), String> {
    if matches!(request.address_algorithm, AddressAlgorithm::Auto) {
        return Err("Address algorithm must be explicit.".to_string());
    }
    if matches!(request.public_key_format, PublicKeyFormat::Auto) {
        return Err("Public key format must be explicit.".to_string());
    }

    if matches!(request.chain, Chain::Bitcoin) {
        if !matches!(
            request.script_type,
            ScriptType::P2pkh | ScriptType::P2shP2wpkh | ScriptType::P2wpkh | ScriptType::P2tr
        ) {
            return Err(
                "Bitcoin script type must be explicit (p2pkh/p2sh-p2wpkh/p2wpkh/p2tr).".to_string(),
            );
        }
    } else if matches!(request.script_type, ScriptType::Auto) {
        return Err("Script type must be explicit.".to_string());
    }

    Ok(())
}

/// Inverse of `parse_chain`: map a `Chain` enum variant back to its numeric
/// id. Used to look up presets loaded from `derivation_presets.toml`.
fn chain_id(chain: Chain) -> u32 {
    match chain {
        Chain::Bitcoin => CHAIN_BITCOIN,
        Chain::Ethereum => CHAIN_ETHEREUM,
        Chain::Solana => CHAIN_SOLANA,
        Chain::BitcoinCash => CHAIN_BITCOIN_CASH,
        Chain::BitcoinSv => CHAIN_BITCOIN_SV,
        Chain::Litecoin => CHAIN_LITECOIN,
        Chain::Dogecoin => CHAIN_DOGECOIN,
        Chain::EthereumClassic => CHAIN_ETHEREUM_CLASSIC,
        Chain::Arbitrum => CHAIN_ARBITRUM,
        Chain::Optimism => CHAIN_OPTIMISM,
        Chain::Avalanche => CHAIN_AVALANCHE,
        Chain::Hyperliquid => CHAIN_HYPERLIQUID,
        Chain::Tron => CHAIN_TRON,
        Chain::Stellar => CHAIN_STELLAR,
        Chain::Xrp => CHAIN_XRP,
        Chain::Cardano => CHAIN_CARDANO,
        Chain::Sui => CHAIN_SUI,
        Chain::Aptos => CHAIN_APTOS,
        Chain::Ton => CHAIN_TON,
        Chain::InternetComputer => CHAIN_INTERNET_COMPUTER,
        Chain::Near => CHAIN_NEAR,
        Chain::Polkadot => CHAIN_POLKADOT,
        Chain::Bittensor => CHAIN_BITTENSOR,
        Chain::Monero => CHAIN_MONERO,
        Chain::Zcash => CHAIN_ZCASH,
        Chain::BitcoinGold => CHAIN_BITCOIN_GOLD,
        Chain::Decred => CHAIN_DECRED,
        Chain::Kaspa => CHAIN_KASPA,
        Chain::Dash => CHAIN_DASH,
    }
}

fn is_secp_chain(chain: Chain) -> bool {
    super::presets::preset_by_chain_id(chain_id(chain))
        .map(|preset| preset.curve == CURVE_SECP256K1)
        .unwrap_or(false)
}

fn is_network_supported(chain: Chain, network: NetworkFlavor) -> bool {
    let wire_network = match network {
        NetworkFlavor::Mainnet => NETWORK_MAINNET,
        NetworkFlavor::Testnet => NETWORK_TESTNET,
        NetworkFlavor::Testnet4 => NETWORK_TESTNET4,
        NetworkFlavor::Signet => NETWORK_SIGNET,
    };
    super::presets::preset_by_chain_id(chain_id(chain))
        .map(|preset| preset.networks.contains(&wire_network))
        .unwrap_or(false)
}

fn derive_secp_material(request: &ParsedRequest) -> Result<(PublicKey, [u8; 32]), String> {
    // Shared secp256k1 key derivation path:
    // BIP-39 seed -> BIP-32 child private key -> secp public key.
    let derivation_path = secp_derivation_path(request)?;
    let seed = derive_bip39_seed_from_request(request)?;
    let xpriv = derive_bip32_xpriv(
        seed.as_ref(),
        &derivation_path,
        request.hmac_key.as_deref(),
    )?;
    let secret_bytes = xpriv.private_key.secret_bytes();
    let secp = Secp256k1::new();
    let public_key = PublicKey::from_secret_key(&secp, &xpriv.private_key);
    Ok((public_key, secret_bytes))
}

fn derive_ed25519_material(request: &ParsedRequest) -> Result<([u8; 32], [u8; 32]), String> {
    // Dispatch on the derivation algorithm — ed25519 chains can consume the
    // same 32-byte private key from several distinct derivation pipelines.
    match request.derivation_algorithm {
        DerivationAlgorithm::Slip10Ed25519 => {
            let path = ed25519_derivation_path(request)?;
            let seed = derive_bip39_seed_from_request(request)?;
            let private_key = derive_slip10_ed25519_key(
                seed.as_ref(),
                &path,
                request.hmac_key.as_deref(),
            )?;
            let signing_key = SigningKey::from_bytes(&private_key);
            let public_key = signing_key.verifying_key().to_bytes();
            Ok((*private_key, public_key))
        }
        DerivationAlgorithm::DirectSeedEd25519 => {
            // NEAR Wallet / MyNearWallet convention: treat BIP-39 PBKDF2
            // seed[0..32] as the ed25519 private key directly. Path is
            // ignored; there is no BIP-32 walk.
            let seed = derive_bip39_seed_from_request(request)?;
            let mut private_key = [0u8; 32];
            private_key.copy_from_slice(&seed[..32]);
            let signing_key = SigningKey::from_bytes(&private_key);
            let public_key = signing_key.verifying_key().to_bytes();
            Ok((private_key, public_key))
        }
        DerivationAlgorithm::TonMnemonic => {
            let seed = derive_ton_seed(
                &request.seed_phrase,
                &request.passphrase,
                request.salt_prefix.as_deref(),
                request.iteration_count,
            )?;
            let mut private_key = [0u8; 32];
            private_key.copy_from_slice(&seed[..32]);
            let signing_key = SigningKey::from_bytes(&private_key);
            let public_key = signing_key.verifying_key().to_bytes();
            Ok((private_key, public_key))
        }
        DerivationAlgorithm::Bip32Ed25519Icarus => derive_cardano_icarus_material(request),
        DerivationAlgorithm::Auto
        | DerivationAlgorithm::Bip32Secp256k1
        | DerivationAlgorithm::SubstrateBip39
        | DerivationAlgorithm::MoneroBip39 => {
            Err("Derivation algorithm is not valid for ed25519 generic dispatch.".to_string())
        }
    }
}

fn derive_bitcoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let secp = Secp256k1::new();
    let derivation_path = secp_derivation_path(&request)?;
    let script_type = request.script_type;
    let seed = derive_bip39_seed_from_request(&request)?;
    let xpriv = derive_bip32_xpriv(
        seed.as_ref(),
        &derivation_path,
        request.hmac_key.as_deref(),
    )?;
    let secret_key = xpriv.private_key;
    let public_key = PublicKey::from_secret_key(&secp, &secret_key);

    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(derive_bitcoin_address(&request, script_type, &public_key, &secp)?)
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(secret_key.secret_bytes()))
        } else {
            None
        },
    })
}

fn derive_bitcoin_legacy_family(
    request: ParsedRequest,
    version: u8,
) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    let pubkey_hash = hash160_bytes(&public_key.serialize());
    let mut payload = vec![version];
    payload.extend_from_slice(&pubkey_hash);
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
    } else {
        None
    };

    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_litecoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    derive_bitcoin_legacy_family(request, 0x30)
}

fn derive_dogecoin(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let version = if matches!(request.network, NetworkFlavor::Testnet) {
        0x71
    } else {
        0x1e
    };
    derive_bitcoin_legacy_family(request, version)
}

fn derive_decred(request: ParsedRequest) -> Result<DerivedOutput, String> {
    use crate::derivation::chains::decred::{dcr_hash160, encode_dcr_p2pkh};
    let (public_key, private_key) = derive_secp_material(&request)?;
    let pubkey_hash = dcr_hash160(&public_key.serialize());
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(encode_dcr_p2pkh(&pubkey_hash))
    } else {
        None
    };
    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_kaspa(request: ParsedRequest) -> Result<DerivedOutput, String> {
    use crate::derivation::chains::kaspa::encode_kaspa_schnorr;
    let (public_key, private_key) = derive_secp_material(&request)?;
    // Kaspa Schnorr addresses use the 32-byte x-only public key.
    let serialized = public_key.serialize(); // 33-byte compressed
    let mut x_only = [0u8; 32];
    x_only.copy_from_slice(&serialized[1..33]);
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(encode_kaspa_schnorr(&x_only))
    } else {
        None
    };
    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_zcash_transparent(request: ParsedRequest) -> Result<DerivedOutput, String> {
    // Mainnet t1 P2PKH version is 0x1CB8 (two bytes); identical wire format to
    // Bitcoin P2PKH otherwise: version || hash160 || sha256d-checksum, base58.
    let (public_key, private_key) = derive_secp_material(&request)?;
    let pubkey_hash = hash160_bytes(&public_key.serialize());
    let mut payload = vec![0x1Cu8, 0xB8u8];
    payload.extend_from_slice(&pubkey_hash);
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
    } else {
        None
    };
    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_evm_family(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(derive_evm_address(&public_key))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_tron(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    let evm_address = derive_evm_address_bytes(&public_key);
    let mut payload = vec![0x41u8];
    payload.extend_from_slice(&evm_address);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_xrp(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (public_key, private_key) = derive_secp_material(&request)?;
    let pubkey_hash = hash160_bytes(&public_key.serialize());
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(&pubkey_hash);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(base58check_encode(&payload, bs58::Alphabet::RIPPLE))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(format_secp_public_key(
                &public_key,
                request.public_key_format,
            )?))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_solana(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(bs58::encode(public_key).into_string())
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_stellar(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    // StrKey for an ed25519 public key: version byte G (6<<3 = 0x30),
    // then 32-byte pubkey, then a 2-byte CRC16-XMODEM over the 33 bytes
    // above. Base32 (RFC 4648) encode the 35-byte blob → 56 chars.
    let mut payload = [0u8; 35];
    payload[0] = 0x30;
    payload[1..33].copy_from_slice(&public_key);
    let checksum = crc16_xmodem(&payload[..33]);
    payload[33] = (checksum & 0xff) as u8;
    payload[34] = (checksum >> 8) as u8;
    let stellar_address = base32_no_pad(&payload);

    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(stellar_address)
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_cardano(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let address = if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
        Some(derive_cardano_shelley_enterprise_address(
            &public_key,
            request.network,
        )?)
    } else {
        None
    };
    Ok(DerivedOutput {
        address,
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_sui(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&[0x00]);
    hasher.update(&public_key);
    hasher.finalize(&mut digest);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(format!("0x{}", hex::encode(digest)))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_aptos(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&public_key);
    hasher.update(&[0x00]);
    hasher.finalize(&mut digest);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(format!("0x{}", hex::encode(digest)))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_ton(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let address_algorithm = request.address_algorithm;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(format_ton_address(&public_key, address_algorithm)?)
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_icp(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    let mut data = Vec::from(public_key);
    data.extend_from_slice(b"icp");
    let digest = sha256_bytes(&data);
    let digest2 = sha256_bytes(&digest);
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(hex::encode(digest2))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_near(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_key, public_key) = derive_ed25519_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(private_key))
        } else {
            None
        },
    })
}

fn derive_polkadot(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (mini_secret, public_key) = derive_substrate_sr25519_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(encode_ss58(&public_key, 0))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(mini_secret))
        } else {
            None
        },
    })
}

fn derive_bittensor(request: ParsedRequest) -> Result<DerivedOutput, String> {
    // Bittensor uses the substrate-generic SS58 prefix (42); everything
    // else mirrors Polkadot — substrate-bip39 expansion + sr25519 keypair.
    let (mini_secret, public_key) = derive_substrate_sr25519_material(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(encode_ss58(&public_key, 42))
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            Some(hex::encode(public_key))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            Some(hex::encode(mini_secret))
        } else {
            None
        },
    })
}

fn derive_monero(request: ParsedRequest) -> Result<DerivedOutput, String> {
    let (private_spend, public_spend, _private_view, public_view) =
        derive_monero_keys_from_request(&request)?;
    Ok(DerivedOutput {
        address: if requests_output(request.requested_outputs, OUTPUT_ADDRESS) {
            Some(encode_monero_main_address(&public_spend, &public_view, request.network)?)
        } else {
            None
        },
        public_key_hex: if requests_output(request.requested_outputs, OUTPUT_PUBLIC_KEY) {
            // Concatenate spend + view as the canonical Monero "public address"
            // bytes — what wallets export when sharing a public viewing key.
            let mut both = [0u8; 64];
            both[..32].copy_from_slice(&public_spend);
            both[32..].copy_from_slice(&public_view);
            Some(hex::encode(both))
        } else {
            None
        },
        private_key_hex: if requests_output(request.requested_outputs, OUTPUT_PRIVATE_KEY) {
            // Private spend key is the canonical "secret" — the view key is
            // derivable from it via Keccak256.
            Some(hex::encode(private_spend))
        } else {
            None
        },
    })
}

fn derive_bip32_xpriv(
    seed_bytes: &[u8],
    derivation_path: &str,
    hmac_key: Option<&str>,
) -> Result<ExtendedPrivateKey, String> {
    // BIP-32 master derivation: I = HMAC-SHA512(Key, seed). The default key
    // is the spec's "Bitcoin seed", but callers may substitute any byte
    // string — cross-ecosystem wallets sometimes use different constants.
    let key_bytes = hmac_key
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes())
        .unwrap_or(b"Bitcoin seed");
    let master = ExtendedPrivateKey::master_from_seed(key_bytes, seed_bytes)?;
    let path = parse_bip32_path(derivation_path)?;
    let secp = Secp256k1::<All>::new();
    master.derive_path(&secp, &path)
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

fn derive_bitcoin_address(
    request: &ParsedRequest,
    script_type: ScriptType,
    public_key: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    derive_bitcoin_address_for_network(
        bitcoin_network_params(request.network),
        script_type,
        public_key,
        secp,
    )
}

fn derive_bitcoin_address_for_network(
    params: BitcoinNetworkParams,
    script_type: ScriptType,
    public_key: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    let compressed = public_key.serialize();
    match script_type {
        ScriptType::P2pkh => Ok(encode_p2pkh(&params, &compressed)),
        ScriptType::P2shP2wpkh => Ok(encode_p2sh_p2wpkh(&params, &compressed)),
        ScriptType::P2wpkh => encode_p2wpkh(&params, &compressed),
        ScriptType::P2tr => encode_p2tr(&params, secp, public_key),
        _ => Err("Unsupported Bitcoin script type.".to_string()),
    }
}

fn derive_bip39_seed(
    seed_phrase: &str,
    passphrase: &str,
    iteration_count: u32,
    mnemonic_wordlist: Option<&str>,
    salt_prefix: Option<&str>,
) -> Result<Zeroizing<[u8; 64]>, String> {
    // BIP-39 normalization + PBKDF2-HMAC-SHA512. All knobs are tunable:
    //   * `iteration_count == 0` selects the BIP-39 default of 2048 rounds.
    //   * `mnemonic_wordlist == None` selects English.
    //   * `salt_prefix == None` selects the BIP-39 default of "mnemonic".
    let language = resolve_bip39_language(mnemonic_wordlist)?;
    let mnemonic =
        Mnemonic::parse_in_normalized(language, seed_phrase).map_err(display_error)?;
    let iterations = if iteration_count == 0 {
        2048
    } else {
        iteration_count
    };
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_mnemonic = Zeroizing::new(mnemonic.to_string().nfkd().collect::<String>());
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let mut seed = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(
        normalized_mnemonic.as_bytes(),
        salt.as_bytes(),
        iterations,
        &mut *seed,
    );
    Ok(seed)
}

fn derive_bip39_seed_from_request(
    request: &ParsedRequest,
) -> Result<Zeroizing<[u8; 64]>, String> {
    derive_bip39_seed(
        &request.seed_phrase,
        &request.passphrase,
        request.iteration_count,
        request.mnemonic_wordlist.as_deref(),
        request.salt_prefix.as_deref(),
    )
}

fn derive_slip10_ed25519_key(
    seed: &[u8],
    derivation_path: &str,
    hmac_key: Option<&str>,
) -> Result<Zeroizing<[u8; 32]>, String> {
    // SLIP-0010 ed25519 derivation (hand-rolled so the HMAC master key is a
    // caller-supplied parameter instead of a hardcoded spec constant).
    let key_bytes = hmac_key
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes())
        .unwrap_or(b"ed25519 seed");

    // Master: I = HMAC-SHA512(Key, seed). IL = private_key, IR = chain_code.
    let master = hmac_sha512(key_bytes, &[seed])?;
    let mut private_key = Zeroizing::new([0u8; 32]);
    let mut chain_code = Zeroizing::new([0u8; 32]);
    private_key.copy_from_slice(&master[..32]);
    chain_code.copy_from_slice(&master[32..]);

    // Walk the path. SLIP-0010 for ed25519 only supports hardened children,
    // so any normal index is silently promoted — matches how every ed25519
    // wallet in the ecosystem interprets paths like `m/44'/501'/0'/0'`.
    for index in parse_slip10_ed25519_path(derivation_path)? {
        let index_bytes = index.to_be_bytes();
        let child = hmac_sha512(
            &*chain_code,
            &[&[0x00], &*private_key as &[u8], &index_bytes],
        )?;
        private_key.copy_from_slice(&child[..32]);
        chain_code.copy_from_slice(&child[32..]);
    }

    Ok(private_key)
}

fn derive_ton_seed(
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
    let iterations = if iteration_count == 0 {
        100_000
    } else {
        iteration_count
    };
    let salt = salt_prefix.unwrap_or("TON default seed");
    let mut seed = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(&*entropy, salt.as_bytes(), iterations, &mut *seed);
    Ok(seed)
}

fn derive_cardano_icarus_material(
    request: &ParsedRequest,
) -> Result<([u8; 32], [u8; 32]), String> {
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let root = derive_cardano_icarus_xprv_root(
        &request.seed_phrase,
        &request.passphrase,
        request.mnemonic_wordlist.as_deref(),
        request.iteration_count,
    )?;
    let path = request
        .derivation_path
        .clone()
        .unwrap_or_else(|| "m/1852'/1815'/0'/0/0".to_string());

    let mut xprv: Zeroizing<[u8; 96]> = Zeroizing::new([0u8; 96]);
    xprv.copy_from_slice(&*root);
    for index in parse_bip32_path_segments(&path)? {
        xprv = cardano_icarus_derive_child(&xprv, index)?;
    }

    let mut private_key = [0u8; 32];
    private_key.copy_from_slice(&xprv[0..32]);

    // Public key = kL * G on Ed25519. kL is already Khovratovich-Law clamped,
    // so reducing mod ℓ does not change the group element.
    let mut scalar_bytes = [0u8; 32];
    scalar_bytes.copy_from_slice(&private_key);
    let scalar = DalekScalar::from_bytes_mod_order(scalar_bytes);
    let point = scalar * ED25519_BASEPOINT_POINT;
    let public_key = point.compress().to_bytes();

    Ok((private_key, public_key))
}

fn derive_cardano_icarus_xprv_root(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 96]>, String> {
    // CIP-3 Icarus / CIP-1852 root:
    //   entropy = BIP-39 entropy decoded from the mnemonic (not the PBKDF2
    //             seed; Daedalus uses a different legacy scheme)
    //   xprv    = PBKDF2-HMAC-SHA512(password = passphrase,
    //                                 salt = entropy,
    //                                 iterations = 4096,
    //                                 dklen = 96)
    //   Then clamp per Khovratovich-Law so kL is a valid ed25519 scalar
    //   multiple of 8 and < 2^254.
    let language = resolve_bip39_language(wordlist)?;
    let parsed =
        Mnemonic::parse_in_normalized(language, mnemonic).map_err(display_error)?;
    let entropy = Zeroizing::new(parsed.to_entropy());
    let iterations = if iteration_count == 0 { 4096 } else { iteration_count };
    let mut xprv = Zeroizing::new([0u8; 96]);
    pbkdf2_hmac::<Sha512>(passphrase.as_bytes(), &entropy, iterations, &mut *xprv);
    xprv[0] &= 0b1111_1000;
    xprv[31] &= 0b0001_1111;
    xprv[31] |= 0b0100_0000;
    Ok(xprv)
}

fn cardano_icarus_derive_child(
    xprv: &[u8; 96],
    index: u32,
) -> Result<Zeroizing<[u8; 96]>, String> {
    // BIP-32-Ed25519 (Khovratovich-Law) child key derivation.
    //   xprv = kL (32) || kR (32) || chain_code (32)
    //   hardened (i >= 2^31):
    //     Z  = HMAC-SHA512(chain_code, 0x00 || kL || kR || i_LE)
    //     cc = HMAC-SHA512(chain_code, 0x01 || kL || kR || i_LE)[32..64]
    //   soft:
    //     A  = compressed(kL * G)   // ed25519 public point
    //     Z  = HMAC-SHA512(chain_code, 0x02 || A || i_LE)
    //     cc = HMAC-SHA512(chain_code, 0x03 || A || i_LE)[32..64]
    //   child_kL = parent_kL + 8 * ZL_28  (256-bit LE, overflow discarded)
    //   child_kR = parent_kR + ZR          (256-bit LE, overflow discarded)
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let kl = &xprv[0..32];
    let kr = &xprv[32..64];
    let cc = &xprv[64..96];
    let hardened = index >= 0x8000_0000;
    let i_le = index.to_le_bytes();

    let (z_tag, cc_tag): (u8, u8) = if hardened { (0x00, 0x01) } else { (0x02, 0x03) };

    let a_compressed = if hardened {
        [0u8; 32]
    } else {
        let mut scalar_bytes = [0u8; 32];
        scalar_bytes.copy_from_slice(kl);
        let scalar = DalekScalar::from_bytes_mod_order(scalar_bytes);
        (scalar * ED25519_BASEPOINT_POINT).compress().to_bytes()
    };

    let z = if hardened {
        hmac_sha512(cc, &[&[z_tag], kl, kr, &i_le])?
    } else {
        hmac_sha512(cc, &[&[z_tag], &a_compressed, &i_le])?
    };
    let child_cc_full = if hardened {
        hmac_sha512(cc, &[&[cc_tag], kl, kr, &i_le])?
    } else {
        hmac_sha512(cc, &[&[cc_tag], &a_compressed, &i_le])?
    };

    let zl_28 = &z[0..28];
    let zr = &z[32..64];

    // 8 * ZL_28 as a 32-byte little-endian integer.
    let mut eight_zl = [0u8; 32];
    let mut carry: u16 = 0;
    for (dst, &src) in eight_zl.iter_mut().zip(zl_28.iter()) {
        let v = (src as u16) * 8 + carry;
        *dst = (v & 0xff) as u8;
        carry = v >> 8;
    }
    if carry > 0 {
        eight_zl[28] = carry as u8;
    }

    let mut child_xprv = Zeroizing::new([0u8; 96]);
    let mut carry: u16 = 0;
    for i in 0..32 {
        let v = (kl[i] as u16) + (eight_zl[i] as u16) + carry;
        child_xprv[i] = (v & 0xff) as u8;
        carry = v >> 8;
    }
    // Discard final carry — BIP-32-Ed25519 treats the addition as 256-bit.
    let mut carry: u16 = 0;
    for i in 0..32 {
        let v = (kr[i] as u16) + (zr[i] as u16) + carry;
        child_xprv[32 + i] = (v & 0xff) as u8;
        carry = v >> 8;
    }
    child_xprv[64..96].copy_from_slice(&child_cc_full[32..64]);
    Ok(child_xprv)
}

fn derive_cardano_shelley_enterprise_address(
    public_key: &[u8; 32],
    network: NetworkFlavor,
) -> Result<String, String> {
    // CIP-19 Shelley address: header_byte || payment_credential.
    //   header: upper 4 bits = address type (0b0110 = enterprise, payment
    //           credential is a key hash); lower 4 bits = network id
    //           (0 = testnet, 1 = mainnet).
    //   payment_credential: Blake2b-224 of the ed25519 public key.
    //   Encoding: bech32 under HRP "addr" (mainnet) / "addr_test" (testnet).
    use blake2::digest::consts::U28;
    use blake2::digest::Digest;
    use blake2::Blake2b;
    type Blake2b224 = Blake2b<U28>;

    let mut hasher = Blake2b224::new();
    hasher.update(public_key);
    let payment_hash = hasher.finalize();

    let network_id: u8 = match network {
        NetworkFlavor::Mainnet => 1,
        NetworkFlavor::Testnet | NetworkFlavor::Testnet4 | NetworkFlavor::Signet => 0,
    };
    let header = 0x60 | network_id;

    let mut payload = Vec::with_capacity(29);
    payload.push(header);
    payload.extend_from_slice(&payment_hash);

    let hrp_str = if matches!(network, NetworkFlavor::Mainnet) {
        "addr"
    } else {
        "addr_test"
    };
    let hrp = bech32::Hrp::parse(hrp_str).map_err(display_error)?;
    bech32::encode::<bech32::Bech32>(hrp, &payload).map_err(display_error)
}

fn derive_substrate_sr25519_material(
    request: &ParsedRequest,
) -> Result<([u8; 32], [u8; 32]), String> {
    // Substrate //hard and /soft junctions are not yet supported. Polkadot.js
    // / subkey defaults to the root mini-secret with no path, so requests
    // with no derivation path (or just "m") map onto that default.
    let path = request.derivation_path.as_deref().unwrap_or("").trim();
    if !path.is_empty() && path != "m" && path != "M" {
        return Err(
            "Substrate junction derivation (//hard, /soft) is not yet supported; \
             omit the derivation path to derive the root sr25519 keypair."
                .to_string(),
        );
    }

    let mini_secret = derive_substrate_mini_secret(
        &request.seed_phrase,
        &request.passphrase,
        request.mnemonic_wordlist.as_deref(),
        request.salt_prefix.as_deref(),
        request.iteration_count,
    )?;

    let mini = schnorrkel::MiniSecretKey::from_bytes(&*mini_secret)
        .map_err(|e| format!("Invalid sr25519 mini-secret: {e}"))?;
    let keypair = mini.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let public_key = keypair.public.to_bytes();

    let mut mini_out = [0u8; 32];
    mini_out.copy_from_slice(&*mini_secret);
    Ok((mini_out, public_key))
}

fn derive_substrate_mini_secret(
    mnemonic: &str,
    passphrase: &str,
    wordlist: Option<&str>,
    salt_prefix: Option<&str>,
    iteration_count: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    // substrate-bip39: mini-secret = PBKDF2-HMAC-SHA512(
    //   password = BIP-39 entropy bytes (NOT the mnemonic string),
    //   salt     = "mnemonic" || passphrase,
    //   iter     = 2048,
    //   dklen    = 64
    // )[0..32].
    // This is the scheme polkadot-js, sr25519 in subkey, and most Substrate
    // wallets use to obtain the root mini-secret from a BIP-39 mnemonic.
    let language = resolve_bip39_language(wordlist)?;
    let parsed =
        Mnemonic::parse_in_normalized(language, mnemonic).map_err(display_error)?;
    let entropy = Zeroizing::new(parsed.to_entropy());
    let prefix = salt_prefix.unwrap_or("mnemonic");
    let normalized_passphrase = Zeroizing::new(passphrase.nfkd().collect::<String>());
    let normalized_prefix = Zeroizing::new(prefix.nfkd().collect::<String>());
    let salt = Zeroizing::new(format!(
        "{}{}",
        normalized_prefix.as_str(),
        normalized_passphrase.as_str()
    ));
    let iterations = if iteration_count == 0 { 2048 } else { iteration_count };
    let mut buf = Zeroizing::new([0u8; 64]);
    pbkdf2_hmac::<Sha512>(&entropy, salt.as_bytes(), iterations, &mut *buf);
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&buf[..32]);
    Ok(out)
}

fn encode_ss58(public_key: &[u8; 32], network_prefix: u16) -> String {
    // SS58 v1: prefix_bytes || pubkey || blake2b_512("SS58PRE" || prefix || pubkey)[0..2],
    // base58-encoded. Single-byte prefix for ids < 64; 2-byte form covers ids up to 16383.
    use blake2::digest::consts::U64;
    use blake2::digest::Digest;
    use blake2::Blake2b;
    type Blake2b512 = Blake2b<U64>;

    let prefix_bytes: Vec<u8> = if network_prefix < 64 {
        vec![network_prefix as u8]
    } else {
        // 14-bit prefix packed into two bytes per the SS58 spec.
        let lower = (network_prefix & 0b0000_0000_1111_1111) as u8;
        let upper = ((network_prefix & 0b0011_1111_0000_0000) >> 8) as u8;
        let first = ((lower & 0b1111_1100) >> 2) | ((upper & 0b0000_0011) << 6);
        let second = (lower & 0b0000_0011) | (upper & 0b1111_1100) | 0b0100_0000;
        vec![first | 0b0100_0000, second]
    };

    let mut payload = Vec::with_capacity(prefix_bytes.len() + 32 + 2);
    payload.extend_from_slice(&prefix_bytes);
    payload.extend_from_slice(public_key);

    let mut hasher = Blake2b512::new();
    hasher.update(b"SS58PRE");
    hasher.update(&payload);
    let checksum = hasher.finalize();
    payload.extend_from_slice(&checksum[..2]);

    bs58::encode(payload).into_string()
}

fn derive_monero_keys_from_request(
    request: &ParsedRequest,
) -> Result<([u8; 32], [u8; 32], [u8; 32], [u8; 32]), String> {
    let seed = derive_bip39_seed_from_request(request)?;
    let mut spend_seed = [0u8; 32];
    spend_seed.copy_from_slice(&seed[..32]);
    derive_monero_keys_from_spend_seed(&spend_seed)
}

fn derive_monero_keys_from_spend_seed(
    spend_seed: &[u8; 32],
) -> Result<([u8; 32], [u8; 32], [u8; 32], [u8; 32]), String> {
    // Monero derivation:
    //   private_spend = sc_reduce32(spend_seed)
    //   private_view  = sc_reduce32(Keccak256(private_spend))
    //   public_*      = scalar * G  (ed25519 basepoint)
    use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
    use curve25519_dalek::scalar::Scalar as DalekScalar;

    let private_spend = DalekScalar::from_bytes_mod_order(*spend_seed).to_bytes();

    let mut hasher = Keccak::v256();
    let mut spend_hash = [0u8; 32];
    hasher.update(&private_spend);
    hasher.finalize(&mut spend_hash);
    let private_view = DalekScalar::from_bytes_mod_order(spend_hash).to_bytes();

    let public_spend = (DalekScalar::from_bytes_mod_order(private_spend)
        * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();
    let public_view = (DalekScalar::from_bytes_mod_order(private_view)
        * ED25519_BASEPOINT_POINT)
        .compress()
        .to_bytes();

    Ok((private_spend, public_spend, private_view, public_view))
}

fn encode_monero_main_address(
    public_spend: &[u8; 32],
    public_view: &[u8; 32],
    network: NetworkFlavor,
) -> Result<String, String> {
    // Monero standard address: network_byte || public_spend (32) ||
    // public_view (32) || keccak256(prev)[0..4]. 69 bytes total → 95 chars
    // in the chunked Base58 encoding.
    let network_byte: u8 = match network {
        NetworkFlavor::Mainnet => 0x12,
        NetworkFlavor::Testnet => 0x35,
        NetworkFlavor::Testnet4 | NetworkFlavor::Signet => {
            return Err("Monero only supports Mainnet and Testnet networks.".to_string())
        }
    };
    let mut payload = Vec::with_capacity(69);
    payload.push(network_byte);
    payload.extend_from_slice(public_spend);
    payload.extend_from_slice(public_view);
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&payload);
    hasher.finalize(&mut digest);
    payload.extend_from_slice(&digest[..4]);
    Ok(monero_base58_encode(&payload))
}

fn monero_base58_encode(data: &[u8]) -> String {
    // Monero's chunked Base58: split into 8-byte blocks, each block encodes
    // as a fixed-width 11 chars (lookup table for partial trailing blocks).
    // Alphabet differs in ordering from BIP-58's standard alphabet but uses
    // the same 58 characters.
    const ALPHABET: &[u8; 58] =
        b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    const FULL_BLOCK_SIZE: usize = 8;
    const FULL_ENCODED_BLOCK_SIZE: usize = 11;
    const ENCODED_BLOCK_SIZES: [usize; FULL_BLOCK_SIZE + 1] = [0, 2, 3, 5, 6, 7, 9, 10, 11];

    let mut out = String::new();
    let full_blocks = data.len() / FULL_BLOCK_SIZE;
    let remainder = data.len() % FULL_BLOCK_SIZE;

    for i in 0..full_blocks {
        let start = i * FULL_BLOCK_SIZE;
        let block = &data[start..start + FULL_BLOCK_SIZE];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let mut chars = [b'1'; FULL_ENCODED_BLOCK_SIZE];
        for j in (0..FULL_ENCODED_BLOCK_SIZE).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    if remainder > 0 {
        let block = &data[full_blocks * FULL_BLOCK_SIZE..];
        let mut value: u64 = 0;
        for &b in block {
            value = (value << 8) | u64::from(b);
        }
        let encoded_len = ENCODED_BLOCK_SIZES[remainder];
        let mut chars = vec![b'1'; encoded_len];
        for j in (0..encoded_len).rev() {
            chars[j] = ALPHABET[(value % 58) as usize];
            value /= 58;
        }
        out.push_str(std::str::from_utf8(&chars).unwrap());
    }
    out
}

// ---------------------------------------------------------------------------
// TON v4R2 wallet address derivation
// ---------------------------------------------------------------------------
//
// The user-friendly bounceable address format for a TON wallet is:
//     tag(1) || workchain(1) || account_id(32) || crc16_xmodem(2)
// encoded as base64url without padding. `account_id` is the SHA-256 cell hash
// of the wallet's `state_init`, which is a cell carrying references to the
// wallet code cell and a freshly-initialized data cell. See the TLB schema in
// `crypto/block/block.tlb` under `StateInit`.
//
// The v4R2 wallet code BOC bytes below were fetched from toncenter/tonweb's
// `WalletContractV4R2.js`. Correctness is locked by a self-test that asserts
// the recomputed root cell hash matches the well-known public constant
// `feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0`.

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

// Default subwallet id for v4 on the basic workchain (0). Hardcoded in every
// popular wallet (tonkeeper, tonhub, tonweb) so anyone generating a v4R2
// address from the same mnemonic produces the same address.
const V4R2_DEFAULT_WALLET_ID: u32 = 698983191;

#[derive(Clone)]
struct ParsedCell {
    d1: u8,
    d2: u8,
    data: Vec<u8>,
    refs: Vec<usize>,
}

/// Minimal parser for BOC v0 (`b5ee9c72`) carrying ordinary (non-exotic,
/// level-0) cells. Supports the index and crc32c flags but validates neither;
/// the parser's correctness is instead locked by a cell-hash self-test.
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

/// Returns (code_hash, code_depth) for the embedded v4R2 wallet code cell,
/// computed once per process.
fn v4r2_code_hash_and_depth() -> Result<([u8; 32], u16), String> {
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

/// Build the v4R2 data cell (321 bits, no refs) carrying the initial seqno,
/// subwallet id, public key, and empty plugin dict, and return its cell hash
/// and depth.
fn v4r2_data_cell_hash(public_key: &[u8; 32]) -> ([u8; 32], u16) {
    // Layout: seqno(32) || wallet_id(32) || pubkey(256) || plugins?(1) = 321 bits.
    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&V4R2_DEFAULT_WALLET_ID.to_be_bytes());
    data.extend_from_slice(public_key);
    // Plugins-dict-present bit = 0 lives in bit 7 of the last byte.
    // Completion bit = 1 in bit 6, remaining bits = 0 → 0x40.
    data.push(0x40);
    let mut repr = Vec::with_capacity(2 + data.len());
    repr.push(0x00); // d1: 0 refs, not exotic, level 0
    repr.push(81); // d2: floor(321/8)+ceil(321/8) = 40+41 = 81
    repr.extend_from_slice(&data);
    let hash = sha256_bytes(&repr);
    (hash, 0)
}

/// Build the state_init cell for v4R2 (5 header bits + 2 refs: code, data)
/// and return its cell hash — which is the TON account id.
fn v4r2_state_init_account_id(public_key: &[u8; 32]) -> Result<[u8; 32], String> {
    let (code_hash, code_depth) = v4r2_code_hash_and_depth()?;
    let (data_hash, data_depth) = v4r2_data_cell_hash(public_key);
    // 5-bit header: split_depth?=0, special?=0, code?=1, data?=1, library?=0
    // Padded: 00110 || 1 (completion) || 00 = 0b0011_0100 = 0x34.
    let header_byte: u8 = 0x34;
    let mut repr = Vec::with_capacity(2 + 1 + 2 * (2 + 32));
    repr.push(0x02); // d1: 2 refs
    repr.push(0x01); // d2: bits=5 → floor(5/8)+ceil(5/8) = 0+1 = 1
    repr.push(header_byte);
    repr.extend_from_slice(&code_depth.to_be_bytes());
    repr.extend_from_slice(&data_depth.to_be_bytes());
    repr.extend_from_slice(&code_hash);
    repr.extend_from_slice(&data_hash);
    Ok(sha256_bytes(&repr))
}

/// CRC-16/XMODEM (poly=0x1021, init=0x0000, no reflection, no xor-out),
/// as required by TON user-friendly address checksums.
fn crc16_xmodem(bytes: &[u8]) -> u16 {
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

fn format_ton_address(
    public_key: &[u8; 32],
    algorithm: AddressAlgorithm,
) -> Result<String, String> {
    match algorithm {
        AddressAlgorithm::TonV4R2 => derive_ton_v4r2_address(public_key),
        AddressAlgorithm::TonRawAccountId | AddressAlgorithm::Auto => {
            Ok(format!("0:{}", hex::encode(public_key)))
        }
        _ => Err("Unsupported address algorithm for TON.".to_string()),
    }
}

fn parse_bip32_path_segments(path: &str) -> Result<Vec<u32>, String> {
    // Accept BIP-32-style path strings; preserves the hardened flag from
    // each segment rather than force-hardening (unlike SLIP-0010 ed25519).
    let trimmed = path.trim();
    let body = trimmed
        .strip_prefix("m/")
        .or_else(|| trimmed.strip_prefix("M/"))
        .unwrap_or_else(|| {
            if trimmed == "m" || trimmed == "M" {
                ""
            } else {
                trimmed
            }
        });
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for segment in body.split('/') {
        let seg = segment.trim();
        let (digits, hardened) = if let Some(s) = seg.strip_suffix('\'') {
            (s, true)
        } else if let Some(s) = seg.strip_suffix('h') {
            (s, true)
        } else {
            (seg, false)
        };
        let raw: u32 = digits
            .parse()
            .map_err(|_| format!("Invalid derivation path segment: {segment}"))?;
        if raw & 0x8000_0000 != 0 {
            return Err(format!("Derivation path segment out of range: {segment}"));
        }
        out.push(if hardened { raw | 0x8000_0000 } else { raw });
    }
    Ok(out)
}

fn parse_slip10_ed25519_path(path: &str) -> Result<Vec<u32>, String> {
    // Accept BIP-32-style path strings (`m/44'/501'/0'/0'`) and coerce every
    // index to hardened, which is the only form SLIP-0010 ed25519 supports.
    let trimmed = path.trim();
    let body = trimmed
        .strip_prefix("m/")
        .or_else(|| trimmed.strip_prefix("M/"))
        .unwrap_or_else(|| {
            if trimmed == "m" || trimmed == "M" {
                ""
            } else {
                trimmed
            }
        });
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut indices = Vec::new();
    for segment in body.split('/') {
        let cleaned = segment.trim_end_matches('\'').trim_end_matches('h');
        let raw: u32 = cleaned
            .parse()
            .map_err(|_| format!("Invalid derivation path segment: {segment}"))?;
        if raw & 0x8000_0000 != 0 {
            return Err(format!(
                "Derivation path segment out of range: {segment}"
            ));
        }
        indices.push(raw | 0x8000_0000);
    }
    Ok(indices)
}

fn resolve_bip39_language(name: Option<&str>) -> Result<Language, String> {
    let value = match name {
        Some(value) if !value.trim().is_empty() => value.trim().to_ascii_lowercase(),
        _ => return Ok(Language::English),
    };
    match value.as_str() {
        "english" | "en" => Ok(Language::English),
        "czech" | "cs" => Ok(Language::Czech),
        "french" | "fr" => Ok(Language::French),
        "italian" | "it" => Ok(Language::Italian),
        "japanese" | "ja" | "jp" => Ok(Language::Japanese),
        "korean" | "ko" | "kr" => Ok(Language::Korean),
        "portuguese" | "pt" => Ok(Language::Portuguese),
        "spanish" | "es" => Ok(Language::Spanish),
        "simplified-chinese"
        | "chinese-simplified"
        | "simplified_chinese"
        | "zh-hans"
        | "zh-cn"
        | "zh" => Ok(Language::SimplifiedChinese),
        "traditional-chinese"
        | "chinese-traditional"
        | "traditional_chinese"
        | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

fn secp_derivation_path(request: &ParsedRequest) -> Result<String, String> {
    request.derivation_path.clone().ok_or_else(|| {
        "Derivation path is required. Provide a preset or custom derivation path from Swift."
            .to_string()
    })
}

fn ed25519_derivation_path(request: &ParsedRequest) -> Result<String, String> {
    request.derivation_path.clone().ok_or_else(|| {
        "Derivation path is required. Provide a preset or custom derivation path from Swift."
            .to_string()
    })
}

fn derive_evm_address(public_key: &PublicKey) -> String {
    format!("0x{}", hex::encode(derive_evm_address_bytes(public_key)))
}

fn derive_evm_address_bytes(public_key: &PublicKey) -> [u8; 20] {
    let uncompressed = public_key.serialize_uncompressed();
    let mut hasher = Keccak::v256();
    let mut digest = [0u8; 32];
    hasher.update(&uncompressed[1..]);
    hasher.finalize(&mut digest);
    let mut out = [0u8; 20];
    out.copy_from_slice(&digest[12..]);
    out
}

fn format_secp_public_key(
    public_key: &PublicKey,
    format: PublicKeyFormat,
) -> Result<Vec<u8>, String> {
    Ok(match format {
        PublicKeyFormat::Compressed => public_key.serialize().to_vec(),
        PublicKeyFormat::Uncompressed => public_key.serialize_uncompressed().to_vec(),
        PublicKeyFormat::XOnly => public_key.x_only_public_key().0.serialize().to_vec(),
        PublicKeyFormat::Raw => public_key.serialize().to_vec(),
        PublicKeyFormat::Auto => {
            return Err("Public key format must be explicit.".to_string());
        }
    })
}

fn base58check_encode(payload: &[u8], alphabet: &bs58::Alphabet) -> String {
    bs58::encode(payload)
        .with_alphabet(alphabet)
        .with_check()
        .into_string()
}

fn base32_no_pad(input: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(input)
}

fn normalize_seed_phrase(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn requests_output(requested_outputs: u32, output: u32) -> bool {
    requested_outputs & output != 0
}

fn display_error(error: impl Display) -> String {
    error.to_string()
}


#[cfg(test)]
mod tests;
