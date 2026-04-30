//! Derivation engine: FFI surface, wire-format types, dispatch table, and
//! per-chain `derive_*` orchestration. This is the "everything that isn't
//! a chain validator/encoder, a primitive, or a curve helper" file.
//!
//! Architecture:
//!   - `chains/` owns per-chain encoders + validators (the leaf code).
//!   - `primitives/` owns shared crypto (BIP-32, BIP-39, SLIP-10, HMAC).
//!   - `curves/` owns curve-level material derivation.
//!   - This file owns the request/response shape, wire-format parsing,
//!     dispatch by chain, and the FFI export functions Swift calls.

use std::fmt::Display;

use bip39::Language;
use ed25519_dalek::SigningKey;
use secp256k1::{All, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};
use zeroize::Zeroize;

use super::chains::bitcoin::{
    encode_p2pkh, encode_p2sh_p2wpkh, encode_p2tr, encode_p2wpkh, hash160 as hash160_bytes,
    sha256 as sha256_bytes, BitcoinNetworkParams, BTC_MAINNET, BTC_TESTNET,
};
use super::chains::cardano::derive_cardano_shelley_enterprise_address;
use super::chains::monero::{derive_monero_keys_from_spend_seed, encode_monero_main_address};
use super::chains::polkadot::encode_ss58;
use super::chains::ton::format_ton_address;

// ── Inlined helpers (formerly in primitives/ and curves/) ────────────────

pub(crate) fn base58check_encode(payload: &[u8], alphabet: &bs58::Alphabet) -> String {
    bs58::encode(payload)
        .with_alphabet(alphabet)
        .with_check()
        .into_string()
}

pub(crate) fn base32_no_pad(input: &[u8]) -> String {
    data_encoding::BASE32_NOPAD.encode(input)
}

pub(crate) fn normalize_seed_phrase(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn display_error(error: impl Display) -> String {
    error.to_string()
}

pub(crate) fn resolve_bip39_language(name: Option<&str>) -> Result<Language, String> {
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
        "simplified-chinese" | "chinese-simplified" | "simplified_chinese" | "zh-hans"
        | "zh-cn" | "zh" => Ok(Language::SimplifiedChinese),
        "traditional-chinese" | "chinese-traditional" | "traditional_chinese" | "zh-hant"
        | "zh-tw" => Ok(Language::TraditionalChinese),
        other => Err(format!("Unsupported mnemonic wordlist: {other}")),
    }
}

/// Parse the BIP-32 purpose level from a derivation path and return the
/// matching Bitcoin script type constant. Defaults to P2PKH when the
/// purpose is unknown. Examples: `m/44'/…` → P2PKH, `m/49'/…` → P2SH-P2WPKH,
/// `m/84'/…` → P2WPKH, `m/86'/…` → P2TR.
pub(crate) fn script_type_from_purpose(path: &str) -> u32 {
    let without_prefix = path
        .trim_start_matches('m')
        .trim_start_matches('M')
        .trim_start_matches('/');
    let purpose_segment = without_prefix.split('/').next().unwrap_or("");
    let purpose_str = purpose_segment.trim_end_matches('\'').trim_end_matches('h');
    match purpose_str {
        "44" => SCRIPT_P2PKH,
        "49" => SCRIPT_P2SH_P2WPKH,
        "84" => SCRIPT_P2WPKH,
        "86" => SCRIPT_P2TR,
        _ => SCRIPT_P2PKH,
    }
}

pub(crate) fn format_secp_public_key(
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


// ── from former wire.rs ─────────────────────────────────────────


// Bitflags describing which output fields the caller wants back.
pub(crate) const OUTPUT_ADDRESS: u32 = 1 << 0;
pub(crate) const OUTPUT_PUBLIC_KEY: u32 = 1 << 1;
pub(crate) const OUTPUT_PRIVATE_KEY: u32 = 1 << 2;

// ── Chain wire IDs ───────────────────────────────────────────────────────
// Mainnets (0–28).
pub(crate) const CHAIN_BITCOIN: u32 = 0;
pub(crate) const CHAIN_ETHEREUM: u32 = 1;
pub(crate) const CHAIN_SOLANA: u32 = 2;
pub(crate) const CHAIN_BITCOIN_CASH: u32 = 3;
pub(crate) const CHAIN_BITCOIN_SV: u32 = 4;
pub(crate) const CHAIN_LITECOIN: u32 = 5;
pub(crate) const CHAIN_DOGECOIN: u32 = 6;
pub(crate) const CHAIN_ETHEREUM_CLASSIC: u32 = 7;
pub(crate) const CHAIN_ARBITRUM: u32 = 8;
pub(crate) const CHAIN_OPTIMISM: u32 = 9;
pub(crate) const CHAIN_AVALANCHE: u32 = 10;
pub(crate) const CHAIN_HYPERLIQUID: u32 = 11;
pub(crate) const CHAIN_TRON: u32 = 12;
pub(crate) const CHAIN_STELLAR: u32 = 13;
pub(crate) const CHAIN_XRP: u32 = 14;
pub(crate) const CHAIN_CARDANO: u32 = 15;
pub(crate) const CHAIN_SUI: u32 = 16;
pub(crate) const CHAIN_APTOS: u32 = 17;
pub(crate) const CHAIN_TON: u32 = 18;
pub(crate) const CHAIN_INTERNET_COMPUTER: u32 = 19;
pub(crate) const CHAIN_NEAR: u32 = 20;
pub(crate) const CHAIN_POLKADOT: u32 = 21;
pub(crate) const CHAIN_MONERO: u32 = 22;
pub(crate) const CHAIN_ZCASH: u32 = 23;
pub(crate) const CHAIN_BITCOIN_GOLD: u32 = 24;
pub(crate) const CHAIN_DECRED: u32 = 25;
pub(crate) const CHAIN_KASPA: u32 = 26;
pub(crate) const CHAIN_DASH: u32 = 27;
pub(crate) const CHAIN_BITTENSOR: u32 = 28;

// Testnets (46–77) — match registry::Chain layout.
pub(crate) const CHAIN_BITCOIN_TESTNET: u32 = 46;
pub(crate) const CHAIN_BITCOIN_TESTNET4: u32 = 47;
pub(crate) const CHAIN_BITCOIN_SIGNET: u32 = 48;
pub(crate) const CHAIN_LITECOIN_TESTNET: u32 = 49;
pub(crate) const CHAIN_BITCOIN_CASH_TESTNET: u32 = 50;
pub(crate) const CHAIN_BITCOIN_SV_TESTNET: u32 = 51;
pub(crate) const CHAIN_DOGECOIN_TESTNET: u32 = 52;
pub(crate) const CHAIN_ZCASH_TESTNET: u32 = 53;
pub(crate) const CHAIN_DECRED_TESTNET: u32 = 54;
pub(crate) const CHAIN_KASPA_TESTNET: u32 = 55;
pub(crate) const CHAIN_DASH_TESTNET: u32 = 56;
pub(crate) const CHAIN_ETHEREUM_SEPOLIA: u32 = 57;
pub(crate) const CHAIN_ETHEREUM_HOODI: u32 = 58;
pub(crate) const CHAIN_ARBITRUM_SEPOLIA: u32 = 59;
pub(crate) const CHAIN_OPTIMISM_SEPOLIA: u32 = 60;
pub(crate) const CHAIN_BASE_SEPOLIA: u32 = 61;
pub(crate) const CHAIN_BNB_TESTNET: u32 = 62;
pub(crate) const CHAIN_AVALANCHE_FUJI: u32 = 63;
pub(crate) const CHAIN_POLYGON_AMOY: u32 = 64;
pub(crate) const CHAIN_HYPERLIQUID_TESTNET: u32 = 65;
pub(crate) const CHAIN_ETHEREUM_CLASSIC_MORDOR: u32 = 66;
pub(crate) const CHAIN_TRON_NILE: u32 = 67;
pub(crate) const CHAIN_SOLANA_DEVNET: u32 = 68;
pub(crate) const CHAIN_XRP_TESTNET: u32 = 69;
pub(crate) const CHAIN_STELLAR_TESTNET: u32 = 70;
pub(crate) const CHAIN_CARDANO_PREPROD: u32 = 71;
pub(crate) const CHAIN_SUI_TESTNET: u32 = 72;
pub(crate) const CHAIN_APTOS_TESTNET: u32 = 73;
pub(crate) const CHAIN_TON_TESTNET: u32 = 74;
pub(crate) const CHAIN_NEAR_TESTNET: u32 = 75;
pub(crate) const CHAIN_POLKADOT_WESTEND: u32 = 76;
pub(crate) const CHAIN_MONERO_STAGENET: u32 = 77;

// ── Curve wire IDs ───────────────────────────────────────────────────────
pub(crate) const CURVE_SECP256K1: u32 = 0;
pub(crate) const CURVE_ED25519: u32 = 1;
pub(crate) const CURVE_SR25519: u32 = 2;

// ── Derivation algorithm wire IDs ────────────────────────────────────────
pub(crate) const DERIVATION_AUTO: u32 = 0;
pub(crate) const DERIVATION_BIP32_SECP256K1: u32 = 1;
pub(crate) const DERIVATION_SLIP10_ED25519: u32 = 2;
pub(crate) const DERIVATION_DIRECT_SEED_ED25519: u32 = 3;
pub(crate) const DERIVATION_TON_MNEMONIC: u32 = 4;
pub(crate) const DERIVATION_BIP32_ED25519_ICARUS: u32 = 5;
pub(crate) const DERIVATION_SUBSTRATE_BIP39: u32 = 6;
pub(crate) const DERIVATION_MONERO_BIP39: u32 = 7;

// ── Address algorithm wire IDs ───────────────────────────────────────────
pub(crate) const ADDRESS_AUTO: u32 = 0;
pub(crate) const ADDRESS_BITCOIN: u32 = 1;
pub(crate) const ADDRESS_EVM: u32 = 2;
pub(crate) const ADDRESS_SOLANA: u32 = 3;
pub(crate) const ADDRESS_NEAR_HEX: u32 = 4;
pub(crate) const ADDRESS_TON_RAW_ACCOUNT_ID: u32 = 5;
pub(crate) const ADDRESS_CARDANO_SHELLEY_ENTERPRISE: u32 = 6;
pub(crate) const ADDRESS_SS58: u32 = 7;
pub(crate) const ADDRESS_MONERO_MAIN: u32 = 8;
pub(crate) const ADDRESS_TON_V4R2: u32 = 9;
pub(crate) const ADDRESS_LITECOIN: u32 = 10;
pub(crate) const ADDRESS_DOGECOIN: u32 = 11;
pub(crate) const ADDRESS_BITCOIN_CASH_LEGACY: u32 = 12;
pub(crate) const ADDRESS_BITCOIN_SV_LEGACY: u32 = 13;
pub(crate) const ADDRESS_TRON_BASE58_CHECK: u32 = 14;
pub(crate) const ADDRESS_XRP_BASE58_CHECK: u32 = 15;
pub(crate) const ADDRESS_STELLAR_STRKEY: u32 = 16;
pub(crate) const ADDRESS_SUI_KECCAK: u32 = 17;
pub(crate) const ADDRESS_APTOS_KECCAK: u32 = 18;
pub(crate) const ADDRESS_ICP_PRINCIPAL: u32 = 19;
pub(crate) const ADDRESS_ZCASH_TRANSPARENT: u32 = 20;
pub(crate) const ADDRESS_BITCOIN_GOLD_LEGACY: u32 = 21;
pub(crate) const ADDRESS_DECRED_P2PKH: u32 = 22;
pub(crate) const ADDRESS_KASPA_SCHNORR: u32 = 23;
pub(crate) const ADDRESS_DASH_LEGACY: u32 = 24;
pub(crate) const ADDRESS_BITTENSOR_SS58: u32 = 25;

// ── Public key format wire IDs ───────────────────────────────────────────
pub(crate) const PUBLIC_KEY_AUTO: u32 = 0;
pub(crate) const PUBLIC_KEY_COMPRESSED: u32 = 1;
pub(crate) const PUBLIC_KEY_UNCOMPRESSED: u32 = 2;
pub(crate) const PUBLIC_KEY_X_ONLY: u32 = 3;
pub(crate) const PUBLIC_KEY_RAW: u32 = 4;

// ── Script type wire IDs ─────────────────────────────────────────────────
pub(crate) const SCRIPT_AUTO: u32 = 0;
pub(crate) const SCRIPT_P2PKH: u32 = 1;
pub(crate) const SCRIPT_P2SH_P2WPKH: u32 = 2;
pub(crate) const SCRIPT_P2WPKH: u32 = 3;
pub(crate) const SCRIPT_P2TR: u32 = 4;
pub(crate) const SCRIPT_ACCOUNT: u32 = 5;

// ── Wire → enum parsers ──────────────────────────────────────────────────

/// Infer the internal `Chain` dispatch tag from the address algorithm.
/// Used when callers omit the `chain` field on a derivation request:
/// every supported `AddressAlgorithm` variant is 1:1 with a concrete chain
/// (or, for EVM, any chain in the EVM family — derivation output is
/// identical across them, so we pick Ethereum as the canonical tag).
pub(crate) fn chain_from_address_algorithm(alg: AddressAlgorithm) -> Result<Chain, String> {
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

pub(crate) fn parse_chain(value: u32) -> Result<Chain, String> {
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
        CHAIN_BITCOIN_TESTNET => Ok(Chain::BitcoinTestnet),
        CHAIN_BITCOIN_TESTNET4 => Ok(Chain::BitcoinTestnet4),
        CHAIN_BITCOIN_SIGNET => Ok(Chain::BitcoinSignet),
        CHAIN_LITECOIN_TESTNET => Ok(Chain::LitecoinTestnet),
        CHAIN_BITCOIN_CASH_TESTNET => Ok(Chain::BitcoinCashTestnet),
        CHAIN_BITCOIN_SV_TESTNET => Ok(Chain::BitcoinSvTestnet),
        CHAIN_DOGECOIN_TESTNET => Ok(Chain::DogecoinTestnet),
        CHAIN_ZCASH_TESTNET => Ok(Chain::ZcashTestnet),
        CHAIN_DECRED_TESTNET => Ok(Chain::DecredTestnet),
        CHAIN_KASPA_TESTNET => Ok(Chain::KaspaTestnet),
        CHAIN_DASH_TESTNET => Ok(Chain::DashTestnet),
        CHAIN_ETHEREUM_SEPOLIA => Ok(Chain::EthereumSepolia),
        CHAIN_ETHEREUM_HOODI => Ok(Chain::EthereumHoodi),
        CHAIN_ARBITRUM_SEPOLIA => Ok(Chain::ArbitrumSepolia),
        CHAIN_OPTIMISM_SEPOLIA => Ok(Chain::OptimismSepolia),
        CHAIN_BASE_SEPOLIA => Ok(Chain::BaseSepolia),
        CHAIN_BNB_TESTNET => Ok(Chain::BnbTestnet),
        CHAIN_AVALANCHE_FUJI => Ok(Chain::AvalancheFuji),
        CHAIN_POLYGON_AMOY => Ok(Chain::PolygonAmoy),
        CHAIN_HYPERLIQUID_TESTNET => Ok(Chain::HyperliquidTestnet),
        CHAIN_ETHEREUM_CLASSIC_MORDOR => Ok(Chain::EthereumClassicMordor),
        CHAIN_TRON_NILE => Ok(Chain::TronNile),
        CHAIN_SOLANA_DEVNET => Ok(Chain::SolanaDevnet),
        CHAIN_XRP_TESTNET => Ok(Chain::XrpTestnet),
        CHAIN_STELLAR_TESTNET => Ok(Chain::StellarTestnet),
        CHAIN_CARDANO_PREPROD => Ok(Chain::CardanoPreprod),
        CHAIN_SUI_TESTNET => Ok(Chain::SuiTestnet),
        CHAIN_APTOS_TESTNET => Ok(Chain::AptosTestnet),
        CHAIN_TON_TESTNET => Ok(Chain::TonTestnet),
        CHAIN_NEAR_TESTNET => Ok(Chain::NearTestnet),
        CHAIN_POLKADOT_WESTEND => Ok(Chain::PolkadotWestend),
        CHAIN_MONERO_STAGENET => Ok(Chain::MoneroStagenet),
        other => Err(format!("Unsupported chain id: {other}")),
    }
}

pub(crate) fn parse_curve(value: u32) -> Result<CurveFamily, String> {
    match value {
        CURVE_SECP256K1 => Ok(CurveFamily::Secp256k1),
        CURVE_ED25519 => Ok(CurveFamily::Ed25519),
        CURVE_SR25519 => Ok(CurveFamily::Sr25519),
        other => Err(format!("Unsupported curve id: {other}")),
    }
}

pub(crate) fn parse_derivation_algorithm(value: u32) -> Result<DerivationAlgorithm, String> {
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

pub(crate) fn parse_address_algorithm(value: u32) -> Result<AddressAlgorithm, String> {
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

pub(crate) fn parse_public_key_format(value: u32) -> Result<PublicKeyFormat, String> {
    match value {
        PUBLIC_KEY_AUTO => Ok(PublicKeyFormat::Auto),
        PUBLIC_KEY_COMPRESSED => Ok(PublicKeyFormat::Compressed),
        PUBLIC_KEY_UNCOMPRESSED => Ok(PublicKeyFormat::Uncompressed),
        PUBLIC_KEY_X_ONLY => Ok(PublicKeyFormat::XOnly),
        PUBLIC_KEY_RAW => Ok(PublicKeyFormat::Raw),
        other => Err(format!("Unsupported public key format id: {other}")),
    }
}

pub(crate) fn parse_script_type(value: u32) -> Result<ScriptType, String> {
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

// ── from former enums.rs ─────────────────────────────────────────


#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chain {
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
    // Testnets — each is its own chain with its own derivation row in
    // derivation_presets.toml. Network-flavor parameters are gone; the
    // chain itself encodes mainnet vs testnet at every byte-selection site.
    BitcoinTestnet,
    BitcoinTestnet4,
    BitcoinSignet,
    LitecoinTestnet,
    BitcoinCashTestnet,
    BitcoinSvTestnet,
    DogecoinTestnet,
    ZcashTestnet,
    DecredTestnet,
    KaspaTestnet,
    DashTestnet,
    EthereumSepolia,
    EthereumHoodi,
    ArbitrumSepolia,
    OptimismSepolia,
    BaseSepolia,
    BnbTestnet,
    AvalancheFuji,
    PolygonAmoy,
    HyperliquidTestnet,
    EthereumClassicMordor,
    TronNile,
    SolanaDevnet,
    XrpTestnet,
    StellarTestnet,
    CardanoPreprod,
    SuiTestnet,
    AptosTestnet,
    TonTestnet,
    NearTestnet,
    PolkadotWestend,
    MoneroStagenet,
}

impl Chain {
    /// Inverse of [`parse_chain`]: map a `Chain` enum variant
    /// back to its numeric wire id. Used to look up presets loaded from
    /// `derivation_presets.toml`.
    pub(crate) fn id(self) -> u32 {
        match self {
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
            Chain::BitcoinTestnet => CHAIN_BITCOIN_TESTNET,
            Chain::BitcoinTestnet4 => CHAIN_BITCOIN_TESTNET4,
            Chain::BitcoinSignet => CHAIN_BITCOIN_SIGNET,
            Chain::LitecoinTestnet => CHAIN_LITECOIN_TESTNET,
            Chain::BitcoinCashTestnet => CHAIN_BITCOIN_CASH_TESTNET,
            Chain::BitcoinSvTestnet => CHAIN_BITCOIN_SV_TESTNET,
            Chain::DogecoinTestnet => CHAIN_DOGECOIN_TESTNET,
            Chain::ZcashTestnet => CHAIN_ZCASH_TESTNET,
            Chain::DecredTestnet => CHAIN_DECRED_TESTNET,
            Chain::KaspaTestnet => CHAIN_KASPA_TESTNET,
            Chain::DashTestnet => CHAIN_DASH_TESTNET,
            Chain::EthereumSepolia => CHAIN_ETHEREUM_SEPOLIA,
            Chain::EthereumHoodi => CHAIN_ETHEREUM_HOODI,
            Chain::ArbitrumSepolia => CHAIN_ARBITRUM_SEPOLIA,
            Chain::OptimismSepolia => CHAIN_OPTIMISM_SEPOLIA,
            Chain::BaseSepolia => CHAIN_BASE_SEPOLIA,
            Chain::BnbTestnet => CHAIN_BNB_TESTNET,
            Chain::AvalancheFuji => CHAIN_AVALANCHE_FUJI,
            Chain::PolygonAmoy => CHAIN_POLYGON_AMOY,
            Chain::HyperliquidTestnet => CHAIN_HYPERLIQUID_TESTNET,
            Chain::EthereumClassicMordor => CHAIN_ETHEREUM_CLASSIC_MORDOR,
            Chain::TronNile => CHAIN_TRON_NILE,
            Chain::SolanaDevnet => CHAIN_SOLANA_DEVNET,
            Chain::XrpTestnet => CHAIN_XRP_TESTNET,
            Chain::StellarTestnet => CHAIN_STELLAR_TESTNET,
            Chain::CardanoPreprod => CHAIN_CARDANO_PREPROD,
            Chain::SuiTestnet => CHAIN_SUI_TESTNET,
            Chain::AptosTestnet => CHAIN_APTOS_TESTNET,
            Chain::TonTestnet => CHAIN_TON_TESTNET,
            Chain::NearTestnet => CHAIN_NEAR_TESTNET,
            Chain::PolkadotWestend => CHAIN_POLKADOT_WESTEND,
            Chain::MoneroStagenet => CHAIN_MONERO_STAGENET,
        }
    }

    /// Returns the mainnet variant for a testnet chain, or `self` for
    /// mainnet chains. Used by family-dispatch sites that share derivation
    /// logic between mainnet and testnet (e.g., EVM chains, Bitcoin family).
    pub(crate) fn mainnet_counterpart(self) -> Chain {
        match self {
            Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => Chain::Bitcoin,
            Chain::LitecoinTestnet => Chain::Litecoin,
            Chain::BitcoinCashTestnet => Chain::BitcoinCash,
            Chain::BitcoinSvTestnet => Chain::BitcoinSv,
            Chain::DogecoinTestnet => Chain::Dogecoin,
            Chain::ZcashTestnet => Chain::Zcash,
            Chain::DecredTestnet => Chain::Decred,
            Chain::KaspaTestnet => Chain::Kaspa,
            Chain::DashTestnet => Chain::Dash,
            Chain::EthereumSepolia | Chain::EthereumHoodi => Chain::Ethereum,
            Chain::ArbitrumSepolia => Chain::Arbitrum,
            Chain::OptimismSepolia => Chain::Optimism,
            Chain::BaseSepolia => Chain::Ethereum,
            Chain::BnbTestnet => Chain::Ethereum,
            Chain::AvalancheFuji => Chain::Avalanche,
            Chain::PolygonAmoy => Chain::Ethereum,
            Chain::HyperliquidTestnet => Chain::Hyperliquid,
            Chain::EthereumClassicMordor => Chain::EthereumClassic,
            Chain::TronNile => Chain::Tron,
            Chain::SolanaDevnet => Chain::Solana,
            Chain::XrpTestnet => Chain::Xrp,
            Chain::StellarTestnet => Chain::Stellar,
            Chain::CardanoPreprod => Chain::Cardano,
            Chain::SuiTestnet => Chain::Sui,
            Chain::AptosTestnet => Chain::Aptos,
            Chain::TonTestnet => Chain::Ton,
            Chain::NearTestnet => Chain::Near,
            Chain::PolkadotWestend => Chain::Polkadot,
            Chain::MoneroStagenet => Chain::Monero,
            c => c,
        }
    }

    /// True if this chain's preset uses secp256k1. Reads the preset table.
    pub(crate) fn is_secp(self) -> bool {
        super::presets::preset_by_chain_id(self.id())
            .map(|preset| preset.curve == CURVE_SECP256K1)
            .unwrap_or(false)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurveFamily {
    Secp256k1,
    Ed25519,
    // Schnorr/Ristretto on Curve25519 — Polkadot/Substrate signing curve.
    // The 32-byte private key is the "mini secret" that schnorrkel expands
    // into a full keypair via SHA-512 (ExpansionMode::Ed25519).
    Sr25519,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DerivationAlgorithm {
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
pub(crate) enum AddressAlgorithm {
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
pub(crate) enum PublicKeyFormat {
    Auto,
    Compressed,
    Uncompressed,
    XOnly,
    Raw,
}

#[derive(Clone, Copy)]
pub(crate) enum ScriptType {
    Auto,
    P2pkh,
    P2shP2wpkh,
    P2wpkh,
    P2tr,
    Account,
}

// ── from former request.rs ─────────────────────────────────────────

#[derive(Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIDerivationRequest {
    // Deprecated: chain is now inferred from `address_algorithm` at parse
    // time. Callers may still send it for backwards compatibility, but the
    // value is ignored by the Rust side.
    #[serde(default)]
    pub chain: Option<u32>,
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
    #[serde(default)]
    pub chain: Option<u32>,
    pub curve: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub private_key_hex: String,
}

#[derive(Debug, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIMaterialRequest {
    #[serde(default)]
    pub chain: Option<u32>,
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
    #[serde(default)]
    pub chain: Option<u32>,
    pub curve: u32,
    pub address_algorithm: u32,
    pub public_key_format: u32,
    pub script_type: u32,
    pub private_key_hex: String,
    pub derivation_path: String,
}

pub(crate) struct ParsedRequest {
    pub(crate) chain: Chain,
    pub(crate) curve: CurveFamily,
    pub(crate) requested_outputs: u32,
    pub(crate) derivation_algorithm: DerivationAlgorithm,
    pub(crate) address_algorithm: AddressAlgorithm,
    pub(crate) public_key_format: PublicKeyFormat,
    pub(crate) script_type: ScriptType,
    pub(crate) seed_phrase: String,
    pub(crate) derivation_path: Option<String>,
    pub(crate) passphrase: String,
    pub(crate) hmac_key: Option<String>,
    pub(crate) mnemonic_wordlist: Option<String>,
    pub(crate) iteration_count: u32,
    pub(crate) salt_prefix: Option<String>,
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

pub(crate) struct ParsedPrivateKeyRequest {
    pub(crate) chain: Chain,
    pub(crate) curve: CurveFamily,
    pub(crate) address_algorithm: AddressAlgorithm,
    pub(crate) public_key_format: PublicKeyFormat,
    pub(crate) script_type: ScriptType,
    pub(crate) private_key: [u8; 32],
}

pub(crate) fn parse_uniffi_request(
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
    // `salt_prefix` is intentionally NOT trimmed — callers may legitimately
    // use a prefix that includes or consists entirely of whitespace. Only
    // an explicit `None` falls back to the BIP-39 default of "mnemonic".
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

pub(crate) fn parse_uniffi_private_key_request(
    request: UniFFIPrivateKeyDerivationRequest,
) -> Result<ParsedPrivateKeyRequest, crate::SpectraBridgeError> {
    let address_algorithm = parse_address_algorithm(request.address_algorithm)?;
    let chain = match request.chain {
        Some(value) => parse_chain(value)?,
        None => chain_from_address_algorithm(address_algorithm)?,
    };
    Ok(ParsedPrivateKeyRequest {
        chain,
        curve: parse_curve(request.curve)?,
        address_algorithm,
        public_key_format: parse_public_key_format(request.public_key_format)?,
        script_type: parse_script_type(request.script_type)?,
        private_key: decode_private_key_hex(&request.private_key_hex)?,
    })
}

// ── Cross-field request validation ───────────────────────────────────────

pub(crate) fn validate_request(request: &ParsedRequest) -> Result<(), String> {
    // Enforce curve/algorithm compatibility. Wordlist, salt prefix, iteration
    // count, and HMAC key are user-customizable and resolved downstream.
    if request.iteration_count == 1 {
        return Err("Iteration count must be 0 (default) or >= 2.".to_string());
    }

    // Validate the wordlist identifier up-front so misspellings fail fast.
    let _ = resolve_bip39_language(request.mnemonic_wordlist.as_deref())?;

    if request.chain.is_secp() {
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
    } else if matches!(request.chain, Chain::Polkadot | Chain::Bittensor | Chain::PolkadotWestend) {
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

// ── from former response.rs ─────────────────────────────────────────


/// Internal: the result of a single `derive(parsed)` call. Each field is
/// optional because callers select which outputs they want via the
/// `requested_outputs` bitflag.
pub(crate) struct DerivedOutput {
    pub(crate) address: Option<String>,
    pub(crate) public_key_hex: Option<String>,
    pub(crate) private_key_hex: Option<String>,
}

/// Internal: signing-material flavor of `DerivedOutput`. Always carries
/// an address + private key + the path's parsed `(account, branch, index)`.
pub(crate) struct DerivedMaterial {
    pub(crate) address: String,
    pub(crate) private_key_hex: String,
    pub(crate) derivation_path: String,
    pub(crate) account: u32,
    pub(crate) branch: u32,
    pub(crate) index: u32,
}

#[derive(Debug, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UniFFIDerivationResponse {
    pub address: Option<String>,
    pub public_key_hex: Option<String>,
    pub private_key_hex: Option<String>,
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

// ── from former material.rs ─────────────────────────────────────────

pub(crate) struct ParsedMaterialRequest {
    pub(crate) request: ParsedRequest,
    pub(crate) derivation_path: String,
}

pub(crate) struct ParsedPrivateKeyMaterialRequest {
    pub(crate) request: ParsedPrivateKeyRequest,
    pub(crate) derivation_path: String,
}

// ── Hex utilities for raw private keys ───────────────────────────────────

pub(crate) fn decode_private_key_hex(raw: &str) -> Result<[u8; 32], String> {
    let value = raw.trim();
    if value.len() != 64 {
        return Err("Private key hex must be exactly 64 hex characters.".to_string());
    }
    let decoded = hex::decode(value).map_err(|e| e.to_string())?;
    if decoded.len() != 32 {
        return Err("Private key must decode to 32 bytes.".to_string());
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

pub(crate) fn encode_private_key_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Trim + non-empty validation shared by both material-request parsers.
/// Returns the trimmed path without re-allocating when no whitespace was
/// present (the common case for canonical derivation paths).
pub(crate) fn require_material_path(raw: String) -> Result<String, crate::SpectraBridgeError> {
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

pub(crate) fn parse_uniffi_material_request(
    request: UniFFIMaterialRequest,
) -> Result<ParsedMaterialRequest, crate::SpectraBridgeError> {
    let derivation_path = require_material_path(request.derivation_path)?;
    let parsed = parse_uniffi_request(UniFFIDerivationRequest {
        chain: request.chain,
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

pub(crate) fn parse_uniffi_private_key_material_request(
    request: UniFFIPrivateKeyMaterialRequest,
) -> Result<ParsedPrivateKeyMaterialRequest, crate::SpectraBridgeError> {
    let derivation_path = require_material_path(request.derivation_path)?;
    let parsed = parse_uniffi_private_key_request(UniFFIPrivateKeyDerivationRequest {
        chain: request.chain,
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

pub(crate) fn build_material(
    request: ParsedMaterialRequest,
    derive_fn: impl FnOnce(ParsedRequest) -> Result<DerivedOutput, String>,
) -> Result<DerivedMaterial, crate::SpectraBridgeError> {
    let result = derive_fn(request.request)?;
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

pub(crate) fn build_material_from_private_key(
    request: ParsedPrivateKeyMaterialRequest,
    derive_fn: impl FnOnce(ParsedPrivateKeyRequest) -> Result<DerivedOutput, String>,
) -> Result<DerivedMaterial, crate::SpectraBridgeError> {
    let ParsedPrivateKeyMaterialRequest {
        request,
        derivation_path,
    } = request;
    let private_key_hex = encode_private_key_hex(&request.private_key);
    let result = derive_fn(request)?;
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

/// Extract `(account, branch, index)` from a BIP-32 path. Returns
/// `(0, 0, 0)` for paths that don't have the expected segment count.
pub(crate) fn parse_account_branch_index(path: &str) -> (u32, u32, u32) {
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

// ── from former api.rs ─────────────────────────────────────────

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
    let result = build_material(parsed, derive)?;
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
    let result = build_material_from_private_key(parsed, derive_from_private_key)?;
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

// ── from former service.rs ─────────────────────────────────────────


/// Derive a single address from a seed phrase, chain name, and derivation
/// path, using the canonical per-chain algorithm defaults from
/// `derivation_presets.toml`. Returns `Ok(None)` for unknown chain names.
pub(super) fn derive_address_for_chain(
    seed_phrase: &str,
    chain_name: &str,
    path: &str,
) -> Result<Option<String>, crate::SpectraBridgeError> {
    let (chain_id, curve, deriv_alg, addr_alg, pubkey_fmt, script_opt) =
        match chain_defaults_from_name(chain_name) {
            Some(defaults) => defaults,
            None => return Ok(None),
        };

    // For Bitcoin, script type depends on the purpose level in the path
    // (44/49/84/86). All other chains use a fixed script type supplied by
    // chain_defaults_from_name.
    let script = script_opt.unwrap_or_else(|| script_type_from_purpose(path));

    let _ = chain_id; // chain is inferred from address_algorithm at parse time
    let request = UniFFIDerivationRequest {
        chain: None,
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

/// Derive full key material (address, public key, private key) for a
/// chain, applying optional power-user overrides (passphrase, wordlist,
/// custom algorithm/curve/address overrides, etc.). Each override that
/// is `Some` replaces the chain preset default; fields that are `None`
/// fall back to the preset.
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
/// Returns `(chain_id, curve, derivation_algorithm, address_algorithm,
/// public_key_format, script_type)`. `script_type = None` means the caller
/// should infer it from the path's purpose level (used for Bitcoin where
/// the address format varies by purpose: 44/49/84/86).
///
/// Data-driven from `core/data/derivation_presets.toml`; see
/// [`super::presets`].
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

// ── from former dispatch.rs ─────────────────────────────────────────

// ── Top-level dispatch ───────────────────────────────────────────────────

/// Validate cross-field constraints, then dispatch by family using
/// `mainnet_counterpart` so testnet variants route to the same shared
/// derivation routine; the routine branches on `request.chain` for
/// byte-selection differences.
pub(crate) fn derive(request: ParsedRequest) -> Result<DerivedOutput, String> {
    validate_request(&request)?;

    let actual = request.chain;
    match actual.mainnet_counterpart() {
        Chain::Bitcoin => super::chains::bitcoin::derive(request),
        Chain::BitcoinCash => super::chains::bitcoin_cash::derive(request),
        Chain::BitcoinSv => super::chains::bitcoin_sv::derive(request),
        Chain::Dash => super::chains::dash::derive(request),
        Chain::BitcoinGold => super::chains::bitcoin_gold::derive(request),
        Chain::Litecoin => super::chains::litecoin::derive(request),
        Chain::Dogecoin => super::chains::dogecoin::derive(request),
        Chain::Ethereum
        | Chain::EthereumClassic
        | Chain::Arbitrum
        | Chain::Optimism
        | Chain::Avalanche
        | Chain::Hyperliquid => super::chains::evm::derive(request),
        Chain::Tron => super::chains::tron::derive(request),
        Chain::Solana => super::chains::solana::derive(request),
        Chain::Stellar => super::chains::stellar::derive(request),
        Chain::Xrp => super::chains::xrp::derive(request),
        Chain::Cardano => super::chains::cardano::derive(request),
        Chain::Sui => super::chains::sui::derive(request),
        Chain::Aptos => super::chains::aptos::derive(request),
        Chain::Ton => super::chains::ton::derive(request),
        Chain::InternetComputer => super::chains::icp::derive(request),
        Chain::Near => super::chains::near::derive(request),
        Chain::Polkadot => super::chains::polkadot::derive(request),
        Chain::Bittensor => super::chains::bittensor::derive(request),
        Chain::Monero => super::chains::monero::derive(request),
        Chain::Zcash => super::chains::zcash::derive(request),
        Chain::Decred => super::chains::decred::derive(request),
        Chain::Kaspa => super::chains::kaspa::derive(request),
        _ => Err("internal error: mainnet_counterpart returned a testnet variant".to_string()),
    }
}

pub(crate) fn derive_from_private_key(
    request: ParsedPrivateKeyRequest,
) -> Result<DerivedOutput, String> {
    if request.chain.is_secp() {
        if request.curve != CurveFamily::Secp256k1 {
            return Err("This chain currently requires secp256k1.".to_string());
        }

        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&request.private_key).map_err(display_error)?;
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);

        let address = derive_address_from_keys(
            request.chain,
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

    if matches!(
        request.chain,
        Chain::Polkadot | Chain::Bittensor | Chain::PolkadotWestend
    ) {
        if request.curve != CurveFamily::Sr25519 {
            return Err("Polkadot/Bittensor currently require sr25519.".to_string());
        }
        let mini = schnorrkel::MiniSecretKey::from_bytes(&request.private_key)
            .map_err(|e| format!("Invalid sr25519 mini-secret: {e}"))?;
        let keypair = mini.expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
        let public_key = keypair.public.to_bytes();
        let prefix: u16 = if matches!(request.chain, Chain::Bittensor | Chain::PolkadotWestend) {
            42
        } else {
            0
        };
        return Ok(DerivedOutput {
            address: Some(encode_ss58(&public_key, prefix)),
            public_key_hex: Some(hex::encode(public_key)),
            private_key_hex: Some(hex::encode(request.private_key)),
        });
    }

    if matches!(request.chain, Chain::Monero | Chain::MoneroStagenet) {
        if request.curve != CurveFamily::Ed25519 {
            return Err("Monero currently requires ed25519.".to_string());
        }
        let (private_spend, public_spend, _private_view, public_view) =
            derive_monero_keys_from_spend_seed(&request.private_key)?;
        let address = encode_monero_main_address(&public_spend, &public_view, request.chain)?;
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
    address_algorithm: AddressAlgorithm,
    script_type: ScriptType,
    public_key: &PublicKey,
    secp: &Secp256k1<All>,
) -> Result<String, String> {
    match chain.mainnet_counterpart() {
        Chain::Bitcoin => {
            if !matches!(address_algorithm, AddressAlgorithm::Bitcoin) {
                return Err("Bitcoin requests require the Bitcoin address algorithm.".to_string());
            }
            derive_bitcoin_address_for_network(
                bitcoin_network_params_for_chain(chain),
                script_type,
                public_key,
                secp,
            )
        }
        Chain::BitcoinCash | Chain::BitcoinSv => {
            let v = if matches!(chain, Chain::BitcoinCashTestnet | Chain::BitcoinSvTestnet) {
                0x6fu8
            } else {
                0x00u8
            };
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![v];
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
            let v = if matches!(chain, Chain::DashTestnet) { 0x8Cu8 } else { 0x4Cu8 };
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![v];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Litecoin => {
            let v = if matches!(chain, Chain::LitecoinTestnet) { 0x6fu8 } else { 0x30u8 };
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = vec![v];
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Zcash => {
            let prefix: [u8; 2] = if matches!(chain, Chain::ZcashTestnet) {
                [0x1Du8, 0x25u8]
            } else {
                [0x1Cu8, 0xB8u8]
            };
            let pubkey_hash = hash160_bytes(&public_key.serialize());
            let mut payload = prefix.to_vec();
            payload.extend_from_slice(&pubkey_hash);
            Ok(base58check_encode(&payload, bs58::Alphabet::DEFAULT))
        }
        Chain::Dogecoin => {
            let version = if matches!(chain, Chain::DogecoinTestnet) { 0x71 } else { 0x1e };
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

fn bitcoin_network_params_for_chain(chain: Chain) -> BitcoinNetworkParams {
    match chain {
        Chain::BitcoinTestnet | Chain::BitcoinTestnet4 | Chain::BitcoinSignet => BTC_TESTNET,
        _ => BTC_MAINNET,
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
        Chain::Cardano => derive_cardano_shelley_enterprise_address(public_key, chain),
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

// ── EVM-specific address derivation ──────────────────────────────────────

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


