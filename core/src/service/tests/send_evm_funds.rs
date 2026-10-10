//! An EVM send is prepared on its own network and signed only when funded.
use super::*;
use crate::derivation::setup::{WalletSetupMethod, tests::fixture};
use serde_json::{Value, json};
use std::sync::Mutex;
use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

/// What the node reports: its chain id and the sender's balance.
#[derive(Clone, Copy)]
struct Node {
    chain_id: u64,
    balance: u128,
}

async fn node(state: Arc<Mutex<Node>>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            let node = *state.lock().unwrap();
            let answer = |call: &Value| {
                let result = match call["method"].as_str().unwrap_or_default() {
                    "eth_chainId" => json!(format!("0x{:x}", node.chain_id)),
                    "eth_getBalance" => json!(format!("0x{:x}", node.balance)),
                    "eth_getTransactionCount" => json!("0x7"),
                    "eth_estimateGas" => json!("0x5208"),
                    "eth_getCode" => json!("0x"),
                    "eth_gasPrice" => json!("0xb2d05e00"),
                    "eth_feeHistory" => {
                        json!({"baseFeePerGas": ["0x3b9aca00"], "reward": [["0x77359400"]]})
                    }
                    _ => {
                        return json!({"jsonrpc": "2.0", "id": call["id"],
                        "error": {"code": -32601, "message": "method not found"}});
                    }
                };
                json!({"jsonrpc": "2.0", "id": call["id"], "result": result})
            };
            let body: Value = request.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(match &body {
                Value::Array(calls) => Value::Array(calls.iter().map(answer).collect()),
                call => answer(call),
            })
        })
        .mount(&server)
        .await;
    server
}

/// The chain id an EIP-1559 transaction signs: the first field of its RLP
/// list, after the type byte.
fn signed_chain_id(payload: &str) -> u64 {
    let bytes = hex::decode(payload.trim_start_matches("0x")).unwrap();
    assert_eq!(bytes[0], 2, "an EIP-1559 transaction");
    let list = bytes[1];
    assert!(list >= 0xc0);
    let start = if list <= 0xf7 {
        2
    } else {
        2 + usize::from(list - 0xf7)
    };
    match bytes[start] {
        small @ 0..0x80 => u64::from(small),
        prefix => {
            let length = usize::from(prefix - 0x80);
            bytes[start + 1..start + 1 + length]
                .iter()
                .fold(0, |id, byte| id << 8 | u64::from(*byte))
        }
    }
}

/// A send built against a node of another network is refused and stores
/// nothing. Signing needs the value plus the whole fee budget: an empty
/// account, one holding the value alone and one a wei short each leave the
/// artifact prepared and unsigned. Funded, the signed bytes carry the
/// network's own chain id.
#[tokio::test]
async fn an_evm_send_is_built_on_its_own_network_and_signed_only_when_funded() {
    let chain = Chain::Plasma;
    let chain_id = chain.evm_chain_id().unwrap();
    let state = Arc::new(Mutex::new(Node {
        chain_id: 1,
        balance: 10 * 10u128.pow(18),
    }));
    let server = node(state.clone()).await;
    let service = WalletService::new(vec![ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id: chain,
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    service.set_secret_store(Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(
            std::env::temp_dir()
                .join(format!("evm-funds-{}.sqlite", crate::store::new_event_id()))
                .to_string_lossy()
                .into(),
        )
        .await
        .unwrap();
    let wallet = service
        .import_wallets(fixture(chain, WalletSetupMethod::ImportPhrase))
        .await
        .unwrap()
        .wallets
        .remove(0);
    let request = crate::send::SendExecutionRequest {
        chain_id: chain,
        wallet_id: wallet.id.clone(),
        password: None,
        to_address: format!("0x{}", "22".repeat(20)),
        amount_str: "0.01".into(),
        contract_address: None,
        token_standard: None,
        token_decimals: None,
        fee_rate_svb: None,
        fee_sat: None,
        gas_budget: None,
        fee_amount: None,
        evm_overrides: None,
        sign_only: false,
        memo: None,
    };

    let refused = service.build_send(request.clone()).await.unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("Endpoint is on the wrong network"),
        "{refused}"
    );
    assert!(service.list_sends().await.unwrap().is_empty());

    state.lock().unwrap().chain_id = chain_id;
    let prepared = service.build_send(request).await.unwrap();
    let crate::send::stages::PreparedPayload::Evm(transaction) =
        serde_json::from_str(&prepared.prepared_details).unwrap()
    else {
        panic!("an EVM payload: {}", prepared.prepared_details);
    };
    assert_eq!(
        (transaction.chain_id, transaction.value_wei),
        (chain_id, 10u128.pow(16))
    );
    let budget = transaction.value_wei + transaction.maximum_fee_wei().unwrap();
    for balance in [0, transaction.value_wei, budget - 1] {
        state.lock().unwrap().balance = balance;
        let refused = service
            .sign_send(prepared.id.clone(), prepared.review_digest.clone(), None)
            .await
            .unwrap_err();
        assert_eq!(
            refused.to_string(),
            "Insufficient funds for the amount plus the network fee.",
            "{balance}"
        );
        let stored = service.inspect_send(prepared.id.clone()).await.unwrap();
        assert_eq!(stored.stage, crate::send::stages::SendStage::Prepared);
        assert_eq!(stored.signed_payload, None);
    }
    state.lock().unwrap().balance = budget;
    let signed = service
        .sign_send(prepared.id.clone(), prepared.review_digest.clone(), None)
        .await
        .unwrap();
    assert_eq!(signed.stage, crate::send::stages::SendStage::Signed);
    assert_eq!(
        signed_chain_id(signed.signed_payload.as_deref().unwrap()),
        chain_id
    );
    let sent: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.body_json().unwrap())
        .collect();
    assert!(
        !serde_json::to_string(&sent)
            .unwrap()
            .contains("eth_sendRawTransaction")
    );
}
