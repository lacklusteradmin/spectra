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

#[cfg(test)]
mod dispatch_export_tests {
    use super::*;

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

