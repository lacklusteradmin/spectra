//! Native transfers against @polkadot/types on each network's own runtime
//! (`substrate-transfers.json`, from
//! tests/vectors/generate-substrate-transfer-vectors.cjs).

use super::*;
use serde_json::Value;

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap().trim_start_matches("0x")).unwrap()
}

/// On every Substrate mainnet, a `transfer_keep_alive` encodes the call,
/// signing payload and signed extrinsic polkadot.js builds from the same
/// runtime metadata, byte for byte; polkadot.js's signature verifies over
/// Spectra's payload and Spectra's own signature sits in the same envelope.
#[test]
fn native_transfers_match_polkadot_js_on_every_network() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/substrate-transfers.json"
    ))
    .unwrap();
    let seed: [u8; 32] = bytes(&fixture["seed"]).try_into().unwrap();
    let pair = schnorrkel::MiniSecretKey::from_bytes(&seed)
        .unwrap()
        .expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let sender = pair.public.to_bytes();
    let recipient: [u8; 32] = bytes(&fixture["recipient"]).try_into().unwrap();
    let fixed = bytes(&fixture["fixedSignature"]);
    assert_eq!(fixed[0], 1, "MultiSignature::Sr25519");
    let fixed: [u8; 64] = fixed[1..].try_into().unwrap();
    let mut networks = 0;
    for chain in Chain::all().filter(|c| c.is_substrate() && !c.is_testnet()) {
        let vectors = &fixture[chain.str_id()];
        assert_eq!(
            vectors["sender"],
            crate::derivation::primitives::encode_ss58(&sender, chain.ss58_prefix().unwrap()),
            "{chain}"
        );
        assert_eq!(
            vectors["genesisHash"],
            chain.substrate_genesis_hash().unwrap(),
            "{chain}"
        );
        let mut runtime = crate::api::substrate_json_rpc::tests::runtime_fixture(chain);
        runtime.spec_version = u32::try_from(vectors["specVersion"].as_u64().unwrap()).unwrap();
        runtime.transaction_version =
            u32::try_from(vectors["transactionVersion"].as_u64().unwrap()).unwrap();
        for case in vectors["cases"].as_array().unwrap() {
            let amount: u128 = case["amount"].as_str().unwrap().parse().unwrap();
            let prepared = PreparedPolkadotTransaction {
                call_data: native_transfer_call(&runtime, &recipient, amount),
                runtime: runtime.clone(),
                sender,
                nonce: u32::try_from(case["nonce"].as_u64().unwrap()).unwrap(),
                amount,
                asset: None,
                fee: 0,
                finalized_number: 0,
            };
            assert_eq!(prepared.call_data, bytes(&case["call"]), "{chain}");
            let payload = prepared.signing_payload().unwrap();
            assert_eq!(payload, bytes(&case["signingPayload"]), "{chain}");
            assert_eq!(
                prepared.extrinsic(fixed).unwrap(),
                bytes(&case["signedWithFixedSignature"]),
                "{chain}"
            );
            let theirs = bytes(&case["signature"]);
            assert_eq!(theirs[0], 1, "{chain}");
            let theirs: [u8; 64] = theirs[1..].try_into().unwrap();
            pair.public
                .verify_simple(
                    b"substrate",
                    &payload,
                    &schnorrkel::Signature::from_bytes(&theirs).unwrap(),
                )
                .unwrap();
            assert_eq!(
                prepared.extrinsic(theirs).unwrap(),
                bytes(&case["signed"]),
                "{chain}"
            );
            let ours = prepared.sign(&seed, &sender).unwrap();
            let mut body = ours.as_slice();
            <Compact<u32> as parity_scale_codec::Decode>::decode(&mut body).unwrap();
            let signature: [u8; 64] = body[35..99].try_into().unwrap();
            pair.public
                .verify_simple(
                    b"substrate",
                    &payload,
                    &schnorrkel::Signature::from_bytes(&signature).unwrap(),
                )
                .unwrap();
            assert_eq!(prepared.extrinsic(signature).unwrap(), ours, "{chain}");
        }
        networks += 1;
    }
    assert_eq!(networks, 4);
}
