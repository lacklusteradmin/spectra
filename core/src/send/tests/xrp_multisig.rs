//! Multi-signed XRP payments against xrpl.js's (`xrp-multisig.json`, from
//! scripts/generate-xrp-multisig-vectors.cjs).

use super::*;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/xrp-multisig.json")).unwrap()
}

fn payment(tx: &serde_json::Value) -> XrpPayment {
    XrpPayment {
        account: account_id(tx["Account"].as_str().unwrap()).unwrap(),
        destination: account_id(tx["Destination"].as_str().unwrap()).unwrap(),
        destination_tag: tx["DestinationTag"].as_u64().map(|tag| tag as u32),
        amount_drops: tx["Amount"].as_str().unwrap().parse().unwrap(),
        fee_drops: tx["Fee"].as_str().unwrap().parse().unwrap(),
        sequence: tx["Sequence"].as_u64().unwrap() as u32,
        last_ledger_sequence: tx["LastLedgerSequence"].as_u64().unwrap() as u32,
    }
}

fn list(fixture: &serde_json::Value) -> XrpSignerList {
    XrpSignerList {
        quorum: fixture["signer_list"]["quorum"].as_u64().unwrap(),
        entries: fixture["signer_list"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["account"].as_str().unwrap().to_string(),
                    entry["weight"].as_u64().unwrap(),
                )
            })
            .collect(),
    }
}

fn upper(bytes: &[u8]) -> String {
    hex::encode_upper(bytes)
}

/// Each signer's signing data and signature are ripple-keypairs', its
/// single-signer blob xrpl.js's, and the two combine into `multisign()`'s
/// blob and `hashSignedTx`'s hash, whichever order they are added in.
#[test]
fn signing_data_signatures_and_blobs_are_xrpl_jss() {
    let fixture = fixture();
    for vector in fixture["transactions"].as_array().unwrap() {
        let payment = payment(&vector["tx"]);
        let mut signers = Vec::new();
        for signed in vector["signatures"].as_array().unwrap() {
            let key = &fixture["keys"][signed["signer"].as_str().unwrap()];
            let account = account_id(signed["account"].as_str().unwrap()).unwrap();
            assert_eq!(
                upper(&payment.multisigning_data(&account).unwrap()),
                signed["multisigning_hex"].as_str().unwrap()
            );
            let signer = sign(
                &payment,
                &hex::decode(key["private_key"].as_str().unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(
                upper(&signer.signature),
                signed["txn_signature"].as_str().unwrap()
            );
            assert_eq!(
                upper(&signer.public_key),
                signed["signing_pub_key"].as_str().unwrap()
            );
            assert!(verify(&payment, &signer));
            let single = payment.blob(std::slice::from_ref(&signer)).unwrap();
            assert_eq!(
                upper(&single),
                signed["single_signer_blob"].as_str().unwrap()
            );
            assert_eq!(
                decode(&single).unwrap(),
                (payment.clone(), vec![signer.clone()])
            );
            signers.push(signer);
        }
        let blob = payment.blob(&signers).unwrap();
        assert_eq!(
            upper(&blob),
            vector["multisigned"]["blob"].as_str().unwrap()
        );
        signers.reverse();
        assert_eq!(payment.blob(&signers).unwrap(), blob);
        assert_eq!(
            upper(&XrpPayment::hash(&blob)),
            vector["multisigned"]["hash"].as_str().unwrap()
        );
        let (decoded, read) = decode(&blob).unwrap();
        assert_eq!(decoded, payment);
        let (addresses, weight) = signed_weight(&decoded, &list(&fixture), &read).unwrap();
        assert_eq!(weight, 2);
        assert_eq!(addresses.len(), 2);
    }
}

/// Refused: a signer not on the list, a signature over another payment, a
/// signer twice, signers out of order, a field no review reads.
#[test]
fn foreign_signers_and_unread_blobs_are_refused() {
    let fixture = fixture();
    let vector = &fixture["transactions"][0];
    let payment = payment(&vector["tx"]);
    let key =
        |name: &str| hex::decode(fixture["keys"][name]["private_key"].as_str().unwrap()).unwrap();
    let p1 = sign(&payment, &key("p1")).unwrap();
    let p2 = sign(&payment, &key("p2")).unwrap();
    let p0 = sign(&payment, &key("p0")).unwrap();
    let list = list(&fixture);
    assert!(signed_weight(&payment, &list, &[p0]).is_err());
    let mut other = payment.clone();
    other.amount_drops += 1;
    assert!(signed_weight(&other, &list, std::slice::from_ref(&p1)).is_err());
    assert!(signed_weight(&payment, &list, &[p1.clone(), p1.clone()]).is_err());
    let mut forged = p2.clone();
    forged.account = p1.account;
    assert!(signed_weight(&payment, &list, &[forged]).is_err());
    // Signers out of account order are not canonical.
    let blob = payment.blob(&[p1.clone(), p2.clone()]).unwrap();
    let unsorted = {
        let first = payment.fields(Some(&[p2, p1])).unwrap();
        assert_ne!(first, blob);
        first
    };
    assert!(decode(&unsorted).is_err());
    let mut memo = blob.clone();
    memo.insert(memo.len() - 1, 0x00);
    assert!(decode(&memo).is_err());
    assert!(decode(&blob[..blob.len() - 1]).is_err());
}
