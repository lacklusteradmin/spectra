use super::*;
use crate::send::polkadot::PreparedPolkadotTransaction;
use parity_scale_codec::{Compact, Decode, Encode};
use std::sync::{Arc, Mutex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

pub(crate) fn fixture(chain: Chain) -> &'static [u8] {
    // Full SCALE state_getMetadata responses from the official HTTP nodes:
    // polkadot-asset-hub-rpc.polkadot.io (spec 2005000, tx 15), and
    // westend-asset-hub-rpc.polkadot.io (spec 1025001, tx 16).
    match chain {
        Chain::Polkadot => {
            include_bytes!("../../../tests/fixtures/asset-hub-polkadot-metadata.scale")
        }
        Chain::PolkadotWestend => {
            include_bytes!("../../../tests/fixtures/asset-hub-westend-metadata.scale")
        }
        Chain::Bittensor => {
            include_bytes!("../../../tests/fixtures/bittensor-finney-metadata.scale")
        }
        _ => panic!("fixture needs an Asset Hub chain"),
    }
}

pub(crate) fn runtime_fixture(chain: Chain) -> PolkadotRuntime {
    let bytes = fixture(chain);
    let (transfer_pallet, transfer_call, existential_deposit, extensions) =
        metadata::Metadata::decode(bytes)
            .unwrap()
            .contract(chain)
            .unwrap();
    PolkadotRuntime {
        spec_version: if chain == Chain::Polkadot {
            2_005_000
        } else {
            1_025_001
        },
        transaction_version: if chain == Chain::Polkadot { 15 } else { 16 },
        genesis_hash: chain.substrate_genesis_hash().unwrap().into(),
        metadata_hash: blake2_hash(bytes),
        transfer_pallet,
        transfer_call,
        existential_deposit,
        extensions,
    }
}

#[derive(Clone)]
struct State {
    genesis: String,
    nonce: u64,
    fee: u128,
    free: u128,
}

async fn node(chain: Chain) -> (SubstrateClient, Arc<Mutex<State>>, MockServer) {
    let state = Arc::new(Mutex::new(State {
        genesis: chain.substrate_genesis_hash().unwrap().into(),
        nonce: 7,
        fee: 8_758_355,
        free: 100_000_000_000,
    }));
    let server = MockServer::start().await;
    let copy = state.clone();
    Mock::given(method("POST")).respond_with(move |request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let live = copy.lock().unwrap();
        let runtime = runtime_fixture(chain);
        let result = match body["method"].as_str().unwrap() {
            "chain_getBlockHash" if body["params"][0] == 0 => json!(live.genesis),
            "chain_getBlockHash" | "chain_getFinalizedHead" => json!(format!("0x{}", "11".repeat(32))),
            "chain_getHeader" => json!({"number":"0x64"}),
            "state_getMetadata" => json!(format!("0x{}",hex::encode(fixture(chain)))),
            "state_getRuntimeVersion" => json!({"specVersion":runtime.spec_version,"transactionVersion":runtime.transaction_version}),
            "system_accountNextIndex" => json!(live.nonce),
            "payment_queryInfo" => json!({"partialFee":live.fee.to_string()}),
            "state_getStorage" => {
                let mut bytes = vec![0;16];
                let width = chain.substrate_balance_bytes().unwrap();
                bytes.extend(&live.free.to_le_bytes()[..width]); bytes.extend(vec![0;2*width+16]);
                json!(format!("0x{}",hex::encode(bytes)))
            }
            method => panic!("unexpected RPC {method}"),
        };
        ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result}))
    }).mount(&server).await;
    (
        SubstrateClient::new(Arc::new(vec![server.uri()])),
        state,
        server,
    )
}

#[test]
fn live_metadata_contracts_select_asset_hub_calls_and_deposits() {
    for (chain, ed) in [
        (Chain::Polkadot, 100_000_000),
        (Chain::PolkadotWestend, 1_000_000_000),
    ] {
        let runtime = runtime_fixture(chain);
        assert_eq!((runtime.transfer_pallet, runtime.transfer_call), (10, 3));
        assert_eq!(runtime.existential_deposit, ed);
        assert!(
            runtime
                .extensions
                .contains(&PolkadotExtension::MetadataHash)
        );
    }
}

#[tokio::test]
async fn wrong_relay_genesis_is_refused_before_balance_or_nonce() {
    for chain in [Chain::Polkadot, Chain::PolkadotWestend, Chain::Bittensor] {
        let (client, live, server) = node(chain).await;
        live.lock().unwrap().genesis =
            "0x91b171bb158e2d3848fa23a9f1c25182fb8e20313b2c1eb49219da7a70ce90c3".into();
        assert!(
            client
                .fetch_balance(chain, &[0; 32])
                .await
                .unwrap_err()
                .to_string()
                .contains("wrong Substrate")
        );
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().all(
            |r| serde_json::from_slice::<Value>(&r.body).unwrap()["method"] == "chain_getBlockHash"
        ));
    }
}

#[tokio::test]
async fn asset_hub_signing_uses_genesis_and_all_current_extensions_and_rechecks_funds() {
    let (client, live, _server) = node(Chain::Polkadot).await;
    let key = [7u8; 32];
    let pair = schnorrkel::MiniSecretKey::from_bytes(&key)
        .unwrap()
        .expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let sender = crate::derivation::primitives::encode_ss58(&pair.public.to_bytes(), 0);
    let prepared = crate::send::polkadot::prepare_transfer(
        &client,
        Chain::Polkadot,
        &sender,
        &sender,
        10_000_000_000,
    )
    .await
    .unwrap();
    let raw = prepared.sign(&key, &pair.public.to_bytes()).unwrap();
    let mut bytes = raw.as_slice();
    let length = Compact::<u32>::decode(&mut bytes).unwrap().0 as usize;
    assert_eq!(length, bytes.len());
    assert_eq!(&bytes[..2], &[0x84, 0]);
    assert_eq!(bytes[2..34], pair.public.to_bytes());
    assert_eq!(bytes[34], 1);
    let signature = schnorrkel::Signature::from_bytes(&bytes[35..99]).unwrap();
    let mut expected = vec![10, 3, 0];
    expected.extend(pair.public.to_bytes());
    expected.extend(Compact(10_000_000_000u128).encode());
    expected.extend([0, 28, 0, 0, 0]); // immortal, nonce7, zero tip, None asset, Disabled metadata
    expected.extend(2_005_000u32.to_le_bytes());
    expected.extend(15u32.to_le_bytes());
    let genesis = hex::decode(
        Chain::Polkadot
            .substrate_genesis_hash()
            .unwrap()
            .trim_start_matches("0x"),
    )
    .unwrap();
    expected.extend(&genesis);
    expected.extend(&genesis);
    expected.push(0); // None metadata hash
    assert_eq!(prepared.signing_payload().unwrap(), expected);
    pair.public
        .verify_simple(b"substrate", &expected, &signature)
        .unwrap();
    let uniform = schnorrkel::MiniSecretKey::from_bytes(&key)
        .unwrap()
        .expand_to_keypair(schnorrkel::ExpansionMode::Uniform);
    let mut uniform_prepared = prepared.clone();
    uniform_prepared.sender = uniform.public.to_bytes();
    let uniform_raw = uniform_prepared
        .sign(&key, &uniform.public.to_bytes())
        .unwrap();
    let (_, uniform_body) = {
        let mut body = uniform_raw.as_slice();
        let length = Compact::<u32>::decode(&mut body).unwrap();
        (length, body)
    };
    let signature = schnorrkel::Signature::from_bytes(&uniform_body[35..99]).unwrap();
    uniform
        .public
        .verify_simple(
            b"substrate",
            &uniform_prepared.signing_payload().unwrap(),
            &signature,
        )
        .unwrap();
    assert_eq!(&bytes[99..104], &[0, 28, 0, 0, 0]);
    assert!(prepared.sign(&[8; 32], &pair.public.to_bytes()).is_err());
    prepared
        .validate_for_signing(&client, Chain::Polkadot, &sender)
        .await
        .unwrap();
    live.lock().unwrap().fee += 1;
    assert!(
        prepared
            .validate_for_signing(&client, Chain::Polkadot, &sender)
            .await
            .unwrap_err()
            .to_string()
            .contains("fee increased")
    );
    live.lock().unwrap().fee -= 1;
    live.lock().unwrap().free =
        prepared.amount + prepared.fee + prepared.runtime.existential_deposit - 1;
    assert!(
        prepared
            .validate_for_signing(&client, Chain::Polkadot, &sender)
            .await
            .unwrap_err()
            .to_string()
            .contains("Insufficient")
    );
    live.lock().unwrap().nonce = u64::from(u32::MAX) + 1;
    assert!(client.fetch_nonce(&sender).await.is_err());
    live.lock().unwrap().nonce = 7;
    live.lock().unwrap().free = 1u128 << 100;
    let large = crate::send::polkadot::prepare_transfer(
        &client,
        Chain::Polkadot,
        &sender,
        &sender,
        u128::from(u64::MAX) + 1,
    )
    .await
    .unwrap();
    let payload = large.signing_payload().unwrap();
    let mut encoded_amount = &payload[35..];
    assert_eq!(
        Compact::<u128>::decode(&mut encoded_amount).unwrap().0,
        u128::from(u64::MAX) + 1
    );
}

#[test]
fn keep_alive_reductions_follow_runtime_freeze_and_reserve_rules() {
    let balance = SubstrateBalance {
        free: 100,
        reserved: 10,
        frozen: 40,
    };
    assert_eq!(balance.keep_alive_spendable(20), 70);
    assert_eq!(balance.keep_alive_spendable(50), 50);
    let prepared = PreparedPolkadotTransaction {
        runtime: runtime_fixture(Chain::PolkadotWestend),
        sender: [0; 32],
        call_data: crate::send::polkadot::native_transfer_call(
            &runtime_fixture(Chain::PolkadotWestend),
            &[1; 32],
            u128::MAX,
        ),
        nonce: u32::MAX,
        amount: u128::MAX,
        fee: 1,
        finalized_number: 0,
    };
    assert!(!prepared.fee_extrinsic().unwrap().is_empty());
}

#[tokio::test]
async fn finney_signing_uses_runtime_call_extensions_and_balance_width() {
    let (client, live, _server) = node(Chain::Bittensor).await;
    let key = [7u8; 32];
    let pair = schnorrkel::MiniSecretKey::from_bytes(&key)
        .unwrap()
        .expand_to_keypair(schnorrkel::ExpansionMode::Ed25519);
    let sender = crate::derivation::primitives::encode_ss58(&pair.public.to_bytes(), 42);
    let prepared = crate::send::polkadot::prepare_transfer(
        &client,
        Chain::Bittensor,
        &sender,
        &sender,
        1_000_000_000,
    )
    .await
    .unwrap();
    assert_eq!(prepared.runtime.existential_deposit, 500);
    let raw = prepared.sign(&key, &pair.public.to_bytes()).unwrap();
    let mut body = raw.as_slice();
    let length = Compact::<u32>::decode(&mut body).unwrap().0 as usize;
    assert_eq!(length, body.len());
    assert_eq!(&body[99..106], &[0, 28, 0, 0, 5, 3, 0]); // immortal, nonce, tip, metadata mode, call
    let payload = prepared.signing_payload().unwrap();
    pair.public
        .verify_simple(
            b"substrate",
            &payload,
            &schnorrkel::Signature::from_bytes(&body[35..99]).unwrap(),
        )
        .unwrap();
    assert_eq!(payload.last(), Some(&0)); // metadata hash None in AdditionalSigned
    prepared
        .validate_for_submission(&client, Chain::Bittensor)
        .await
        .unwrap();
    live.lock().unwrap().fee += 1;
    assert!(
        prepared
            .validate_for_submission(&client, Chain::Bittensor)
            .await
            .is_err()
    );
    live.lock().unwrap().genesis = Chain::Polkadot.substrate_genesis_hash().unwrap().into();
    assert!(
        client
            .polkadot_context(Chain::Bittensor)
            .await
            .unwrap_err()
            .to_string()
            .contains("wrong Substrate")
    );
    assert!(
        crate::send::polkadot::prepare_transfer(
            &client,
            Chain::Bittensor,
            &sender,
            &sender,
            u128::from(u64::MAX) + 1
        )
        .await
        .is_err()
    );
}
