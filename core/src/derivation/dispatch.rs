use crate::derivation::types::{BitcoinScriptType, DerivationResult, Wants};
use crate::error::SpectraBridgeError;

pub fn script_type_for_path(path: &str) -> BitcoinScriptType {
    let purpose = crate::derivation::bip32::parse_path(path)
        .ok()
        .and_then(|path| path.into_iter().next().copied())
        .map(crate::derivation::bip32::step_number);
    match purpose {
        Some(44) => BitcoinScriptType::P2pkh,
        Some(49) => BitcoinScriptType::P2shP2wpkh,
        Some(86) => BitcoinScriptType::P2tr,
        _ => BitcoinScriptType::P2wpkh,
    }
}

/// Derive `chain`'s wallet from a seed phrase along `derivation_path`.
///
/// Each family has one derive function and reads its network (address
/// versions, HRP, SS58 prefix, network byte) off `chain`, so the arms are
/// keyed on the mainnet a chain belongs to and its testnets fall out of
/// `mainnet_counterpart`. `script_type` applies to the UTXO family and
/// defaults to the one the path's purpose names.
pub fn derive_for_chain(
    chain: crate::registry::Chain,
    seed_phrase: &str,
    derivation_path: &str,
    passphrase: Option<&str>,
    hmac_key: Option<&str>,
    script_type: Option<BitcoinScriptType>,
    wants: Wants,
) -> Result<DerivationResult, SpectraBridgeError> {
    use crate::registry::Chain;

    use crate::derivation::{
        aptos, bitcoin, bittensor, cardano, evm, icp, monero, near, polkadot, solana, stellar, sui,
        ton, tron, xrp,
    };

    // A chain that derives without a path refuses one rather than deriving
    // its one key whatever the path says.
    if !chain.uses_derivation_path()
        && !chain.derives_along_junctions()
        && !derivation_path.trim().is_empty()
    {
        return Err(crate::derivation::error::DerivationError::refused(
            "%@ derives without a derivation path.",
            [chain.chain_display_name()],
        )
        .into());
    }
    let (phrase, path) = (seed_phrase, derivation_path);

    Ok(match chain.mainnet_counterpart() {
        Chain::Bitcoin
        | Chain::Litecoin
        | Chain::Peercoin
        | Chain::BitcoinCash
        | Chain::BitcoinSV
        | Chain::Dogecoin
        | Chain::Dash
        | Chain::BitcoinGold
        | Chain::Zcash
        | Chain::Decred
        | Chain::Kaspa => {
            let script = script_type.unwrap_or_else(|| script_type_for_path(path));
            bitcoin::derive_from_seed_phrase(chain, phrase, path, passphrase, script, wants)?
        }
        Chain::Tron => tron::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Solana => {
            solana::derive_from_seed_phrase(phrase, path, passphrase, hmac_key, wants)?
        }
        Chain::Stellar => {
            stellar::derive_from_seed_phrase(phrase, path, passphrase, hmac_key, wants)?
        }
        Chain::Xrp => xrp::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Cardano => cardano::derive_from_seed_phrase(chain, phrase, path, passphrase, wants)?,
        Chain::Sui => sui::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Aptos => aptos::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Ton => ton::derive_from_seed_phrase(chain, phrase, passphrase, wants)?,
        Chain::Icp => icp::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Near => near::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Polkadot => {
            polkadot::derive_from_seed_phrase(chain, phrase, path, passphrase, hmac_key, wants)?
        }
        Chain::Bittensor => bittensor::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        Chain::Monero => monero::derive_from_seed_phrase(chain, phrase, wants)?,
        // Every EVM chain derives the same address from the same path — there
        // is no chain-specific encoding.
        _ if chain.is_evm() => evm::derive_from_seed_phrase(phrase, path, passphrase, wants)?,
        _ => {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("unsupported chain: {}", chain.chain_display_name()).into(),
            });
        }
    })
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
                Wants::ALL,
            )
            .unwrap();
            let actual = derive_from_private_key(
                chain,
                expected.private_key_hex.as_deref().unwrap().to_string(),
                true,
                true,
            )
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
                Wants::ADDRESS,
            );
            match result {
                Ok(r) if r.address.is_some() => {}
                _ => missing.push(chain.str_id()),
            }
        }
        assert!(missing.is_empty(), "no address derived for: {missing:?}");
    }
}
