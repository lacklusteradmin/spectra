use crate::SpectraBridgeError;
use crate::derivation::types::{BitcoinScriptType, DerivationResult};

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

pub fn derive_for_chain(
    chain: crate::registry::Chain,
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

    use crate::derivation::{
        aptos, bitcoin as btc, bitcoin_cash as bch, bitcoin_gold as btg, bitcoin_sv as bsv,
        bittensor, cardano, dash, decred, dogecoin as doge, evm, icp, kaspa, litecoin as ltc,
        monero as xmr, near, peercoin, polkadot, solana, stellar, sui, ton, tron, xrp, zcash,
    };

    let s = seed_phrase.to_string();
    let p = derivation_path.to_string();
    let pass = passphrase.map(str::to_string);
    let hmac = hmac_key.map(str::to_string);
    let script = script_type.unwrap_or_else(|| script_type_for_path(derivation_path));
    let wa = want_address;
    let wp = want_public_key;
    let wk = want_private_key;

    // Keyed on `Chain`, not on the display name. The string match this replaces
    // had seventy-eight arms and no way to say it had them all; a name with a
    // typo fell through to the error arm and read as an unsupported chain.
    let result = match chain {
        Chain::Bitcoin => btc::derive_bitcoin(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinTestnet => btc::derive_bitcoin_testnet(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinTestnet4 => btc::derive_bitcoin_testnet4(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinSignet => btc::derive_bitcoin_signet(s, p, pass, script, wa, wp, wk)?,
        Chain::BitcoinCash => {
            bch::derive_bitcoin_cash(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        Chain::BitcoinCashTestnet => {
            bch::derive_bitcoin_cash_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        Chain::BitcoinSV => {
            bsv::derive_bitcoin_sv(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        Chain::BitcoinSVTestnet => {
            bsv::derive_bitcoin_sv_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        Chain::Litecoin => ltc::derive_litecoin(s, p, pass, script, wa, wp, wk)?,
        Chain::LitecoinTestnet => ltc::derive_litecoin_testnet(s, p, pass, script, wa, wp, wk)?,
        Chain::Peercoin => peercoin::derive_peercoin(s, p, pass, script, wa, wp, wk)?,
        Chain::PeercoinTestnet => {
            peercoin::derive_peercoin_testnet(s, p, pass, script, wa, wp, wk)?
        }
        Chain::Dogecoin => doge::derive_dogecoin(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        Chain::DogecoinTestnet => {
            doge::derive_dogecoin_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        Chain::Dash => dash::derive_dash(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?,
        Chain::DashTestnet => {
            dash::derive_dash_testnet(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
        Chain::BitcoinGold => {
            btg::derive_bitcoin_gold(s, p, pass, BitcoinScriptType::P2pkh, wa, wp, wk)?
        }
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
        Chain::Near => near::derive_near(s, p, pass, wa, wp, wk)?,
        Chain::NearTestnet => near::derive_near_testnet(s, p, pass, wa, wp, wk)?,
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
                message: format!("unsupported chain: {}", other.chain_display_name()).into(),
            });
        }
    };

    Ok(result)
}

/// Derive an address from a raw private key, whatever the chain.
///
/// Which family a chain belongs to is a registry fact, so the arms are
/// predicates on `Chain` rather than a typed-out list, and testnets fall out
/// of `mainnet_counterpart`.
///
/// `Ok(None)` means the chain has no private-key derivation, which is not an
/// error. The import picker is built from
/// [`crate::registry::Chain::derives_from_private_key`], which
/// `the_registry_flag_and_the_dispatcher_agree_on_every_chain` pins to this
/// match, so a chain that lands here was named by a caller rather than chosen
/// in the app.
pub fn derive_from_private_key(
    chain: crate::registry::Chain,
    private_key_hex: String,
    want_address: bool,
    want_public_key: bool,
) -> Result<Option<DerivationResult>, SpectraBridgeError> {
    if !chain.derives_from_private_key() {
        return Ok(None);
    }
    let result = if chain.is_evm() {
        crate::derivation::evm::derive_evm_from_private_key(
            private_key_hex,
            want_address,
            want_public_key,
        )?
    } else {
        super::private_key::derive(chain, &private_key_hex, want_address, want_public_key)?
    };
    Ok(Some(result))
}

#[cfg(test)]
mod dispatch_export_tests {
    use super::*;

    #[test]
    fn raw_keys_derive_the_same_network_address_as_their_mnemonic_keys() {
        use crate::derivation::phrase::test_phrase;
        use crate::registry::Chain;
        for chain in Chain::all().filter(|c| c.derives_from_private_key()) {
            let path = crate::derivation::path::default_path_from_catalog(chain).unwrap();
            let expected = derive_for_chain(
                chain,
                test_phrase(chain),
                &path,
                None,
                None,
                None,
                true,
                true,
                true,
            )
            .unwrap();
            let actual =
                derive_from_private_key(chain, expected.private_key_hex.unwrap(), true, true)
                    .unwrap()
                    .unwrap();
            assert_eq!(actual.public_key_hex, expected.public_key_hex, "{chain}");
            if chain.mainnet_counterpart() == Chain::Cardano {
                // A raw extended key holds no stake key: it derives the
                // enterprise address of the payment key whose base address
                // the phrase derives.
                let payment = |address: &Option<String>| {
                    bech32::decode(address.as_deref().unwrap()).unwrap().1[1..29].to_vec()
                };
                assert_eq!(payment(&actual.address), payment(&expected.address));
                assert_ne!(actual.address, expected.address, "{chain}");
            } else {
                assert_eq!(actual.address, expected.address, "{chain}");
            }
            assert!(
                crate::send::flow::is_valid_send_address(chain, actual.address.unwrap()),
                "{chain}"
            );
            assert!(
                derive_from_private_key(chain, "00".repeat(31), true, true).is_err(),
                "{chain}"
            );
        }
    }

    /// The dispatcher and `Chain::derives_from_private_key` are one answer.
    ///
    /// This match decides *which algorithm*; the registry flag decides
    /// *whether one exists*, and the import flow reads the flag to know what to
    /// offer. Walking every chain keeps a chain from being offered, accepted
    /// and then producing no address.
    #[test]
    fn the_registry_flag_and_the_dispatcher_agree_on_every_chain() {
        const KEY: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
        let derives = |chain| {
            derive_from_private_key(
                chain,
                if chain.mainnet_counterpart() == crate::registry::Chain::Cardano {
                    let vector: serde_json::Value = serde_json::from_str(include_str!(
                        "../../tests/fixtures/cardano-emurgo-witness.json"
                    ))
                    .unwrap();
                    vector["privateKey"].as_str().unwrap().into()
                } else {
                    KEY.to_string()
                },
                true,
                false,
            )
            .expect("a valid key never errors")
            .and_then(|r| r.address)
        };

        for chain in crate::registry::Chain::all() {
            let claimed = chain.derives_from_private_key();
            let produced = derives(chain).is_some();
            assert_eq!(
                claimed,
                produced,
                "{}: the registry says derives_from_private_key = {claimed} and the \
                 dispatcher produced an address = {produced}",
                chain.str_id()
            );
        }
    }

    /// Every chain the registry lists derives through the one dispatcher.
    ///
    /// This is the property the 50 separate entry points could not state: that
    /// the set of derivable chains and the set the registry knows are the same.
    #[test]
    fn every_registry_chain_derives_through_one_call() {
        let mut missing = Vec::new();
        for chain in crate::registry::Chain::all() {
            // Every chain, with no `continue`: a chain the catalog gives no
            // path for answers "", and the arms that ignore the path do not
            // mind receiving one.
            let path = crate::derivation::path::default_path_from_catalog(chain)
                .expect("a registry chain always has an answer, even when it is none");
            let result = derive_for_chain(
                chain,
                crate::derivation::phrase::test_phrase(chain),
                &path,
                None,
                None,
                None,
                true,
                false,
                false,
            );
            match result {
                Ok(r) if r.address.is_some() => {}
                _ => missing.push(chain.str_id()),
            }
        }
        assert!(missing.is_empty(), "no address derived for: {missing:?}");
    }
}
