use crate::derivation::types::{BitcoinScriptType, DerivationResult};
use crate::SpectraBridgeError;

pub fn script_type_for_path(path: &str) -> BitcoinScriptType {
    let purpose = path
        .split('/')
        .find(|segment| *segment != "m" && *segment != "M")
        .map(|segment| segment.trim_end_matches('\''));
    match purpose {
        Some("44") => BitcoinScriptType::P2pkh,
        Some("49") => BitcoinScriptType::P2shP2wpkh,
        Some("86") => BitcoinScriptType::P2tr,
        _ => BitcoinScriptType::P2wpkh,
    }
}

pub fn derive_for_chain_name(
    chain_name: &str,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    script_type: Option<BitcoinScriptType>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    use crate::derivation::chains::{
        aptos, bitcoin as btc, bitcoin_cash as bch, bitcoin_gold as btg, bitcoin_sv as bsv,
        bittensor, cardano, dash, decred, dogecoin as doge, evm, icp, kaspa, litecoin as ltc,
        monero as xmr, near, polkadot, solana, stellar, sui, ton, tron, xrp, zcash,
    };

    let s = seed_phrase.to_string();
    let p = derivation_path.to_string();
    let pass = passphrase.map(str::to_string);
    let hmac = hmac_key.map(str::to_string);
    let script = script_type.unwrap_or_else(|| script_type_for_path(derivation_path));
    let wa = want_address;
    let wp = want_public_key;
    let wk = want_private_key;

    let result = match chain_name {
        "Bitcoin" => btc::derive_bitcoin(s, p, pass, script, wa, wp, wk)?,
        "Bitcoin Testnet" => btc::derive_bitcoin_testnet(s, p, pass, script, wa, wp, wk)?,
        "Bitcoin Testnet4" => btc::derive_bitcoin_testnet4(s, p, pass, script, wa, wp, wk)?,
        "Bitcoin Signet" => btc::derive_bitcoin_signet(s, p, pass, script, wa, wp, wk)?,
        "Bitcoin Cash" => {
            bch::derive_bitcoin_cash(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        "Bitcoin Cash Testnet" => {
            bch::derive_bitcoin_cash_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        "Bitcoin SV" => bsv::derive_bitcoin_sv(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        "Bitcoin SV Testnet" => {
            bsv::derive_bitcoin_sv_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        "Litecoin" => ltc::derive_litecoin(s, p, pass, script, wa, wp, wk)?,
        "Litecoin Testnet" => ltc::derive_litecoin_testnet(s, p, pass, script, wa, wp, wk)?,
        "Dogecoin" => doge::derive_dogecoin(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        "Dogecoin Testnet" => {
            doge::derive_dogecoin_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        "Dash" => dash::derive_dash(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        "Dash Testnet" => {
            dash::derive_dash_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        "Bitcoin Gold" => {
            btg::derive_bitcoin_gold(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        "Zcash" => zcash::derive_zcash(s, p, pass, wa, wp, wk)?,
        "Zcash Testnet" => zcash::derive_zcash_testnet(s, p, pass, wa, wp, wk)?,
        "Decred" => decred::derive_decred(s, p, pass, wa, wp, wk)?,
        "Decred Testnet" => decred::derive_decred_testnet(s, p, pass, wa, wp, wk)?,
        "Kaspa" => kaspa::derive_kaspa(s, p, pass, wa, wp, wk)?,
        "Kaspa Testnet" => kaspa::derive_kaspa_testnet(s, p, pass, wa, wp, wk)?,
        "Ethereum" => evm::derive_ethereum(s, p, pass, wa, wp, wk)?,
        "Ethereum Classic" => evm::derive_ethereum_classic(s, p, pass, wa, wp, wk)?,
        "Arbitrum" => evm::derive_arbitrum(s, p, pass, wa, wp, wk)?,
        "Optimism" => evm::derive_optimism(s, p, pass, wa, wp, wk)?,
        "Avalanche" => evm::derive_avalanche(s, p, pass, wa, wp, wk)?,
        "Base" => evm::derive_base(s, p, pass, wa, wp, wk)?,
        "BNB Chain" => evm::derive_bnb(s, p, pass, wa, wp, wk)?,
        "Polygon" => evm::derive_polygon(s, p, pass, wa, wp, wk)?,
        "Hyperliquid" => evm::derive_hyperliquid(s, p, pass, wa, wp, wk)?,
        "Linea" => evm::derive_linea(s, p, pass, wa, wp, wk)?,
        "Scroll" => evm::derive_scroll(s, p, pass, wa, wp, wk)?,
        "Blast" => evm::derive_blast(s, p, pass, wa, wp, wk)?,
        "Mantle" => evm::derive_mantle(s, p, pass, wa, wp, wk)?,
        "Sei" => evm::derive_sei(s, p, pass, wa, wp, wk)?,
        "Celo" => evm::derive_celo(s, p, pass, wa, wp, wk)?,
        "Cronos" => evm::derive_cronos(s, p, pass, wa, wp, wk)?,
        "opBNB" => evm::derive_op_bnb(s, p, pass, wa, wp, wk)?,
        "zkSync Era" => evm::derive_zksync_era(s, p, pass, wa, wp, wk)?,
        "Sonic" => evm::derive_sonic(s, p, pass, wa, wp, wk)?,
        "Berachain" => evm::derive_berachain(s, p, pass, wa, wp, wk)?,
        "Unichain" => evm::derive_unichain(s, p, pass, wa, wp, wk)?,
        "Ink" => evm::derive_ink(s, p, pass, wa, wp, wk)?,
        "X Layer" => evm::derive_x_layer(s, p, pass, wa, wp, wk)?,
        "Ethereum Sepolia" => evm::derive_ethereum_sepolia(s, p, pass, wa, wp, wk)?,
        "Ethereum Hoodi" => evm::derive_ethereum_hoodi(s, p, pass, wa, wp, wk)?,
        "Ethereum Classic Mordor" => evm::derive_ethereum_classic_mordor(s, p, pass, wa, wp, wk)?,
        "Arbitrum Sepolia" => evm::derive_arbitrum_sepolia(s, p, pass, wa, wp, wk)?,
        "Optimism Sepolia" => evm::derive_optimism_sepolia(s, p, pass, wa, wp, wk)?,
        "Base Sepolia" => evm::derive_base_sepolia(s, p, pass, wa, wp, wk)?,
        "BNB Chain Testnet" => evm::derive_bnb_testnet(s, p, pass, wa, wp, wk)?,
        "Avalanche Fuji" => evm::derive_avalanche_fuji(s, p, pass, wa, wp, wk)?,
        "Polygon Amoy" => evm::derive_polygon_amoy(s, p, pass, wa, wp, wk)?,
        "Hyperliquid Testnet" => evm::derive_hyperliquid_testnet(s, p, pass, wa, wp, wk)?,
        "Tron" => tron::derive_tron(s, p, pass, wa, wp, wk)?,
        "Tron Nile" => tron::derive_tron_nile(s, p, pass, wa, wp, wk)?,
        "Solana" => solana::derive_solana(s, p, pass, hmac, wa, wp, wk)?,
        "Solana Devnet" => solana::derive_solana_devnet(s, p, pass, hmac, wa, wp, wk)?,
        "Stellar" => stellar::derive_stellar(s, p, pass, hmac, wa, wp, wk)?,
        "Stellar Testnet" => stellar::derive_stellar_testnet(s, p, pass, hmac, wa, wp, wk)?,
        "XRP Ledger" => xrp::derive_xrp(s, p, pass, wa, wp, wk)?,
        "XRP Ledger Testnet" => xrp::derive_xrp_testnet(s, p, pass, wa, wp, wk)?,
        "Cardano" => cardano::derive_cardano(s, Some(p), pass, wa, wp, wk)?,
        "Cardano Preprod" => cardano::derive_cardano_preprod(s, Some(p), pass, wa, wp, wk)?,
        "Sui" => sui::derive_sui(s, p, pass, wa, wp, wk)?,
        "Sui Testnet" => sui::derive_sui_testnet(s, p, pass, wa, wp, wk)?,
        "Aptos" => aptos::derive_aptos(s, p, pass, wa, wp, wk)?,
        "Aptos Testnet" => aptos::derive_aptos_testnet(s, p, pass, wa, wp, wk)?,
        "TON" => ton::derive_ton(s, pass, wa, wp, wk)?,
        "TON Testnet" => ton::derive_ton_testnet(s, pass, wa, wp, wk)?,
        "Internet Computer" => icp::derive_icp(s, p, pass, wa, wp, wk)?,
        "NEAR" => near::derive_near(s, pass, wa, wp, wk)?,
        "NEAR Testnet" => near::derive_near_testnet(s, pass, wa, wp, wk)?,
        "Polkadot" => polkadot::derive_polkadot(s, pass, hmac, wa, wp, wk)?,
        "Polkadot Westend" => polkadot::derive_polkadot_westend(s, pass, hmac, wa, wp, wk)?,
        "Bittensor" => bittensor::derive_bittensor(s, pass, wa, wp, wk)?,
        "Monero" => xmr::derive_monero(s, wa, wp, wk)?,
        "Monero Stagenet" => xmr::derive_monero_stagenet(s, wa, wp, wk)?,
        other => {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("unsupported chain: {other}"),
            })
        }
    };

    Ok(result)
}

/// Derive for a chain named at the boundary.
///
/// One export in place of ~50 `derive<Chain>` functions, each of which Swift
/// called from exactly one arm of a 212-line switch that reproduced this
/// dispatch. The switch existed because the FFI offered no way to say "this
/// chain" — the dispatcher was here the whole time, and the CLI already used
/// it. `script_type` is honoured for the Bitcoin family and ignored elsewhere,
/// as before; `None` derives it from the path.
#[uniffi::export]
pub fn core_derive_for_chain(
    chain_name: String,
    seed_phrase: String,
    derivation_path: String,
    passphrase: Option<String>,
    hmac_key: Option<String>,
    script_type: Option<BitcoinScriptType>,
    want_address: bool,
    want_public_key: bool,
    want_private_key: bool,
) -> Result<DerivationResult, SpectraBridgeError> {
    derive_for_chain_name(
        &chain_name,
        &seed_phrase,
        &derivation_path,
        passphrase.as_deref(),
        hmac_key.as_deref(),
        script_type,
        want_address,
        want_public_key,
        want_private_key,
    )
}

/// Derive an address from a raw private key, whatever the chain.
///
/// The counterpart of [`core_derive_for_chain`], and it replaces the same
/// shape: Swift held a thirty-arm switch listing which chains derive by which
/// algorithm, calling six per-chain exports that existed only to be called from
/// it. Which family a chain belongs to is a registry fact, so the arms are
/// predicates on `Chain` rather than a typed-out list — and testnets fall out
/// of `mainnet_counterpart` instead of needing a case each.
///
/// `Ok(None)` means the chain has no private-key derivation, which is not an
/// error: the import flow offers the chain and shows no address.
#[uniffi::export]
pub fn core_derive_from_private_key(
    chain_name: String,
    private_key_hex: String,
    want_address: bool,
    want_public_key: bool,
) -> Result<Option<DerivationResult>, SpectraBridgeError> {
    use crate::derivation::chains::{
        bitcoin as btc, bitcoin_cash as bch, decred, dogecoin as doge, evm, litecoin as ltc,
    };
    use crate::registry::Chain;

    let Some(chain) = Chain::from_display_name(&chain_name) else {
        return Ok(None);
    };
    let result = match chain.mainnet_counterpart() {
        c if c.is_evm() => evm::derive_evm_from_private_key(private_key_hex, want_address, want_public_key)?,
        Chain::Bitcoin => btc::derive_bitcoin_from_private_key(
            private_key_hex,
            BitcoinScriptType::P2wpkh,
            want_address,
            want_public_key,
        )?,
        Chain::BitcoinCash => {
            bch::derive_bitcoin_cash_from_private_key(private_key_hex, want_address, want_public_key)?
        }
        Chain::Litecoin => {
            ltc::derive_litecoin_from_private_key(private_key_hex, want_address, want_public_key)?
        }
        Chain::Dogecoin => {
            doge::derive_dogecoin_from_private_key(private_key_hex, want_address, want_public_key)?
        }
        Chain::Decred => {
            decred::derive_decred_from_private_key(private_key_hex, want_address, want_public_key)?
        }
        _ => return Ok(None),
    };
    Ok(Some(result))
}

#[cfg(test)]
mod dispatch_export_tests {
    use super::*;

    /// What private-key derivation actually covers, stated once.
    ///
    /// The import gate (`PRIVATE_KEY_SUPPORTED_CHAINS`) names thirty-nine
    /// chains. Derivation covers the EVM family and five UTXO chains, and the
    /// difference is older than this dispatcher — Swift's switch had the same
    /// arms. A private key imported for XRP Ledger passes the gate and yields
    /// no address. Asserted rather than papered over: widening it is new
    /// derivation work and narrowing the gate removes an import path, so both
    /// are decisions of their own rather than a side effect of this collapse.
    #[test]
    fn private_key_derivation_covers_the_evm_family_and_five_utxo_chains() {
        const KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let derives = |name: &str| {
            core_derive_from_private_key(name.to_string(), KEY.to_string(), true, false)
                .expect("a valid key never errors")
                .and_then(|r| r.address)
        };

        for chain in crate::registry::Chain::all().filter(|c| c.is_evm()) {
            assert!(
                derives(chain.chain_display_name()).is_some(),
                "{} is EVM but derived no address from a private key",
                chain.chain_display_name()
            );
        }
        for name in ["Bitcoin", "Bitcoin Cash", "Litecoin", "Dogecoin", "Decred"] {
            assert!(derives(name).is_some(), "{name} derived no address");
        }
        for name in [
            "Bitcoin SV",
            "XRP Ledger",
            "Solana",
            "Cardano",
            "Polkadot",
            "Monero",
        ] {
            assert!(
                derives(name).is_none(),
                "{name} now derives from a private key — the import gate and the \
                 derivation table agree further than they did; update this test"
            );
        }
    }

    /// Every chain the registry lists derives through the one export.
    ///
    /// This is the property the 50 separate exports could not state: that the
    /// set of derivable chains and the set the registry knows are the same.
    #[test]
    fn every_registry_chain_derives_through_one_call() {
        const PHRASE: &str =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mut missing = Vec::new();
        for chain in crate::registry::Chain::all() {
            // Testnets derive from their mainnet's catalog path.
            let Some(template) = crate::chains::default_derivation_path_template_by_id(
                chain.mainnet_counterpart().str_id(),
            ) else {
                continue;
            };
            let path = template.replace("{account}", "0");
            let result = core_derive_for_chain(
                chain.chain_display_name().to_string(),
                PHRASE.to_string(),
                path,
                None,
                None,
                None,
                true,
                false,
                false,
            );
            match result {
                Ok(r) if r.address.is_some() => {}
                _ => missing.push(chain.chain_display_name()),
            }
        }
        assert!(missing.is_empty(), "no address derived for: {missing:?}");
    }
}
