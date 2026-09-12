//! Offline SDK oracles: derive real mnemonic keys, then use production signers.
use super::*;
use crate::{derivation::dispatch::derive_for_chain_name, send::keys::Ed25519Seed};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
fn vectors() -> Value {
    serde_json::from_str(include_str!("../../../testdata/send-audit-vectors.json")).unwrap()
}
fn derived(chain: &str, path: &str) -> crate::derivation::types::DerivationResult {
    derive_for_chain_name(
        chain,
        vectors()["mnemonic"].as_str().unwrap(),
        path,
        None,
        None,
        None,
        true,
        true,
        true,
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
    let d = derived("Aptos", "m/44'/637'/0'/0'/0'");
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
    let signed: Value = serde_json::from_str(&prepared.sign(&key(&d)).unwrap()).unwrap();
    assert_eq!(
        signed["signature"]["signature"],
        format!("0x{}", expected["signature"].as_str().unwrap())
    );
    let message = hex::decode(expected["message"].as_str().unwrap()).unwrap();
    let signature = hex::decode(
        signed["signature"]["signature"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
    )
    .unwrap();
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
    let d = derived("Sui", "m/44'/784'/0'/0'/0'");
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
    let (bytes, signature) = prepare(d.address.as_deref().unwrap(), 123456789, 10_000_000)
        .unwrap()
        .sign(&key(&d))
        .unwrap();
    assert_eq!(
        hex::encode(STANDARD.decode(bytes).unwrap()),
        expected["raw"]
    );
    assert_eq!(signature, expected["signature"]);
    assert!(prepare("0x11", 123456789, 10_000_000)
        .unwrap()
        .sign(&key(&d))
        .is_err());
    assert!(prepare(d.address.as_deref().unwrap(), 195_000_000, 10_000_000).is_err());
    assert!(prepare(d.address.as_deref().unwrap(), u64::MAX, 1).is_err());
}
#[test]
fn audit_tron_local_native_and_token_bytes_match_tronweb() {
    let v = vectors();
    let t = &v["tron"];
    let d = derived("Tron", "m/44'/195'/0'/0/0");
    assert_eq!(d.address.as_deref().unwrap(), t["from"]);
    assert_eq!(d.private_key_hex.as_deref().unwrap(), t["key"]);
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
            tron::BlockReference {
                number: 7,
                id,
                timestamp_ms: 1800000000000,
            },
        )
        .unwrap();
        let actual: Value = serde_json::from_str(&prepared.sign(&key).unwrap()).unwrap();
        let expected = &t["transactions"][usize::from(token)];
        assert_eq!(actual["raw_data_hex"], expected["raw_data_hex"]);
        assert_eq!(actual["txID"], expected["txID"]);
        assert_eq!(
            actual["signature"][0].as_str().unwrap(),
            expected["signature"][0]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
        );
        assert_eq!(actual["raw_data"], expected["raw_data"]);
    }
}
#[test]
fn audit_solana_mnemonic_native_and_spl_instructions_match_official_sdk() {
    let v = vectors();
    let expected = &v["solana"];
    let d = derived("Solana", expected["path"].as_str().unwrap());
    assert_eq!(d.address.as_deref().unwrap(), expected["address"]);
    let key = key(&d);
    let from = key.public_key();
    let to = [0x22; 32];
    let mint = [0x44; 32];
    let blockhash = expected["blockhash"].as_str().unwrap();
    let native = solana::build_sol_transfer(&from, &to, 123456789, blockhash, &key).unwrap();
    assert_eq!(hex::encode(native), expected["native"]);
    let source =
        solana::derive_associated_token_account(&from, &mint, &solana::SPL_TOKEN_PROGRAM_ID)
            .unwrap();
    let dest =
        solana::derive_associated_token_account(&to, &mint, &solana::SPL_TOKEN_PROGRAM_ID).unwrap();
    assert_eq!(hex::encode(source), expected["source_ata"]);
    assert_eq!(hex::encode(dest), expected["dest_ata"]);
    let spl = solana::build_spl_transfer_checked(
        &from,
        &to,
        &mint,
        &source,
        &dest,
        &solana::SPL_TOKEN_PROGRAM_ID,
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
    let accounts = message[4..4 + count * 32]
        .chunks_exact(32)
        .collect::<Vec<_>>();
    ed25519_dalek::VerifyingKey::from_bytes(accounts[0].try_into().unwrap())
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
            tron::BlockReference {
                number: 7,
                id,
                timestamp_ms: 1800000000000,
            },
        )
    };
    assert!(prepare(0).is_err());
    assert!(prepare(u64::MAX).is_err());
    assert!(prepare(1).unwrap().sign(&[1; 32]).is_err());
    let d = derived("Sui", "m/44'/784'/0'/0'/0'");
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
    assert!(sui::prepare_transfer(
        d.address.as_deref().unwrap(),
        "0x22",
        1,
        10000,
        1000,
        &coins
    )
    .is_err());
}

#[tokio::test]
async fn audit_failed_sui_execution_is_not_a_successful_send() {
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(any()).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"jsonrpc":"2.0","id":1,"result":{"digest":"11111111111111111111111111111111","effects":{"status":{"status":"failure","error":"InsufficientGas"}}}}))).mount(&server).await;
    let client = crate::fetch::chains::sui::SuiClient::new(std::sync::Arc::new(vec![server.uri()]));
    assert!(client
        .execute_signed_tx("test-payload", "test-signature")
        .await
        .is_err());
}

#[test]
fn audit_solana_self_transfers_merge_account_privileges() {
    let v = vectors();
    let d = derived("Solana", v["solana"]["path"].as_str().unwrap());
    let key = key(&d);
    let owner = key.public_key();
    let hash = v["solana"]["blockhash"].as_str().unwrap();
    let native = solana::build_sol_transfer(&owner, &owner, 1, hash, &key).unwrap();
    assert_eq!(native[68], 2, "payer and system program only");
    let decoded = solana_semantics(&native);
    assert_eq!(decoded[1][0][1][0], decoded[1][0][1][1]);
    let mint = [0x44; 32];
    let ata = solana::derive_associated_token_account(&owner, &mint, &solana::SPL_TOKEN_PROGRAM_ID)
        .unwrap();
    let spl = solana::build_spl_transfer_checked(
        &owner,
        &owner,
        &mint,
        &ata,
        &ata,
        &solana::SPL_TOKEN_PROGRAM_ID,
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
        let keys = tx[69..69 + tx[68] as usize * 32]
            .chunks_exact(32)
            .collect::<Vec<_>>();
        assert_eq!(
            keys.len(),
            keys.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }
}
