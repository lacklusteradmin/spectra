use super::*;
use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;
use ::bitcoin::hashes::Hash;

fn address(version: u8, hash: &[u8; 20]) -> String {
    let mut payload = vec![version];
    payload.extend(hash);
    bs58::encode(payload).with_check().into_string()
}

fn signed_transaction(chain: Chain, to: &str) -> ::bitcoin::Transaction {
    let (version, _) = chain.fixed_utxo_address_versions().unwrap();
    let sender = address(version, &[0x22; 20]);
    let inputs = vec![(
        "11".repeat(32),
        0,
        100_000,
        bitcoin_wire::p2pkh_script(&[0x22; 20]),
    )];
    let key = [1; 32];
    let result = match chain.mainnet_counterpart() {
        Chain::BitcoinCash => {
            bitcoin_cash::sign_bch_tx(chain, &inputs, to, 50_000, 1_000, &sender, &key, None)
        }
        Chain::BitcoinSV => {
            bitcoin_sv::sign_bsv_tx(chain, &inputs, to, 50_000, 1_000, &sender, &key, None)
        }
        Chain::BitcoinGold => {
            bitcoin_gold::sign_btg_tx(chain, &inputs, to, 50_000, 1_000, &sender, &key, None)
        }
        Chain::Dogecoin => {
            dogecoin::sign_doge_p2pkh(chain, &inputs, to, 50_000, 1_000, &sender, &key, None)
        }
        Chain::Dash => {
            dash::sign_dash_p2pkh(chain, &inputs, to, 50_000, 1_000, &sender, &key, None)
        }
        Chain::Litecoin => litecoin::sign_ltc_with_output_script(
            chain,
            &inputs,
            &parse_utxo_address(chain, to).unwrap().script_pubkey(),
            50_000,
            1_000,
            &sender,
            &key,
            None,
        ),
        _ => panic!("unexpected chain"),
    };
    ::bitcoin::consensus::deserialize(&result.unwrap()).unwrap()
}

#[test]
fn every_fixed_utxo_signer_pays_the_recipient_script_type() {
    for chain in Chain::all() {
        let Ok((p2pkh, p2sh)) = chain.fixed_utxo_address_versions() else {
            continue;
        };
        for (version, expected) in std::iter::once((
            p2pkh,
            ::bitcoin::ScriptBuf::new_p2pkh(&::bitcoin::PubkeyHash::from_byte_array([0x33; 20])),
        ))
        .chain(p2sh.iter().map(|version| {
            (
                *version,
                ::bitcoin::ScriptBuf::new_p2sh(&::bitcoin::ScriptHash::from_byte_array([0x33; 20])),
            )
        })) {
            let to = address(version, &[0x33; 20]);
            assert!(flow::is_valid_send_address(chain, to.clone()), "{chain}");
            let tx = signed_transaction(chain, &to);
            assert_eq!(tx.output[0].value.to_sat(), 50_000);
            assert_eq!(tx.output[0].script_pubkey, expected, "{chain}: {to}");
            assert_eq!(tx.output[1].value.to_sat(), 49_000);
            assert_eq!(
                tx.output[1].script_pubkey.as_bytes(),
                bitcoin_wire::p2pkh_script(&[0x22; 20])
            );
        }
    }
}

#[test]
fn cashaddr_and_legacy_addresses_pay_identical_outputs() {
    // https://github.com/bitcoincashorg/bitcoincash.org/blob/master/spec/cashaddr.md
    for (legacy, cash) in [
        (
            "1BpEi6DfDAUFd7GtittLSdBeYJvcoaVggu",
            "bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a",
        ),
        (
            "3CWFddi6m4ndiGyKqzYvsFYagqDLPVMTzC",
            "bitcoincash:ppm2qsznhks23z7629mms6s4cwef74vcwvn0h829pq",
        ),
    ] {
        let expected = signed_transaction(Chain::BitcoinCash, legacy).output;
        for address in [
            cash.to_string(),
            cash.to_uppercase(),
            cash.split_once(':').unwrap().1.to_string(),
        ] {
            assert!(flow::is_valid_send_address(
                Chain::BitcoinCash,
                address.clone()
            ));
            assert_eq!(
                signed_transaction(Chain::BitcoinCash, &address).output,
                expected
            );
            assert!(!flow::is_valid_send_address(
                Chain::BitcoinCashTestnet,
                address
            ));
        }
    }
    let testnet = "bchtest:pr6m7j9njldwwzlg9v7v53unlr4jkmx6eyvwc0uz5t";
    let tx = signed_transaction(Chain::BitcoinCashTestnet, testnet);
    assert!(tx.output[0].script_pubkey.is_p2sh());
}

#[test]
fn litecoin_and_bitcoin_gold_pay_witness_programs() {
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet, Chain::BitcoinGold] {
        let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap();
        for (version, program) in [
            (bech32::segwit::VERSION_0, vec![0x33; 20]),
            (bech32::segwit::VERSION_0, vec![0x44; 32]),
            (bech32::segwit::VERSION_1, vec![0x55; 32]),
        ] {
            let to = bech32::segwit::encode(hrp, version, &program).unwrap();
            if chain == Chain::BitcoinGold && version == bech32::segwit::VERSION_1 {
                assert!(!flow::is_valid_send_address(chain, to));
                continue;
            }
            assert!(flow::is_valid_send_address(chain, to.clone()));
            let tx = signed_transaction(chain, &to);
            let expected = ::bitcoin::ScriptBuf::new_witness_program(
                &::bitcoin::WitnessProgram::new(
                    ::bitcoin::WitnessVersion::try_from(version.to_u8()).unwrap(),
                    &program,
                )
                .unwrap(),
            );
            assert_eq!(tx.output[0].script_pubkey, expected, "{chain}: {to}");
        }
    }
}

#[test]
fn utxo_address_parser_refuses_wrong_network_and_malformed_cashaddr() {
    for (chain, other) in [
        (Chain::BitcoinCash, Chain::BitcoinCashTestnet),
        (Chain::BitcoinSV, Chain::BitcoinSVTestnet),
        (Chain::Dogecoin, Chain::DogecoinTestnet),
        (Chain::Litecoin, Chain::LitecoinTestnet),
        (Chain::Dash, Chain::DashTestnet),
    ] {
        for source in [chain, other] {
            let (version, p2sh) = source.fixed_utxo_address_versions().unwrap();
            for version in std::iter::once(version).chain(p2sh.iter().copied()) {
                let to = address(version, &[0x33; 20]);
                assert!(parse_utxo_address(source, &to).is_ok());
                assert!(
                    parse_utxo_address(if source == chain { other } else { chain }, &to).is_err()
                );
            }
        }
    }
    for invalid in [
        "bitcoincash:qPm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a",
        "bitcoincash:qpzry9x8gf2tvdw0s3jn54khce6mua7lcw20ayyn",
        "bitcoincash:q9adhakpwzztepkpwp5z0dq62m6u5v5xtyj7j3h2ws4mr9g0",
    ] {
        assert!(
            !flow::is_valid_send_address(Chain::BitcoinCash, invalid.into()),
            "{invalid}"
        );
    }
}

#[test]
fn fixed_utxo_witness_refuses_unsupported_versions_and_lengths() {
    for chain in [Chain::Litecoin, Chain::LitecoinTestnet, Chain::BitcoinGold] {
        let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap();
        for (version, length) in [
            (bech32::segwit::VERSION_1, 20),
            (bech32::Fe32::try_from(2u8).unwrap(), 32),
        ] {
            let to = bech32::segwit::encode(hrp, version, &vec![0x33; length]).unwrap();
            assert!(!flow::is_valid_send_address(chain, to.clone()));
            assert!(parse_utxo_address(chain, &to).is_err());
        }
        if chain == Chain::BitcoinGold {
            let to = bech32::segwit::encode(hrp, bech32::segwit::VERSION_1, &[0x33; 32]).unwrap();
            assert!(!flow::is_valid_send_address(chain, to.clone()));
            assert!(parse_utxo_address(chain, &to).is_err());
        }
    }
}
