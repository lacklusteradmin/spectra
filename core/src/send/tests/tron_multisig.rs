//! Tron permissions and multi-signed transactions against TronWeb's
//! (`tron-multisig.json`, from scripts/generate-tron-multisig-vectors.cjs).

use super::*;
use crate::api::tron_http::BlockReference;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/tron-multisig.json")).unwrap()
}

fn block(fixture: &serde_json::Value) -> BlockReference {
    BlockReference {
        number: fixture["block"]["number"].as_u64().unwrap(),
        id: hex::decode(fixture["block"]["id"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
        timestamp_ms: fixture["block"]["timestamp_ms"].as_u64().unwrap(),
    }
}

fn permissions(fixture: &serde_json::Value) -> TronPermissions {
    crate::api::tron_http::account_permissions(
        &fixture["getaccount"],
        fixture["account"].as_str().unwrap(),
    )
    .unwrap()
}

/// Each transaction encodes as TronWeb's under its permission and
/// lifetime, decodes back to itself, and each key's signature is
/// TronWeb's and recovers to that key's account.
#[test]
fn transactions_and_signatures_are_tronwebs() {
    let fixture = fixture();
    let owner = fixture["account"].as_str().unwrap();
    let recipient = fixture["recipient"].as_str().unwrap();
    let token = fixture["trc20_contract"].as_str().unwrap();
    let keys: Vec<Vec<u8>> = fixture["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| hex::decode(key["private_key"].as_str().unwrap()).unwrap())
        .collect();
    for vector in fixture["transactions"].as_array().unwrap() {
        let raw_data = &vector["raw_data"];
        let lifetime = raw_data["expiration"].as_u64().unwrap() - block(&fixture).timestamp_ms;
        let transfer = match raw_data["fee_limit"].as_u64() {
            Some(fee_limit) => Transfer::Token {
                contract: token,
                to: recipient,
                amount: 1_234_567,
                fee_limit,
            },
            None => Transfer::Native {
                to: recipient,
                amount: 1_000_000,
            },
        };
        let prepared = crate::send::tron::prepare_transfer_under(
            owner,
            transfer,
            block(&fixture),
            vector["permission_id"].as_u64().unwrap() as u8,
            lifetime,
        )
        .unwrap();
        assert_eq!(
            hex::encode(&prepared.raw),
            vector["raw_data_hex"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
        assert_eq!(prepared.body["txID"], vector["txID"]);
        assert_eq!(
            prepared.body["raw_data"]["contract"][0].get("Permission_id"),
            raw_data["contract"][0].get("Permission_id")
        );
        let decoded = decode(&prepared.raw).unwrap();
        assert_eq!(decoded.owner_address(), owner);
        assert_eq!(decoded.prepared().unwrap().body, prepared.body);
        for (signed, key) in vector["signatures"].as_array().unwrap().iter().zip(&keys) {
            let signature = sign(&prepared.raw, key).unwrap();
            assert_eq!(
                hex::encode(signature),
                signed["signature"].as_str().unwrap()
            );
            assert_eq!(
                signer(&prepared.raw, &signature).unwrap(),
                signed["address"].as_str().unwrap()
            );
        }
    }
}

/// `getaccount`'s permissions read as java-tron writes them: the owner's
/// without a type or id covering everything, the active one covering only
/// its operations' contract types.
#[test]
fn permissions_read_and_cover_what_java_tron_says() {
    let fixture = fixture();
    let permissions = permissions(&fixture);
    let keys: Vec<&str> = fixture["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key["address"].as_str().unwrap())
        .collect();
    assert_eq!(permissions.owner.id, 0);
    assert_eq!(permissions.owner.threshold, 2);
    assert_eq!(permissions.owner.keys.len(), 3);
    let active = permissions.by_id(2).unwrap();
    for (contract_type, covered) in [
        (1, true),
        (2, true),
        (31, true),
        (4, false),
        (0, false),
        (54, false),
    ] {
        assert_eq!(active.covers(contract_type), covered, "{contract_type}");
        assert!(permissions.owner.covers(contract_type));
    }
    // No key meets any threshold alone; a two-of-three active spend.
    for key in &keys {
        assert_eq!(permissions.alone(key, TRANSFER_CONTRACT), None);
    }
    assert_eq!(permissions.for_contract(TRANSFER_CONTRACT).id, 2);
    assert_eq!(permissions.for_contract(4).id, 0);
    let single = TronPermissions::single(keys[0]);
    assert_eq!(single.alone(keys[0], TRANSFER_CONTRACT), Some(0));
    assert_eq!(single.alone(keys[1], TRANSFER_CONTRACT), None);
    // An empty answer is an account not yet on the network.
    assert_eq!(
        crate::api::tron_http::account_permissions(&serde_json::json!({}), keys[0]).unwrap(),
        single
    );
}

/// Under the active permission, P1 and P2 meet its threshold together, P0
/// holds no weight there, and a key twice is refused.
#[test]
fn signatures_count_only_the_permissions_keys() {
    let fixture = fixture();
    let permissions = permissions(&fixture);
    let vector = &fixture["transactions"][0];
    let raw = hex::decode(vector["raw_data_hex"].as_str().unwrap()).unwrap();
    let signature = |index: usize| {
        hex::decode(vector["signatures"][index]["signature"].as_str().unwrap()).unwrap()
    };
    let active = permissions.by_id(2).unwrap();
    let (signers, weight) = signed_weight(&raw, active, &[signature(1), signature(2)]).unwrap();
    assert_eq!(weight, 2);
    assert_eq!(signers.len(), 2);
    assert!(signed_weight(&raw, active, &[signature(0)]).is_err());
    assert!(signed_weight(&raw, active, &[signature(1), signature(1)]).is_err());
    let (_, weight) = signed_weight(&raw, &permissions.owner, &[signature(0)]).unwrap();
    assert_eq!(weight, 1);
}

/// Bytes that do not encode back the same are refused: an extra field, a
/// field moved, a contract type no review reads.
#[test]
fn raw_data_spectra_does_not_write_is_refused() {
    let fixture = fixture();
    let raw = hex::decode(fixture["transactions"][0]["raw_data_hex"].as_str().unwrap()).unwrap();
    assert!(decode(&raw).is_ok());
    let mut memo = raw.clone();
    memo.extend([0x52, 0x02, 0x68, 0x69]); // data (field 10): "hi"
    assert!(decode(&memo).is_err());
    let mut truncated = raw.clone();
    truncated.pop();
    assert!(decode(&truncated).is_err());
    assert!(decode(&[]).is_err());
}
