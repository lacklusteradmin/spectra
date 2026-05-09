//! Funds Finder — scan many derivation paths from a seed phrase to locate
//! hidden or lost funds across supported blockchains.
//!
//! `core_generate_funds_finder_candidates` is a pure computation step: it
//! derives addresses for every (chain, path, script-type) combination in the
//! candidate matrix and returns them to Swift. Swift then calls the existing
//! `fetch_native_balance_summary` for each candidate and surfaces hits.

use crate::derivation::chains::{
    bitcoin::derive_bitcoin,
    bitcoin_cash::derive_bitcoin_cash,
    bitcoin_gold::derive_bitcoin_gold,
    bitcoin_sv::derive_bitcoin_sv,
    dash::derive_dash,
    dogecoin::derive_dogecoin,
    evm::derive_ethereum,
    litecoin::derive_litecoin,
    polkadot::derive_polkadot,
    solana::derive_solana,
    stellar::derive_stellar,
    tron::derive_tron,
    xrp::derive_xrp,
    zcash::derive_zcash,
};
use crate::derivation::types::BitcoinScriptType;
use crate::SpectraBridgeError;

// ── Public types ─────────────────────────────────────────────────────────────

/// Input to the candidate generation step.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FundsFinderRequest {
    pub seed_phrase: String,
    pub passphrase: Option<String>,
}

/// A single (chain, derivation path, address) tuple derived from the seed.
/// The balance of this address is checked separately by Swift.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FundsFinderCandidate {
    /// Machine-readable chain ID understood by `fetch_native_balance_summary`.
    pub chain_id: String,
    /// Human-readable chain name shown in the UI.
    pub chain_name: String,
    /// Full BIP-32 path used (e.g. `m/84'/0'/0'/0/0`).
    pub derivation_path: String,
    /// Short human-readable label (e.g. `"BIP84 Native SegWit · Account 0"`).
    pub path_label: String,
    /// The derived address to check.
    pub address: String,
}

// ── FFI export ────────────────────────────────────────────────────────────────

/// Derive addresses for every (chain, path) combination in the candidate
/// matrix and return the full list. Pure computation — no network calls.
///
/// Derivation errors for individual candidates are silently skipped so that
/// a bad path for one chain doesn't abort the entire scan.
#[uniffi::export]
pub fn core_generate_funds_finder_candidates(
    request: FundsFinderRequest,
) -> Result<Vec<FundsFinderCandidate>, SpectraBridgeError> {
    let seed = &request.seed_phrase;
    let pass = request.passphrase.as_deref().filter(|s| !s.is_empty()).map(str::to_string);
    let mut out: Vec<FundsFinderCandidate> = Vec::new();

    // ── Bitcoin ───────────────────────────────────────────────────────────────
    // Four script types × accounts 0-2
    let btc_variants: &[(BitcoinScriptType, &str, &str)] = &[
        (BitcoinScriptType::P2wpkh,     "m/84'/0'/{a}'/0/0", "BIP84 Native SegWit (bc1q)"),
        (BitcoinScriptType::P2pkh,      "m/44'/0'/{a}'/0/0", "BIP44 Legacy (1…)"),
        (BitcoinScriptType::P2shP2wpkh, "m/49'/0'/{a}'/0/0", "BIP49 Nested SegWit (3…)"),
        (BitcoinScriptType::P2tr,       "m/86'/0'/{a}'/0/0", "BIP86 Taproot (bc1p)"),
    ];
    for (script, path_tpl, label) in btc_variants {
        for account in 0u32..3 {
            let path = path_tpl.replace("{a}", &account.to_string());
            let label_full = if account == 0 {
                label.to_string()
            } else {
                format!("{label} · Account {account}")
            };
            push_candidate(
                &mut out,
                "bitcoin", "Bitcoin", &path, &label_full,
                || derive_bitcoin(seed.clone(), path.clone(), pass.clone(), *script, true, false, false),
            );
        }
    }

    // ── Ethereum (address is reused across all EVM chains) ────────────────────
    // Standard: m/44'/60'/account'/0/address_idx
    for account in 0u32..3 {
        for addr_idx in 0u32..3 {
            let path = format!("m/44'/60'/{}'/0/{}", account, addr_idx);
            let label = match (account, addr_idx) {
                (0, 0) => "Standard".to_string(),
                (a, 0) => format!("Account {a}"),
                (a, i) => format!("Account {a} · Address {i}"),
            };
            push_candidate(
                &mut out,
                "ethereum", "Ethereum", &path, &label,
                || derive_ethereum(seed.clone(), path.clone(), pass.clone(), true, false, false),
            );
        }
    }
    // Legacy truncated paths used by some older wallets (MyEtherWallet, etc.)
    for (path, label) in [
        ("m/44'/60'/0'",     "Legacy (m/44'/60'/0')"),
        ("m/44'/60'/0'/0",   "Legacy (m/44'/60'/0'/0)"),
        ("m/44'/60'",        "Legacy (m/44'/60')"),
    ] {
        push_candidate(
            &mut out,
            "ethereum", "Ethereum", path, label,
            || derive_ethereum(seed.clone(), path.to_string(), pass.clone(), true, false, false),
        );
    }

    // ── Solana ────────────────────────────────────────────────────────────────
    // Standard: m/44'/501'/account'/0'
    // Legacy:   m/44'/501'/account'
    for account in 0u32..3 {
        let std_path  = format!("m/44'/501'/{}'/0'", account);
        let leg_path  = format!("m/44'/501'/{}'"   , account);
        let std_label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        let leg_label = if account == 0 { "Legacy (m/44'/501'/0')".to_string() } else { format!("Legacy · Account {account}") };
        push_candidate(
            &mut out,
            "solana", "Solana", &std_path, &std_label,
            || derive_solana(seed.clone(), std_path.clone(), pass.clone(), None, true, false, false),
        );
        push_candidate(
            &mut out,
            "solana", "Solana", &leg_path, &leg_label,
            || derive_solana(seed.clone(), leg_path.clone(), pass.clone(), None, true, false, false),
        );
    }

    // ── Litecoin ──────────────────────────────────────────────────────────────
    let ltc_variants: &[(BitcoinScriptType, &str, &str)] = &[
        (BitcoinScriptType::P2wpkh,     "m/84'/2'/{a}'/0/0", "BIP84 Native SegWit (ltc1q)"),
        (BitcoinScriptType::P2pkh,      "m/44'/2'/{a}'/0/0", "BIP44 Legacy (L…/M…)"),
        (BitcoinScriptType::P2shP2wpkh, "m/49'/2'/{a}'/0/0", "BIP49 Nested SegWit (M…)"),
    ];
    for (script, path_tpl, label) in ltc_variants {
        for account in 0u32..2 {
            let path = path_tpl.replace("{a}", &account.to_string());
            let label_full = if account == 0 {
                label.to_string()
            } else {
                format!("{label} · Account {account}")
            };
            push_candidate(
                &mut out,
                "litecoin", "Litecoin", &path, &label_full,
                || derive_litecoin(seed.clone(), path.clone(), pass.clone(), *script, true, false, false),
            );
        }
    }

    // ── Dogecoin ──────────────────────────────────────────────────────────────
    for account in 0u32..3 {
        let path = format!("m/44'/3'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "dogecoin", "Dogecoin", &path, &label,
            || derive_dogecoin(seed.clone(), path.clone(), pass.clone(), BitcoinScriptType::P2pkh, true, false, false),
        );
    }

    // ── Bitcoin Cash ──────────────────────────────────────────────────────────
    for account in 0u32..2 {
        let path = format!("m/44'/145'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "bitcoin-cash", "Bitcoin Cash", &path, &label,
            || derive_bitcoin_cash(seed.clone(), path.clone(), pass.clone(), BitcoinScriptType::P2pkh, true, false, false),
        );
    }

    // ── Bitcoin SV ────────────────────────────────────────────────────────────
    for account in 0u32..2 {
        let path = format!("m/44'/236'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "bitcoin-sv", "Bitcoin SV", &path, &label,
            || derive_bitcoin_sv(seed.clone(), path.clone(), pass.clone(), BitcoinScriptType::P2pkh, true, false, false),
        );
    }

    // ── XRP Ledger ────────────────────────────────────────────────────────────
    for account in 0u32..3 {
        let path = format!("m/44'/144'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "xrp", "XRP Ledger", &path, &label,
            || derive_xrp(seed.clone(), path.clone(), pass.clone(), true, false, false),
        );
    }

    // ── Tron ──────────────────────────────────────────────────────────────────
    for account in 0u32..3 {
        let path = format!("m/44'/195'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "tron", "Tron", &path, &label,
            || derive_tron(seed.clone(), path.clone(), pass.clone(), true, false, false),
        );
    }

    // ── Stellar ───────────────────────────────────────────────────────────────
    for account in 0u32..2 {
        let path = format!("m/44'/148'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "stellar", "Stellar", &path, &label,
            || derive_stellar(seed.clone(), path.clone(), pass.clone(), None, true, false, false),
        );
    }
    // Legacy Stellar path
    push_candidate(
        &mut out,
        "stellar", "Stellar", "m/44'/148'", "Legacy (m/44'/148')",
        || derive_stellar(seed.clone(), "m/44'/148'".to_string(), pass.clone(), None, true, false, false),
    );

    // ── Polkadot ──────────────────────────────────────────────────────────────
    // Polkadot uses sr25519 with no path — just one canonical address.
    push_candidate(
        &mut out,
        "polkadot", "Polkadot", "sr25519", "Standard (sr25519)",
        || derive_polkadot(seed.clone(), pass.clone(), None, true, false, false),
    );

    // ── Dash ──────────────────────────────────────────────────────────────────
    for account in 0u32..2 {
        let path = format!("m/44'/5'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "dash", "Dash", &path, &label,
            || derive_dash(seed.clone(), path.clone(), pass.clone(), BitcoinScriptType::P2pkh, true, false, false),
        );
    }

    // ── Zcash ─────────────────────────────────────────────────────────────────
    for account in 0u32..2 {
        let path = format!("m/44'/133'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "zcash", "Zcash", &path, &label,
            || derive_zcash(seed.clone(), path.clone(), pass.clone(), true, false, false),
        );
    }

    // ── Bitcoin Gold ──────────────────────────────────────────────────────────
    for account in 0u32..2 {
        let path = format!("m/44'/156'/{}'/0/0", account);
        let label = if account == 0 { "Standard".to_string() } else { format!("Account {account}") };
        push_candidate(
            &mut out,
            "bitcoin-gold", "Bitcoin Gold", &path, &label,
            || derive_bitcoin_gold(seed.clone(), path.clone(), pass.clone(), BitcoinScriptType::P2pkh, true, false, false),
        );
    }

    Ok(out)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn push_candidate(
    out: &mut Vec<FundsFinderCandidate>,
    chain_id: &str,
    chain_name: &str,
    path: &str,
    label: &str,
    derive: impl FnOnce() -> Result<crate::derivation::types::DerivationResult, SpectraBridgeError>,
) {
    if let Ok(result) = derive() {
        if let Some(address) = result.address {
            out.push(FundsFinderCandidate {
                chain_id:        chain_id.to_string(),
                chain_name:      chain_name.to_string(),
                derivation_path: path.to_string(),
                path_label:      label.to_string(),
                address,
            });
        }
    }
}
