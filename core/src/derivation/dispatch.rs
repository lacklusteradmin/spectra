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
    use crate::registry::Chain;

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

    let Some(chain) = crate::registry::Chain::from_display_name(chain_name) else {
        return Err(SpectraBridgeError::InvalidInput {
            message: format!("unsupported chain: {chain_name}"),
        });
    };

    // Keyed on `Chain`, not on the display name. The string match this replaces
    // had seventy-eight arms and no way to say it had them all; a name with a
    // typo fell through to the error arm and read as an unsupported chain.
    let result = match chain {
        Chain::Bitcoin => btc::derive_bitcoin(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinTestnet => btc::derive_bitcoin_testnet(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinTestnet4 => btc::derive_bitcoin_testnet4(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinSignet => btc::derive_bitcoin_signet(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinCash => { bch::derive_bitcoin_cash(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)? },
        Chain::BitcoinCashTestnet => { bch::derive_bitcoin_cash_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)? },
        Chain::BitcoinSV => bsv::derive_bitcoin_sv(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        Chain::BitcoinSVTestnet => { bsv::derive_bitcoin_sv_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)? },
        Chain::Litecoin => ltc::derive_litecoin(s, p, pass, script, wa, wp, wk)?,
        Chain::LitecoinTestnet => ltc::derive_litecoin_testnet(s, p, pass, script, wa, wp, wk)?,
        Chain::Dogecoin => doge::derive_dogecoin(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        Chain::DogecoinTestnet => { doge::derive_dogecoin_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)? },
        Chain::Dash => dash::derive_dash(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        Chain::DashTestnet => { dash::derive_dash_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)? },
        Chain::BitcoinGold => { btg::derive_bitcoin_gold(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)? },
        Chain::Zcash => zcash::derive_zcash(s, p, pass, wa, wp, wk)?,
        Chain::ZcashTestnet => zcash::derive_zcash_testnet(s, p, pass, wa, wp, wk)?,
        Chain::Decred => decred::derive_decred(s, p, pass, wa, wp, wk)?,
        Chain::DecredTestnet => decred::derive_decred_testnet(s, p, pass, wa, wp, wk)?,
        Chain::Kaspa => kaspa::derive_kaspa(s, p, pass, wa, wp, wk)?,
        Chain::KaspaTestnet => kaspa::derive_kaspa_testnet(s, p, pass, wa, wp, wk)?,
        Chain::Tron => tron::derive_tron(s, p, pass, wa, wp, wk)?,
        Chain::TronNile => tron::derive_tron_nile(s, p, pass, wa, wp, wk)?,
        Chain::Solana => solana::derive_solana(s, p, pass, hmac, wa, wp, wk)?,
        Chain::SolanaDevnet => solana::derive_solana_devnet(s, p, pass, hmac, wa, wp, wk)?,
        Chain::Stellar => stellar::derive_stellar(s, p, pass, hmac, wa, wp, wk)?,
        Chain::StellarTestnet => stellar::derive_stellar_testnet(s, p, pass, hmac, wa, wp, wk)?,
        Chain::Xrp => xrp::derive_xrp(s, p, pass, wa, wp, wk)?,
        Chain::XrpTestnet => xrp::derive_xrp_testnet(s, p, pass, wa, wp, wk)?,
        Chain::Cardano => cardano::derive_cardano(s, Some(p), pass, wa, wp, wk)?,
        Chain::CardanoPreprod => cardano::derive_cardano_preprod(s, Some(p), pass, wa, wp, wk)?,
        Chain::Sui => sui::derive_sui(s, p, pass, wa, wp, wk)?,
        Chain::SuiTestnet => sui::derive_sui_testnet(s, p, pass, wa, wp, wk)?,
        Chain::Aptos => aptos::derive_aptos(s, p, pass, wa, wp, wk)?,
        Chain::AptosTestnet => aptos::derive_aptos_testnet(s, p, pass, wa, wp, wk)?,
        Chain::Ton => ton::derive_ton(s, pass, wa, wp, wk)?,
        Chain::TonTestnet => ton::derive_ton_testnet(s, pass, wa, wp, wk)?,
        Chain::Icp => icp::derive_icp(s, p, pass, wa, wp, wk)?,
        Chain::Near => near::derive_near(s, pass, wa, wp, wk)?,
        Chain::NearTestnet => near::derive_near_testnet(s, pass, wa, wp, wk)?,
        Chain::Polkadot => polkadot::derive_polkadot(s, pass, hmac, wa, wp, wk)?,
        Chain::PolkadotWestend => polkadot::derive_polkadot_westend(s, pass, hmac, wa, wp, wk)?,
        Chain::Bittensor => bittensor::derive_bittensor(s, pass, wa, wp, wk)?,
        Chain::Monero => xmr::derive_monero(s, wa, wp, wk)?,
        Chain::MoneroStagenet => xmr::derive_monero_stagenet(s, wa, wp, wk)?,
        // Every EVM chain derives the same address from the same path — there
        // is no chain-specific encoding — so the thirty-three arms that stood
        // here picked between thirty-three copies of one function.
        c if c.is_evm() => evm::derive_evm(s, p, pass, wa, wp, wk)?,
        other => {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("unsupported chain: {}", other.chain_display_name()),
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
/// error — but it is no longer something a user can reach either. The import
/// picker is built from [`Chain::derives_from_private_key`], which
/// `the_registry_flag_and_the_dispatcher_agree_on_every_chain` pins to this
/// match, so a chain that lands here was named by a caller rather than chosen
/// in the app.
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

    /// The dispatcher and `Chain::derives_from_private_key` are one answer.
    ///
    /// This match decides *which algorithm*; the registry flag decides
    /// *whether one exists*, and the import flow reads the flag to know what to
    /// offer. Before they were bound together there were four lists — the
    /// picker's, the submit gate's, Swift's switch and this match — and no two
    /// agreed, so a chain could be offered, accepted, and then produce no
    /// address. Walking every chain is what none of the four could do.
    #[test]
    fn the_registry_flag_and_the_dispatcher_agree_on_every_chain() {
        const KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let derives = |name: &str| {
            core_derive_from_private_key(name.to_string(), KEY.to_string(), true, false)
                .expect("a valid key never errors")
                .and_then(|r| r.address)
        };

        for chain in crate::registry::Chain::all() {
            let claimed = chain.derives_from_private_key();
            let produced = derives(chain.chain_display_name()).is_some();
            assert_eq!(
                claimed,
                produced,
                "{}: the registry says derives_from_private_key = {claimed} and the \
                 dispatcher produced an address = {produced}",
                chain.chain_display_name()
            );
        }
    }

    /// What private-key derivation does *not* cover, named rather than implied.
    ///
    /// Widening it is new derivation work, not a gate edit: these chains are
    /// absent from the import picker because nothing can derive them, and the
    /// day one of them can, it appears there without anyone editing a list.
    #[test]
    fn a_key_alone_is_not_enough_on_these_chains() {
        for name in [
            "Bitcoin SV",
            "XRP Ledger",
            "Solana",
            "Stellar",
            "Cardano",
            "Sui",
            "Aptos",
            "TON",
            "Internet Computer",
            "NEAR",
            "Polkadot",
            "Monero",
        ] {
            let chain =
                crate::registry::Chain::from_display_name(name).expect("a chain the registry knows");
            assert!(
                !chain.derives_from_private_key(),
                "{name} now derives from a private key — that is a widening, so say so \
                 in PLAN.md and take it off this list"
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
            // Every chain, with no `continue`. It used to skip the ones the
            // catalog gives no path for, which is exactly the set this test
            // most needed to cover: Monero was the only member, and it was
            // unreachable from every front end for as long as the skip was
            // here. `default_path_from_catalog` answers "" for those now, and
            // the arms that ignore the path do not mind receiving one.
            let path = crate::app_core::default_path_for_chain(chain.chain_display_name())
                .expect("a registry chain always has an answer, even when it is none");
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
