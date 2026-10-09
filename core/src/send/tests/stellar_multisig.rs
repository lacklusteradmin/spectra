//! Multisig Stellar payments against stellar-base's
//! (`stellar-multisig.json`, from scripts/generate-stellar-multisig-vectors.cjs).

use super::*;
use base64::Engine;

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/stellar-multisig.json"
    ))
    .unwrap()
}

fn b64(text: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap()
}

fn payment(fixture: &serde_json::Value, vector: &serde_json::Value) -> StellarPayment {
    StellarPayment {
        source: decode_stellar_address(fixture["keys"]["p0"]["address"].as_str().unwrap()).unwrap(),
        fee: vector["xdr_fee"].as_u64().unwrap() as u32,
        sequence: vector["seq_num"].as_str().unwrap().parse().unwrap(),
        min_time: vector["time_bounds"]["minTime"].as_u64().unwrap(),
        max_time: vector["time_bounds"]["maxTime"].as_u64().unwrap(),
        memo: match vector["memo"]["type"].as_str().unwrap() {
            "none" => None,
            "text" => Some(OwnedMemo::Text(
                vector["memo"]["value"].as_str().unwrap().into(),
            )),
            other => panic!("{other}"),
        },
        destination: decode_stellar_address(vector["payment"]["destination"].as_str().unwrap())
            .unwrap(),
        stroops: vector["payment"]["amount_stroops"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    }
}

fn signers(fixture: &serde_json::Value) -> StellarSigners {
    let policy = &fixture["policy"];
    StellarSigners {
        keys: policy["signers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|signer| {
                (
                    signer["key"].as_str().unwrap().to_string(),
                    signer["weight"].as_u64().unwrap(),
                )
            })
            .collect(),
        other: Vec::new(),
        low: policy["thresholds"]["low"].as_u64().unwrap(),
        medium: policy["thresholds"]["medium"].as_u64().unwrap(),
        high: policy["thresholds"]["high"].as_u64().unwrap(),
    }
}

fn seed(fixture: &serde_json::Value, key: &str) -> [u8; 32] {
    hex::decode(fixture["keys"][key]["secret_seed"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

/// Each payment encodes as stellar-base's unsigned envelope and hashes to
/// its hash; P0's and P1's signatures are stellar-base's, and the
/// envelopes with them are its envelopes, which decode back.
#[test]
fn envelopes_hashes_and_signatures_are_stellar_bases() {
    let fixture = fixture();
    let passphrase = fixture["network_passphrase"].as_str().unwrap();
    for vector in fixture["transactions"].as_array().unwrap() {
        let payment = payment(&fixture, vector);
        assert_eq!(
            payment.envelope(&[]).unwrap(),
            b64(vector["unsigned_envelope_xdr"].as_str().unwrap()),
            "{}",
            vector["name"]
        );
        assert_eq!(
            hex::encode(payment.hash(passphrase).unwrap()),
            vector["hash"].as_str().unwrap()
        );
        let signed: Vec<StellarSignature> = vector["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|signature| {
                let ours = sign(
                    &payment,
                    passphrase,
                    &seed(&fixture, signature["signer"].as_str().unwrap()),
                )
                .unwrap();
                assert_eq!(hex::encode(ours.hint), signature["hint"].as_str().unwrap());
                assert_eq!(
                    hex::encode(&ours.signature),
                    signature["signature"].as_str().unwrap()
                );
                ours
            })
            .collect();
        let both = b64(vector["envelope_p0_p1_xdr"].as_str().unwrap());
        assert_eq!(payment.envelope(&signed).unwrap(), both);
        assert_eq!(
            payment.envelope(&signed[1..]).unwrap(),
            b64(vector["envelope_p1_only_xdr"].as_str().unwrap())
        );
        let (read, signatures) = decode(&both).unwrap();
        assert_eq!(read, payment);
        let (addresses, weight) =
            signed_weight(&read, passphrase, &signers(&fixture), &signatures).unwrap();
        assert_eq!(weight, 2);
        assert_eq!(addresses.len(), 2);
    }
}

/// Refused: a signature on another network, a signer twice, a key that is
/// no signer, an envelope with anything no review reads.
#[test]
fn foreign_signatures_and_unread_envelopes_are_refused() {
    let fixture = fixture();
    let passphrase = fixture["network_passphrase"].as_str().unwrap();
    let vector = &fixture["transactions"][0];
    let payment = payment(&fixture, vector);
    let signers = signers(&fixture);
    let p1 = sign(&payment, passphrase, &seed(&fixture, "p1")).unwrap();
    let mainnet = sign(
        &payment,
        "Public Global Stellar Network ; September 2015",
        &seed(&fixture, "p1"),
    )
    .unwrap();
    assert!(signed_weight(&payment, passphrase, &signers, &[mainnet]).is_err());
    assert!(signed_weight(&payment, passphrase, &signers, &[p1.clone(), p1.clone()]).is_err());
    let stranger = sign(&payment, passphrase, &[7; 32]).unwrap();
    assert!(signed_weight(&payment, passphrase, &signers, &[stranger]).is_err());
    let envelope = payment.envelope(&[p1]).unwrap();
    assert!(decode(&envelope[..envelope.len() - 4]).is_err());
    let mut extra = envelope.clone();
    extra.extend([0, 0, 0, 0]);
    assert!(decode(&extra).is_err());
    // An operation source, a field no review reads.
    let tx = payment.tx().unwrap();
    let mut sourced = 2u32.to_be_bytes().to_vec();
    let at = tx.len() - 4 - 8 - 4 - 32 - 4 - 4 - 4;
    sourced.extend(&tx[..at]);
    sourced.extend(1u32.to_be_bytes());
    assert!(decode(&sourced).is_err());
}
