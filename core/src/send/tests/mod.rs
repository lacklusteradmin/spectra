//! Offline SDK oracles: derive real mnemonic keys, then use production signers.
//! Beside them, the outputs each UTXO signer pays and the send rules every
//! chain shares: addresses, preflight, fees and affordability.

use super::*;
use crate::derivation::utxo_address::parse_utxo_address;
use crate::registry::Chain;
use crate::{derivation::dispatch::derive_for_chain, send::keys::Ed25519Seed};
use ::bitcoin::hashes::Hash;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

/// The SPL Token program id. Production reads it off the mint's owner in the
/// RPC response, so the bytes are not a constant anywhere in the crate.
fn spl_token_program_id() -> [u8; 32] {
    crate::derivation::solana::decode_b58_32("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap()
}
fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/send-audit-vectors.json"
    ))
    .unwrap()
}
fn derived(
    chain: crate::registry::Chain,
    path: &str,
) -> crate::derivation::types::DerivationResult {
    derive_for_chain(
        chain,
        vectors()["mnemonic"].as_str().unwrap(),
        path,
        None,
        None,
        None,
        crate::derivation::types::Wants::ALL,
    )
    .unwrap()
}
fn key(result: &crate::derivation::types::DerivationResult) -> Ed25519Seed {
    Ed25519Seed::from_hex(result.private_key_hex.as_deref().unwrap()).unwrap()
}

#[test]
fn audit_aptos_mnemonic_address_and_signed_message_match_official_sdk() {
    let v = vectors();
    let expected = &v["aptos"];
    let d = derived(crate::registry::Chain::Aptos, "m/44'/637'/0'/0'/0'");
    assert_eq!(d.address.as_deref().unwrap(), expected["address"]);
    assert_eq!(d.public_key_hex.as_deref().unwrap(), expected["public_key"]);
    let prepared = aptos::prepare_transfer(
        d.address.as_deref().unwrap(),
        &format!("0x{}", "22".repeat(32)),
        123456789,
        7,
        100,
        10000,
        1800000000,
        1,
    )
    .unwrap();
    let (signed, hash) = prepared.sign(&key(&d)).unwrap();
    assert_eq!(hash, expected["transaction_hash"].as_str().unwrap());
    let signature = signed[signed.len() - 64..].to_vec();
    assert_eq!(
        hex::encode(&signature),
        expected["signature"].as_str().unwrap()
    );
    let message = hex::decode(expected["message"].as_str().unwrap()).unwrap();
    ed25519_dalek::VerifyingKey::from_bytes(&key(&d).public_key())
        .unwrap()
        .verify_strict(
            &message,
            &ed25519_dalek::Signature::from_slice(&signature).unwrap(),
        )
        .unwrap();
    let wrong = aptos::prepare_transfer(
        &format!("0x{}", "55".repeat(32)),
        "0x22",
        1,
        7,
        100,
        10000,
        1800000000,
        1,
    )
    .unwrap();
    assert!(wrong.sign(&key(&d)).is_err());
}
#[test]
fn audit_sui_local_ptb_and_intent_signature_match_official_sdk() {
    let v = vectors();
    let expected = &v["sui"];
    let d = derived(crate::registry::Chain::Sui, "m/44'/784'/0'/0'/0'");
    assert_eq!(d.address.as_deref().unwrap(), expected["address"]);
    assert_eq!(d.public_key_hex.as_deref().unwrap(), expected["public_key"]);
    let coins = [sui::GasCoin {
        id: [0x33; 32],
        version: 7,
        digest: [0; 32],
        balance: 200_000_000,
    }];
    let prepare = |from: &str, amount, gas| {
        sui::prepare_transfer(
            from,
            &format!("0x{}", "22".repeat(32)),
            amount,
            gas,
            1000,
            &coins,
        )
    };
    let prepared = prepare(d.address.as_deref().unwrap(), 123456789, 10_000_000).unwrap();
    assert_eq!(
        prepared.transaction_digest(),
        expected["transaction_digest"]
    );
    let (bytes, signature) = prepared.sign(&key(&d)).unwrap();
    assert_eq!(
        hex::encode(STANDARD.decode(bytes).unwrap()),
        expected["raw"]
    );
    assert_eq!(signature, expected["signature"]);
    assert!(
        prepare("0x11", 123456789, 10_000_000)
            .unwrap()
            .sign(&key(&d))
            .is_err()
    );
    assert!(prepare(d.address.as_deref().unwrap(), 195_000_000, 10_000_000).is_err());
    assert!(prepare(d.address.as_deref().unwrap(), u64::MAX, 1).is_err());
}
#[test]
fn audit_tron_local_native_and_token_bytes_match_tronweb() {
    let v = vectors();
    let t = &v["tron"];
    let d = derived(crate::registry::Chain::Tron, "m/44'/195'/0'/0/0");
    assert_eq!(d.address.as_deref().unwrap(), t["from"]);
    assert_eq!(d.private_key_hex.as_deref().unwrap().as_str(), t["key"]);
    let key = hex::decode(t["key"].as_str().unwrap()).unwrap();
    for token in [false, true] {
        let to = t["to"].as_str().unwrap();
        let transfer = if token {
            tron::Transfer::Token {
                contract: t["contract"].as_str().unwrap(),
                to,
                amount: 123456789,
                fee_limit: 100000000,
            }
        } else {
            tron::Transfer::Native {
                to,
                amount: 123456789,
            }
        };
        let mut id = [0x33; 32];
        id[..8].copy_from_slice(&7u64.to_be_bytes());
        let prepared = tron::prepare_transfer(
            t["from"].as_str().unwrap(),
            transfer,
            crate::api::tron_http::BlockReference {
                number: 7,
                id,
                timestamp_ms: 1800000000000,
            },
        )
        .unwrap();
        let (raw, signatures) = tron::split_signed(&prepared.sign(&key).unwrap());
        let expected = &t["transactions"][usize::from(token)];
        assert_eq!(
            hex::encode(&raw),
            expected["raw_data_hex"].as_str().unwrap()
        );
        assert_eq!(prepared.txid(), expected["txID"].as_str().unwrap());
        assert_eq!(
            hex::encode(&signatures[0]),
            expected["signature"][0]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
        );
        // The JSON a multisig session's signers exchange is TronWeb's.
        let (_, json) = crate::send::tron_multisig::decode(&raw)
            .unwrap()
            .prepared()
            .unwrap();
        assert_eq!(json["raw_data"], expected["raw_data"]);
    }
}
#[test]
fn audit_solana_mnemonic_native_and_spl_instructions_match_official_sdk() {
    let v = vectors();
    let expected = &v["solana"];
    let d = derived(
        crate::registry::Chain::Solana,
        expected["path"].as_str().unwrap(),
    );
    assert_eq!(d.address.as_deref().unwrap(), expected["address"]);
    let key = key(&d);
    let from = key.public_key();
    let to = [0x22; 32];
    let mint = [0x44; 32];
    let blockhash = expected["blockhash"].as_str().unwrap();
    let native = solana::build_sol_transfer(&from, &to, 123456789, blockhash, &key).unwrap();
    assert_eq!(hex::encode(native), expected["native"]);
    let source =
        solana::derive_associated_token_account(&from, &mint, &spl_token_program_id()).unwrap();
    let dest =
        solana::derive_associated_token_account(&to, &mint, &spl_token_program_id()).unwrap();
    assert_eq!(hex::encode(source), expected["source_ata"]);
    assert_eq!(hex::encode(dest), expected["dest_ata"]);
    let spl = solana::build_spl_transfer_checked(
        &from,
        &to,
        &mint,
        &source,
        &dest,
        &spl_token_program_id(),
        123456789,
        6,
        blockhash,
        &key,
    )
    .unwrap();
    // Account-key ordering need not match web3.js. Compare resolved instruction
    // accounts, privileges, data and blockhash, and independently verify the signature.
    assert_eq!(
        solana_semantics(&spl),
        solana_semantics(&hex::decode(expected["spl"].as_str().unwrap()).unwrap())
    );
    assert!(solana::build_sol_transfer(&[0x55; 32], &to, 1, blockhash, &key).is_err());
}
fn solana_semantics(tx: &[u8]) -> Value {
    assert_eq!(tx[0], 1);
    let message = &tx[65..];
    let header = &message[..3];
    let count = message[3] as usize;
    assert!(count < 128);
    let (accounts, _) = message[4..4 + count * 32].as_chunks::<32>();
    ed25519_dalek::VerifyingKey::from_bytes(&accounts[0])
        .unwrap()
        .verify_strict(
            message,
            &ed25519_dalek::Signature::from_slice(&tx[1..65]).unwrap(),
        )
        .unwrap();
    let mut offset = 4 + count * 32;
    let hash = hex::encode(&message[offset..offset + 32]);
    offset += 32;
    let instructions = message[offset] as usize;
    offset += 1;
    let mut decoded = Vec::new();
    for _ in 0..instructions {
        let program = hex::encode(accounts[message[offset] as usize]);
        offset += 1;
        let count = message[offset] as usize;
        offset += 1;
        let accts = message[offset..offset + count]
            .iter()
            .map(|index| {
                let i = *index as usize;
                let signed = i < header[0] as usize;
                let writable = if signed {
                    i < (header[0] - header[1]) as usize
                } else {
                    i < accounts.len() - header[2] as usize
                };
                serde_json::json!([hex::encode(accounts[i]), signed, writable])
            })
            .collect::<Vec<_>>();
        offset += count;
        let len = message[offset] as usize;
        assert!(len < 128);
        offset += 1;
        let data = hex::encode(&message[offset..offset + len]);
        offset += len;
        decoded.push(serde_json::json!([program, accts, data]));
    }
    assert_eq!(offset, message.len());
    serde_json::json!([hash, decoded])
}

#[test]
fn audit_local_builders_refuse_unfunded_or_mismatched_inputs() {
    let fixture = vectors();
    let t = &fixture["tron"];
    let mut id = [0x33; 32];
    id[..8].copy_from_slice(&7u64.to_be_bytes());
    let prepare = |amount| {
        tron::prepare_transfer(
            t["from"].as_str().unwrap(),
            tron::Transfer::Native {
                to: t["to"].as_str().unwrap(),
                amount,
            },
            crate::api::tron_http::BlockReference {
                number: 7,
                id,
                timestamp_ms: 1800000000000,
            },
        )
    };
    assert!(prepare(0).is_err());
    assert!(prepare(u64::MAX).is_err());
    assert!(prepare(1).unwrap().sign(&[1; 32]).is_err());
    let d = derived(crate::registry::Chain::Sui, "m/44'/784'/0'/0'/0'");
    let coins = [
        sui::GasCoin {
            id: [0x33; 32],
            version: 7,
            digest: [0; 32],
            balance: 200_000_000,
        },
        sui::GasCoin {
            id: [0x33; 32],
            version: 7,
            digest: [0; 32],
            balance: 200_000_000,
        },
    ];
    assert!(
        sui::prepare_transfer(
            d.address.as_deref().unwrap(),
            "0x22",
            1,
            10000,
            1000,
            &coins
        )
        .is_err()
    );
}

#[tokio::test]
async fn audit_failed_sui_execution_is_not_a_successful_send() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any};
    let server = MockServer::start().await;
    Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"digest":"11111111111111111111111111111111","effects":{"status":{"status":"failure","error":"InsufficientGas"}}}}))).mount(&server).await;
    let client = crate::api::sui_json_rpc::SuiClient::new(std::sync::Arc::new(vec![server.uri()]));
    assert!(
        client
            .execute_signed_tx("test-payload", "test-signature")
            .await
            .is_err()
    );
}

#[test]
fn audit_solana_self_transfers_merge_account_privileges() {
    let v = vectors();
    let d = derived(
        crate::registry::Chain::Solana,
        v["solana"]["path"].as_str().unwrap(),
    );
    let key = key(&d);
    let owner = key.public_key();
    let hash = v["solana"]["blockhash"].as_str().unwrap();
    let native = solana::build_sol_transfer(&owner, &owner, 1, hash, &key).unwrap();
    assert_eq!(native[68], 2, "payer and system program only");
    let decoded = solana_semantics(&native);
    assert_eq!(decoded[1][0][1][0], decoded[1][0][1][1]);
    let mint = [0x44; 32];
    let ata =
        solana::derive_associated_token_account(&owner, &mint, &spl_token_program_id()).unwrap();
    let spl = solana::build_spl_transfer_checked(
        &owner,
        &owner,
        &mint,
        &ata,
        &ata,
        &spl_token_program_id(),
        1,
        6,
        hash,
        &key,
    )
    .unwrap();
    assert_eq!(spl[68], 6, "owner and ATA aliases must each merge");
    let decoded = solana_semantics(&spl);
    assert_eq!(decoded[1][1][1][0], decoded[1][1][1][2]);
    assert_eq!(
        decoded[1][0][1][0], decoded[1][0][1][2],
        "ATA owner aliases signer, including writable privilege"
    );
    for tx in [native, spl] {
        let (keys, _) = tx[69..69 + tx[68] as usize * 32].as_chunks::<32>();
        assert_eq!(
            keys.len(),
            keys.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }
}

fn address(version: u8, hash: &[u8; 20]) -> String {
    let mut payload = vec![version];
    payload.extend(hash);
    bs58::encode(payload).with_check().into_string()
}

fn signed_transaction(chain: Chain, to: &str) -> ::bitcoin::Transaction {
    let (version, _) = chain.fixed_utxo_address_versions().unwrap();
    let key = [1; 32];
    let hash = sender_hash(&key);
    let sender = address(version, &hash);
    let inputs = vec![crate::api::utxo::UtxoInput::fixture(
        &"11".repeat(32),
        0,
        100_000,
        bitcoin_wire::p2pkh_script(&hash),
    )];
    let recipient = parse_utxo_address(chain, to).unwrap().script_pubkey();
    let outputs = vec![
        (recipient.clone(), 50_000),
        (bitcoin_wire::p2pkh_script(&hash), 49_000),
    ];
    let legacy = |fork_id| {
        legacy_p2pkh::sign(
            &[legacy_p2pkh::LegacyInput {
                utxo: &inputs[0],
                private_key: &key,
            }],
            &outputs,
            fork_id,
        )
    };
    let result = match chain.mainnet_counterpart() {
        Chain::Bitcoin | Chain::Qtum | Chain::FractalBitcoin | Chain::Digibyte => {
            bitcoin::sign_inputs(
                &[bitcoin::SigningInput {
                    utxo: &inputs[0],
                    private_key: &key,
                }],
                &outputs,
                ::bitcoin::transaction::Version::TWO,
                ::bitcoin::Sequence::ENABLE_RBF_NO_LOCKTIME,
            )
        }
        Chain::BitcoinCash | Chain::BitcoinSV | Chain::BitcoinGold => {
            legacy(Some(chain.sighash_fork_id().unwrap()))
        }
        Chain::Dogecoin | Chain::Dash => legacy(None),
        Chain::Litecoin => litecoin::sign_ltc_with_output_script(
            chain, &inputs, &recipient, 50_000, 1_000, &sender, &key,
        ),
        Chain::Peercoin => {
            peercoin::sign_peercoin_tx(chain, &inputs, to, 50_000, 10_000, &sender, &key)
        }
        _ => panic!("unexpected chain"),
    };
    ::bitcoin::consensus::deserialize(&result.unwrap()).unwrap()
}

fn sender_hash(key: &[u8; 32]) -> [u8; 20] {
    let secp = secp256k1::Secp256k1::new();
    let public = secp256k1::PublicKey::from_secret_key(
        &secp,
        &secp256k1::SecretKey::from_slice(key).unwrap(),
    );
    ::bitcoin::hashes::hash160::Hash::hash(&public.serialize()).to_byte_array()
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
            let change = if chain.mainnet_counterpart() == Chain::Peercoin {
                40_000
            } else {
                49_000
            };
            assert_eq!(tx.output[1].value.to_sat(), change);
            assert_eq!(
                tx.output[1].script_pubkey.as_bytes(),
                bitcoin_wire::p2pkh_script(&sender_hash(&[1; 32]))
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
fn witness_utxo_chains_pay_supported_witness_programs() {
    for chain in [
        Chain::Bitcoin,
        Chain::BitcoinSignet,
        Chain::Litecoin,
        Chain::LitecoinTestnet,
        Chain::BitcoinGold,
        Chain::Peercoin,
        Chain::PeercoinTestnet,
    ] {
        let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap();
        for (version, program) in [
            (bech32::segwit::VERSION_0, vec![0x33; 20]),
            (bech32::segwit::VERSION_0, vec![0x44; 32]),
            (bech32::segwit::VERSION_1, vec![0x55; 32]),
        ] {
            let to = bech32::segwit::encode(hrp, version, &program).unwrap();
            if !chain.fixed_utxo_supports_witness(version.to_u8(), program.len()) {
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
        (Chain::Bitcoin, Chain::BitcoinTestnet),
        (Chain::BitcoinCash, Chain::BitcoinCashTestnet),
        (Chain::BitcoinSV, Chain::BitcoinSVTestnet),
        (Chain::Dogecoin, Chain::DogecoinTestnet),
        (Chain::Litecoin, Chain::LitecoinTestnet),
        (Chain::Dash, Chain::DashTestnet),
        (Chain::Peercoin, Chain::PeercoinTestnet),
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
    for chain in [
        Chain::Litecoin,
        Chain::LitecoinTestnet,
        Chain::BitcoinGold,
        Chain::Peercoin,
        Chain::PeercoinTestnet,
    ] {
        let hrp = bech32::Hrp::parse(chain.fixed_utxo_segwit_hrp().unwrap()).unwrap();
        for (version, length) in [
            (bech32::segwit::VERSION_1, 20),
            (bech32::Fe32::try_from(2u8).unwrap(), 32),
        ] {
            let to = bech32::segwit::encode(hrp, version, &vec![0x33; length]).unwrap();
            assert!(!flow::is_valid_send_address(chain, to.clone()));
            assert!(parse_utxo_address(chain, &to).is_err());
        }
        if !chain.fixed_utxo_supports_witness(1, 32) {
            let to = bech32::segwit::encode(hrp, bech32::segwit::VERSION_1, &[0x33; 32]).unwrap();
            assert!(!flow::is_valid_send_address(chain, to.clone()));
            assert!(parse_utxo_address(chain, &to).is_err());
        }
    }
}

/// Every chain sends its native asset, so every chain can name a fee: its
/// own preview path, a shared-path shape, or a fallback.
#[test]
fn every_chain_has_a_send_preview() {
    for chain in Chain::all() {
        assert!(
            chain.has_send_preview(),
            "{} sends but the screen would say it has no network preview",
            chain.str_id()
        );
    }
}

/// Every EVM chain folds its addresses the same way, mainnets and testnets
/// alike. Seven of twenty-three were named before, so an address book entry
/// on the other sixteen kept whatever case was typed.
#[test]
fn every_evm_chain_lowercases_its_addresses() {
    use crate::registry::Chain;
    use crate::send::flow::normalize_address;

    let checksummed = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e";
    for chain in Chain::all().filter(|c| c.is_evm()) {
        assert_eq!(
            normalize_address(chain, checksummed),
            checksummed.to_lowercase(),
            "{} left a mixed-case address as typed",
            chain.str_id()
        );
    }
    // XDC's own spelling is stored and sent as the `0x` address.
    let xdc = "xdc742d35Cc6634C0532925a3b844Bc454e4438f44e";
    assert_eq!(
        normalize_address(Chain::Xdc, xdc),
        checksummed.to_lowercase()
    );
    assert_eq!(normalize_address(Chain::Ethereum, xdc), xdc);
}

/// A send's destination is judged with its EIP-55 checksum. Normalizing
/// lowercased first, so a mixed-case address with a mistyped character — one
/// letter's case changed is what that looks like — passed as a destination
/// and as an address book entry; it stays as typed now, and is refused. The
/// one-case spellings carry no checksum and still pass.
#[test]
fn a_destination_with_a_broken_checksum_is_refused_on_every_evm_chain() {
    use crate::registry::Chain;
    use crate::send::flow::{is_valid_send_address, normalize_address};

    let broken = "0x742d35cC6634C0532925a3b844Bc454e4438f44e";
    for chain in Chain::all().filter(|c| c.is_evm()) {
        assert!(!is_valid_send_address(chain, broken.into()), "{chain}");
        assert_eq!(normalize_address(chain, broken), broken, "{chain}");
        for unchecked in [
            "0x742d35cc6634c0532925a3b844bc454e4438f44e",
            "0x742D35CC6634C0532925A3B844BC454E4438F44E",
            " 0x742d35Cc6634C0532925a3b844Bc454e4438f44e ",
        ] {
            assert!(
                is_valid_send_address(chain, unchecked.into()),
                "{chain} {unchecked}"
            );
        }
    }
    assert!(!is_valid_send_address(
        Chain::Xdc,
        "xdc742d35cC6634C0532925a3b844Bc454e4438f44e".into()
    ));
}

/// And a chain whose addresses are case-significant is left alone. Folding
/// a Bitcoin or Solana address destroys it.
#[test]
fn case_significant_chains_are_untouched() {
    use crate::registry::{AddressNormalization, Chain};
    use crate::send::flow::normalize_address;

    for chain in Chain::all() {
        if chain.address_normalization() != AddressNormalization::None {
            continue;
        }
        let sample = "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2";
        assert_eq!(
            normalize_address(chain, sample),
            sample,
            "{} altered an address whose case is significant",
            chain.str_id()
        );
    }
}

#[test]
fn stellar_addresses_remain_canonical_uppercase_through_sender_resolution() {
    let address = "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ";
    for chain in [
        crate::registry::Chain::Stellar,
        crate::registry::Chain::StellarTestnet,
    ] {
        assert_eq!(
            crate::send::flow::normalize_address(chain, address),
            address
        );
        assert_eq!(
            crate::send::flow::normalize_address(chain, &address.to_ascii_lowercase()),
            address
        );
        assert!(crate::send::flow::is_valid_send_address(
            chain,
            address.to_string()
        ));
        let validated =
            crate::validation::address::validate_address(chain, &address.to_ascii_lowercase());
        assert_eq!(validated.normalized_value.as_deref(), Some(address));
        assert!(
            crate::derivation::stellar::decode_stellar_address(&(address.to_string() + "A"))
                .is_err()
        );
    }
}

/// A destination's first letters say nothing about its network: an ENS
/// name beginning `a`, `d` or `t` and a Solana address beginning `T` raise
/// no warning of their shape, and every EVM chain but Ethereum flags the
/// ENS name.
#[test]
fn destination_warnings_do_not_guess_a_network_from_letters() {
    use crate::registry::Chain;
    use crate::send::flow::{HighRiskSendRequest, evaluate_high_risk_send_reasons};

    let raised = |chain: Chain, input: &str, address: &str| -> Vec<String> {
        evaluate_high_risk_send_reasons(HighRiskSendRequest {
            chain_id: chain,
            symbol: chain.coin_symbol().to_string(),
            amount: "1".into(),
            holding_amount: "100".into(),
            destination_address: address.to_string(),
            destination_input: input.to_string(),
            used_ens_resolution: false,
            wallet_chain_id: chain,
            address_book_entries: Vec::new(),
            tx_addresses: Vec::new(),
        })
        .into_iter()
        .map(|w| w.code().to_string())
        // A destination never sent to is new on every chain.
        .filter(|code| code != "new_address")
        .collect()
    };

    let resolved = format!("0x{}", "22".repeat(20));
    for chain in Chain::all().filter(|c| c.is_evm()) {
        for name in ["vitalik.eth", "alice.eth", "dave.eth", "tim.eth"] {
            let expected: &[&str] = if chain.mainnet_counterpart() == Chain::Ethereum {
                &[]
            } else {
                &["ens_off_ethereum"]
            };
            assert_eq!(
                raised(chain, name, &resolved),
                expected,
                "{} {name}",
                chain.str_id()
            );
        }
    }
    let solana = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    assert!(raised(Chain::Solana, solana, solana).is_empty());
}

fn asset(chain: Chain, symbol: &str, kind: SendAssetKind) -> SendAsset {
    SendAsset {
        chain,
        symbol: symbol.into(),
        kind,
    }
}

#[test]
fn rejects_zero_amount_for_non_evm_native_sends() {
    let btc = asset(Chain::Bitcoin, "BTC", SendAssetKind::Native);
    let error = validate_send_preflight(true, Some(&btc), "1", "bc1qdestination", "0")
        .expect_err("bitcoin zero-value sends should be rejected in preflight");
    assert_eq!(error.to_string(), "Enter a valid amount");
}

#[test]
fn preserves_zero_amount_for_native_evm_preflight() {
    let eth = asset(Chain::Ethereum, "ETH", SendAssetKind::Native);
    let plan = validate_send_preflight(true, Some(&eth), "1", "0xabc", "0")
        .expect("native EVM zero-value sends remain allowed");
    assert_eq!(plan.amount, "0");
}

/// A token nothing tracks is refused, whatever chain it is on. The EVM
/// and Tron routes used to let one through with no contract to send.
#[test]
fn an_untracked_token_is_refused() {
    let usdt = asset(Chain::Ethereum, "USDT", SendAssetKind::UntrackedToken);
    let error = validate_send_preflight(true, Some(&usdt), "10", "0xabc", "1")
        .expect_err("an untracked token has no contract to send");
    assert_eq!(error.to_string(), "USDT transfers are not enabled yet.");
}

#[test]
fn send_execution_request_scrubs_secret_fields() {
    let mut request = SendExecutionRequest {
        token_standard: None,
        chain_id: crate::registry::Chain::Ethereum,
        wallet_id: "w".into(),
        password: Some("password".into()),
        to_address: "0xto".into(),
        amount_str: "1".into(),
        contract_address: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        memo: None,
    };
    request.zeroize_sensitive_fields();
    assert_eq!(request.password.as_deref(), Some(""));
}

mod the_generic_submit_chains_can_name_a_fee {
    /// Zcash, Bitcoin Gold, Decred, Kaspa and Dash take the shared submit path
    /// with no shared-path preview, so their fee comes from the fallback.
    #[test]
    fn each_has_a_fee_fallback() {
        use crate::registry::Chain;
        for chain in [
            Chain::Zcash,
            Chain::BitcoinGold,
            Chain::Decred,
            Chain::Kaspa,
            Chain::Dash,
        ] {
            let name = chain.str_id();
            assert!(
                chain.uses_generic_send_submit(),
                "{name} must take the shared submit path"
            );
            assert!(
                chain.send_execution_shape().fee_fallback.is_some(),
                "{name} needs a fee fallback"
            );
            assert!(!chain.has_shared_send_preview(), "{name}");
        }
    }
}

mod token_decimals_are_not_assumed {
    /// No chain's tokens all share one decimal count: Tron's USDT has six, and
    /// BTT, TUSD, USD1 and USDD have eighteen.
    #[test]
    fn a_chain_can_host_tokens_of_different_decimals() {
        use std::collections::{HashMap, HashSet};
        let mut by_chain: HashMap<crate::registry::Chain, HashSet<u32>> = HashMap::new();
        for token in crate::catalog::tokens::list_token_deployments(None) {
            by_chain
                .entry(token.chain_id)
                .or_default()
                .insert(token.decimals);
        }
        let mixed: Vec<_> = by_chain
            .iter()
            .filter(|(_, d)| d.len() > 1)
            .map(|(c, d)| {
                let mut v: Vec<_> = d.iter().copied().collect();
                v.sort_unstable();
                (*c, v)
            })
            .collect();
        assert!(
            !mixed.is_empty(),
            "if this ever holds, the assumption a caller could make is at least true"
        );
        let tron = by_chain
            .get(&crate::registry::Chain::Tron)
            .expect("tron hosts tokens");
        assert!(
            tron.len() > 1,
            "tron's tokens are all {tron:?} decimals — the hardcoded 6 would have been harmless"
        );
    }
}

mod affordability_reads_the_chain_rather_than_the_caller {
    use super::{SendAffordability, SendAffordabilityInput, send_affordability};

    fn input(chain: crate::registry::Chain, symbol: &str) -> SendAffordabilityInput {
        SendAffordabilityInput {
            pays_own_fee: chain.fee_holding_template().symbol == symbol,
            chain_id: chain,
            symbol: symbol.to_string(),
            amount: "1".into(),
            network_fee: "0.5".into(),
            holding_balance: "1.2".into(),
            gas_balance: Some("0.1".into()),
        }
    }

    /// The four Swift call sites each decided "is this the chain's own asset"
    /// themselves, and spelled it four ways. Naming the chain is enough, and
    /// the answer comes from the same column a balance is denominated in.
    #[test]
    fn a_governance_token_is_not_the_asset_the_fee_comes_out_of() {
        // Arbitrum charges gas in ETH. A caller that took ARB for the native
        // asset would check the fee against the ARB balance — the same pair,
        // and the same mistake, the destination probe's five-symbol list made.
        assert_eq!(
            send_affordability(input(crate::registry::Chain::Arbitrum, "ARB")),
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol: "ETH".to_string(),
                fee: "0.5".to_string(),
                chain_id: crate::registry::Chain::Arbitrum,
            }
        );
        // VeChainThor's coin pays no fee: VTHO does.
        assert_eq!(
            send_affordability(input(crate::registry::Chain::Vechain, "VET")),
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol: "VTHO".to_string(),
                fee: "0.5".to_string(),
                chain_id: crate::registry::Chain::Vechain,
            }
        );
        assert_eq!(
            send_affordability(input(crate::registry::Chain::Vechain, "VTHO")),
            SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: "VTHO".to_string(),
                required: "1.5".to_string(),
            }
        );
        // Tron's own asset, which its Swift caller matched with a literal.
        assert_eq!(
            send_affordability(input(crate::registry::Chain::Tron, "TRX")),
            SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: "TRX".to_string(),
                required: "1.5".to_string(),
            }
        );
    }

    #[test]
    fn the_fee_is_quoted_to_the_chains_own_decimals() {
        let mut btc = input(crate::registry::Chain::Bitcoin, "BTC");
        btc.amount = "2".into();
        assert_eq!(
            send_affordability(btc),
            SendAffordability::AmountPlusFeeExceedsBalance {
                symbol: "BTC".to_string(),
                required: "2.5".to_string(),
            }
        );
    }

    #[test]
    fn a_token_send_is_refused_on_its_own_balance_before_the_fee_is_looked_at() {
        let mut over = input(crate::registry::Chain::Ethereum, "USDC");
        over.amount = "5".into();
        assert_eq!(
            send_affordability(over),
            SendAffordability::AmountExceedsBalance {
                symbol: "USDC".to_string()
            }
        );
    }

    #[test]
    fn both_fitting_is_affordable() {
        let mut ok = input(crate::registry::Chain::Ethereum, "USDC");
        ok.gas_balance = Some("2".into());
        assert_eq!(send_affordability(ok), SendAffordability::Affordable);

        let mut native = input(crate::registry::Chain::Ethereum, "ETH");
        native.holding_balance = "10".into();
        assert_eq!(send_affordability(native), SendAffordability::Affordable);
    }
}
