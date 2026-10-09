//! Safe transactions against protocol-kit's (`safe-multisig.json`, from
//! scripts/generate-safe-multisig-vectors.cjs).

use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/safe-multisig.json")).unwrap()
}

fn text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn tx(vector: &serde_json::Value) -> SafeTx {
    SafeTransactionData {
        to: text(&vector["to"]),
        value: Number::Text(text(&vector["value"])),
        data: text(&vector["data"]),
        operation: vector["operation"].as_u64().unwrap() as u8,
        safe_tx_gas: Number::Text(text(&vector["safe_tx_gas"])),
        base_gas: Number::Text(text(&vector["base_gas"])),
        gas_price: Number::Text(text(&vector["gas_price"])),
        gas_token: text(&vector["gas_token"]),
        refund_receiver: text(&vector["refund_receiver"]),
        nonce: Number::Integer(vector["nonce"].as_u64().unwrap()),
    }
    .parse()
    .unwrap()
}

fn policy(fixture: &serde_json::Value, version: &str) -> SafePolicy {
    SafePolicy {
        version: version.into(),
        owners: fixture["owners"]
            .as_array()
            .unwrap()
            .iter()
            .map(|owner| owner["address"].as_str().unwrap().to_ascii_lowercase())
            .collect(),
        threshold: 2,
    }
}

fn bytes(value: &serde_json::Value) -> Vec<u8> {
    hex_bytes(value.as_str().unwrap()).unwrap()
}

/// Every transaction's domain separator, struct hash and safeTxHash are
/// protocol-kit's on both networks; each owner's ECDSA and eth_sign
/// signatures recover to that owner, and the ECDSA one is the signature
/// signing here gives.
#[test]
fn hashes_and_signatures_are_protocol_kits() {
    let fixture = fixture();
    let safe = parse_address(fixture["safe"].as_str().unwrap()).unwrap();
    let keys: Vec<Vec<u8>> = fixture["owners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|owner| bytes(&owner["private_key"]))
        .collect();
    for vector in fixture["transactions"].as_array().unwrap() {
        let tx = tx(vector);
        for network in vector["chains"].as_array().unwrap() {
            let chain_id = network["chain_id"].as_u64().unwrap();
            assert_eq!(
                SafeTx::domain_separator(chain_id, &safe).to_vec(),
                bytes(&network["domain_separator"])
            );
            assert_eq!(tx.struct_hash().to_vec(), bytes(&network["struct_hash"]));
            let hash = tx.hash(chain_id, &safe);
            assert_eq!(hash.to_vec(), bytes(&network["safe_tx_hash"]));
            let Some(signatures) = network["signatures"].as_array() else {
                continue;
            };
            for (signature, key) in signatures.iter().zip(&keys) {
                let owner = parse_address(signature["owner"].as_str().unwrap()).unwrap();
                for kind in ["ecdsa", "eth_sign"] {
                    assert_eq!(
                        recover_signer(&hash, &bytes(&signature[kind])).unwrap(),
                        owner,
                        "{kind}"
                    );
                }
                assert_eq!(sign(&hash, key).unwrap(), bytes(&signature["ecdsa"]));
            }
        }
    }
}

/// A session of two owners reviews complete, its signatures in owner
/// order as protocol-kit concatenates them, and its `execTransaction` call
/// is viem's.
#[test]
fn a_reviewed_session_executes_as_protocol_kit_encodes_it() {
    let fixture = fixture();
    let safe = parse_address(fixture["safe"].as_str().unwrap()).unwrap();
    let vector = &fixture["transactions"][0];
    let network = &vector["chains"][0];
    let chain_id = network["chain_id"].as_u64().unwrap();
    let policy = policy(&fixture, "1.4.1");
    let tx = tx(vector);
    let signatures: Vec<(EvmAddress, Vec<u8>)> = network["signatures"].as_array().unwrap()[..2]
        .iter()
        .map(|signature| {
            (
                parse_address(signature["owner"].as_str().unwrap()).unwrap(),
                bytes(&signature["ecdsa"]),
            )
        })
        .collect();
    let data = session_data(&safe, chain_id, &policy, &tx, &signatures);
    let reviewed = review(&policy, &safe, chain_id, &data).unwrap();
    assert!(reviewed.complete(&policy));
    assert_eq!(
        reviewed.signature_bytes(&policy),
        bytes(&network["signatures_p0_p1"])
    );
    assert_eq!(
        reviewed
            .tx
            .exec_calldata(&reviewed.signature_bytes(&policy)),
        bytes(&network["exec_transaction_p0_p1"])
    );
    // The interchange form reads back as the same session.
    let json = serde_json::to_string(&data).unwrap();
    let read: SafeSessionData = serde_json::from_str(&json).unwrap();
    assert_eq!(review(&policy, &safe, chain_id, &read).unwrap(), reviewed);
    // protocol-kit's own shape: nonce a number, amounts strings.
    let sdk: SafeSessionData = serde_json::from_value(serde_json::json!({
        "safe": fixture["safe"],
        "chainId": chain_id,
        "version": "1.4.1",
        "transaction": {
            "to": vector["to"], "value": vector["value"], "data": vector["data"],
            "operation": 0, "safeTxGas": "0", "baseGas": "0", "gasPrice": "0",
            "gasToken": vector["gas_token"], "refundReceiver": vector["refund_receiver"],
            "nonce": 7,
        },
        "signatures": [],
    }))
    .unwrap();
    assert_eq!(
        review(&policy, &safe, chain_id, &sdk).unwrap().hash,
        reviewed.hash
    );
}

/// Refused: another Safe, network or version, a delegate call, a signature
/// by someone who is no owner, one named for another owner, an owner twice,
/// and approved-hash or contract signatures.
#[test]
fn foreign_and_unverifiable_sessions_are_refused() {
    let fixture = fixture();
    let safe = parse_address(fixture["safe"].as_str().unwrap()).unwrap();
    let vector = &fixture["transactions"][0];
    let network = &vector["chains"][0];
    let chain_id = network["chain_id"].as_u64().unwrap();
    let policy = policy(&fixture, "1.4.1");
    let tx = tx(vector);
    let signed = |index: usize| {
        let signature = &network["signatures"][index];
        (
            parse_address(signature["owner"].as_str().unwrap()).unwrap(),
            bytes(&signature["ecdsa"]),
        )
    };
    let data = session_data(&safe, chain_id, &policy, &tx, &[signed(0)]);
    assert!(review(&policy, &safe, chain_id, &data).is_ok());
    assert!(review(&policy, &[0x11; 20], chain_id, &data).is_err());
    assert!(review(&policy, &safe, 1, &data).is_err());
    assert!(review(&self::policy(&fixture, "1.3.0"), &safe, chain_id, &data).is_err());
    let mut delegate = data.clone();
    delegate.transaction = self::tx(&fixture["transactions"][2]).data();
    delegate.signatures.clear();
    assert!(review(&policy, &safe, chain_id, &delegate).is_err());
    let mut stranger = policy.clone();
    stranger.owners.remove(0);
    assert!(review(&stranger, &safe, chain_id, &data).is_err());
    let mut misnamed = data.clone();
    misnamed.signatures[0].signer = address_text(&signed(1).0);
    assert!(review(&policy, &safe, chain_id, &misnamed).is_err());
    let twice = session_data(&safe, chain_id, &policy, &tx, &[signed(0), signed(0)]);
    assert!(review(&policy, &safe, chain_id, &twice).is_err());
    for v in [0u8, 1] {
        let mut signature = signed(0).1;
        signature[64] = v;
        assert!(recover_signer(&tx.hash(chain_id, &safe), &signature).is_err());
    }
}

/// Only an official proxy's code is one, and only the listed singletons
/// are official, each with its version.
#[test]
fn official_proxies_and_singletons() {
    let fixture = fixture();
    for (address, kind) in fixture["singleton_kinds"].as_object().unwrap() {
        assert_eq!(
            official_singleton(&parse_address(address).unwrap()),
            Some(kind["version"].as_str().unwrap()),
            "{address}"
        );
    }
    assert_eq!(official_singleton(&[0x11; 20]), None);
    let proxy_130 = hex::decode("608060405273ffffffffffffffffffffffffffffffffffffffff600054167fa619486e0000000000000000000000000000000000000000000000000000000060003514156050578060005260206000f35b3660008037600080366000845af43d6000803e60008114156070573d6000fd5b3d6000f3fea2646970667358221220d1429297349653a4918076d650332de1a1068c5f3e07c5c82360c277770b955264736f6c63430007060033").unwrap();
    assert!(is_official_proxy(&proxy_130));
    let mut tampered = proxy_130.clone();
    tampered[1] ^= 1;
    assert!(!is_official_proxy(&tampered));
    assert!(!is_official_proxy(&[]));
}
