use crate::derivation::api::bitcoin as bitcoin_api;
use crate::derivation::api::types::BitcoinScriptType;
use crate::derivation::chains::{
    aptos, bittensor, cardano, decred, evm, icp, kaspa,
    monero as monero_chain, near, polkadot, solana, stellar, sui, ton, tron, xrp,
};
use crate::derivation::chains::bitcoin as bitcoin_chain;
use crate::SpectraBridgeError;

// ── script type inference from BIP-44 purpose level ──────────────────────────

fn script_type_from_path(path: &str) -> BitcoinScriptType {
    let purpose = path
        .split('/')
        .find(|s| *s != "m" && *s != "M")
        .map(|s| s.trim_end_matches('\''));
    match purpose {
        Some("44") => BitcoinScriptType::P2pkh,
        Some("49") => BitcoinScriptType::P2shP2wpkh,
        Some("86") => BitcoinScriptType::P2tr,
        _          => BitcoinScriptType::P2wpkh,
    }
}

fn script_type_from_override(name: &str) -> Option<BitcoinScriptType> {
    match name.to_lowercase().as_str() {
        "p2pkh"      => Some(BitcoinScriptType::P2pkh),
        "p2shp2wpkh" | "p2sh-p2wpkh" => Some(BitcoinScriptType::P2shP2wpkh),
        "p2wpkh"     => Some(BitcoinScriptType::P2wpkh),
        "p2tr"       => Some(BitcoinScriptType::P2tr),
        _ => None,
    }
}

// ── primary dispatch ──────────────────────────────────────────────────────────

/// Derive key material (address, private key, public key) for a chain given by
/// display name. Used by the signing pipeline (`service.rs`) and diagnostics.
/// Only `passphrase`, `hmac_key`, and `script_type_override` are honored from
/// power-user overrides; algorithm overrides are ignored because each named
/// function has its algorithm baked in.
pub fn derive_for_chain(
    chain_name: &str,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    script_type_override: Option<&str>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<(Option<String>, Option<String>, Option<String>), SpectraBridgeError> {
    let pass = passphrase.filter(|s| !s.is_empty());
    let hmac = hmac_key.filter(|s| !s.is_empty());
    let script = script_type_override
        .and_then(script_type_from_override)
        .unwrap_or_else(|| script_type_from_path(derivation_path));
    let wa = want_address; let wp = want_public_key; let wk = want_private_key;
    let path = derivation_path;

    let result = match chain_name {
        // ── Bitcoin family ────────────────────────────────────────────────
        "Bitcoin" =>
            bitcoin_api::derive_from_seed_phrase(bitcoin_chain::BTC_MAINNET, script, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin Testnet" =>
            bitcoin_api::derive_from_seed_phrase(bitcoin_chain::BTC_TESTNET, script, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin Testnet4" | "Bitcoin Signet" =>
            bitcoin_api::derive_from_seed_phrase(bitcoin_chain::BTC_TESTNET, script, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin Cash" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::BCH_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin Cash Testnet" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::BCH_TESTNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin SV" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::BSV_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin SV Testnet" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::BSV_TESTNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Litecoin" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::LTC_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Litecoin Testnet" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::LTC_TESTNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Dogecoin" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::DOGE_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Dogecoin Testnet" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::DOGE_TESTNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Dash" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::DASH_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Dash Testnet" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::DASH_TESTNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Bitcoin Gold" =>
            bitcoin_api::derive_simple_p2pkh(bitcoin_api::BTG_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Zcash" =>
            bitcoin_api::derive_zcash_internal(bitcoin_api::ZCASH_MAINNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Zcash Testnet" =>
            bitcoin_api::derive_zcash_internal(bitcoin_api::ZCASH_TESTNET_VERSION, seed_phrase, path, pass, wa, wp, wk)?,
        "Decred" =>
            decred::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,
        "Decred Testnet" =>
            decred::derive_from_seed_phrase_testnet(seed_phrase, path, pass, wa, wp, wk)?,
        "Kaspa" =>
            kaspa::derive_from_seed_phrase("kaspa", seed_phrase, path, pass, wa, wp, wk)?,
        "Kaspa Testnet" =>
            kaspa::derive_from_seed_phrase("kaspatest", seed_phrase, path, pass, wa, wp, wk)?,

        // ── EVM (all share the same key derivation) ───────────────────────
        "Ethereum" | "Ethereum Classic" | "Arbitrum" | "Optimism" | "Avalanche" | "Base"
        | "BNB Chain" | "Polygon" | "Hyperliquid" | "Linea" | "Scroll" | "Blast" | "Mantle"
        | "Sei" | "Celo" | "Cronos" | "opBNB" | "zkSync Era" | "Sonic" | "Berachain"
        | "Unichain" | "Ink" | "X Layer"
        | "Ethereum Sepolia" | "Ethereum Hoodi" | "Ethereum Classic Mordor"
        | "Arbitrum Sepolia" | "Optimism Sepolia" | "Base Sepolia" | "BNB Chain Testnet"
        | "Avalanche Fuji" | "Polygon Amoy" | "Hyperliquid Testnet" =>
            evm::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,

        // ── Tron ─────────────────────────────────────────────────────────
        "Tron" | "Tron Nile" =>
            tron::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,

        // ── Solana ───────────────────────────────────────────────────────
        "Solana" | "Solana Devnet" =>
            solana::derive_from_seed_phrase(seed_phrase, path, pass, hmac, wa, wp, wk)?,

        // ── Stellar ──────────────────────────────────────────────────────
        "Stellar" | "Stellar Testnet" =>
            stellar::derive_from_seed_phrase(seed_phrase, path, pass, hmac, wa, wp, wk)?,

        // ── XRP ──────────────────────────────────────────────────────────
        "XRP Ledger" | "XRP Ledger Testnet" =>
            xrp::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,

        // ── Cardano ───────────────────────────────────────────────────────
        "Cardano" =>
            cardano::derive_from_seed_phrase(true, seed_phrase, Some(path), pass, wa, wp, wk)?,
        "Cardano Preprod" =>
            cardano::derive_from_seed_phrase(false, seed_phrase, Some(path), pass, wa, wp, wk)?,

        // ── Sui ──────────────────────────────────────────────────────────
        "Sui" | "Sui Testnet" =>
            sui::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,

        // ── Aptos ────────────────────────────────────────────────────────
        "Aptos" | "Aptos Testnet" =>
            aptos::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,

        // ── TON (no derivation path) ──────────────────────────────────────
        "TON" | "TON Testnet" =>
            ton::derive_ton_standard(seed_phrase, pass, wa, wp, wk)?,

        // ── Internet Computer ─────────────────────────────────────────────
        "Internet Computer" =>
            icp::derive_from_seed_phrase(seed_phrase, path, pass, wa, wp, wk)?,

        // ── NEAR (no derivation path) ─────────────────────────────────────
        "NEAR" | "NEAR Testnet" =>
            near::derive_from_seed_phrase(seed_phrase, pass, wa, wp, wk)?,

        // ── Polkadot / Westend ────────────────────────────────────────────
        "Polkadot" => {
            let (mini, pub_key) = polkadot::derive_substrate_sr25519_material(
                seed_phrase, pass.unwrap_or(""), None, None, 0, None,
                hmac.map(|h| h == "uniform").unwrap_or(false),
            ).map_err(SpectraBridgeError::from)?;
            (
                wa.then(|| polkadot::encode_ss58(&pub_key, 0)),
                wp.then(|| hex::encode(pub_key)),
                wk.then(|| hex::encode(mini)),
            )
        }
        "Polkadot Westend" => {
            let (mini, pub_key) = polkadot::derive_substrate_sr25519_material(
                seed_phrase, pass.unwrap_or(""), None, None, 0, None,
                hmac.map(|h| h == "uniform").unwrap_or(false),
            ).map_err(SpectraBridgeError::from)?;
            (
                wa.then(|| polkadot::encode_ss58(&pub_key, 42)),
                wp.then(|| hex::encode(pub_key)),
                wk.then(|| hex::encode(mini)),
            )
        }

        // ── Bittensor ─────────────────────────────────────────────────────
        "Bittensor" =>
            bittensor::derive_from_seed_phrase(seed_phrase, pass, None, wa, wp, wk)?,

        // ── Monero ────────────────────────────────────────────────────────
        "Monero" =>
            monero_chain::derive_from_seed_phrase(true, seed_phrase, wa, wp, wk)?,
        "Monero Stagenet" =>
            monero_chain::derive_from_seed_phrase(false, seed_phrase, wa, wp, wk)?,

        other => return Err(SpectraBridgeError::from(
            format!("unsupported chain for derivation: {other}")
        )),
    };

    Ok(result)
}
