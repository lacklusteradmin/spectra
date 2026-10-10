use super::*;

/// EIP-137's own namehash vectors.
#[test]
fn namehash_is_eip137s() {
    let hash = |name| hex::encode(crate::api::evm_json_rpc::ens_namehash(name));
    assert_eq!(hash(""), "0".repeat(64));
    assert_eq!(
        hash("eth"),
        "93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae"
    );
    assert_eq!(
        hash("foo.eth"),
        "de9b09fd7c5f901e23a3f19fecc54828e9c848539801e86591bd9801b019f84f"
    );
}

/// A revocation is `approve(spender, 0)`: the selector, the spender as a
/// word, and a zero word.
#[test]
fn a_revocation_approves_nothing() {
    let spender = "0x1111111254eeb25477b68fb85ed929f73a960582";
    let data = crate::send::evm::encode_erc20_approve(spender, 0).unwrap();
    assert_eq!(
        hex::encode(data),
        format!("095ea7b3{:0>64}{}", &spender[2..], "0".repeat(64))
    );
}

/// Neither a network without EVM approvals nor a wallet that is gone is
/// asked anything.
#[tokio::test]
async fn only_an_evm_wallet_has_approvals() {
    let (service, directory) = crate::derivation::setup::tests::service().await;
    let wallet = service
        .import_wallets(crate::derivation::setup::tests::fixture(
            Chain::Solana,
            crate::derivation::setup::WalletSetupMethod::ImportPhrase,
        ))
        .await
        .unwrap()
        .wallets[0]
        .id
        .clone();
    assert!(matches!(
        service.wallet_token_approvals(wallet.clone()).await,
        Err(SpectraBridgeError::InvalidInput { .. })
    ));
    assert!(service.wallet_ens_name("missing".into()).await.is_err());
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
}

const OWNER: &str = "0x9858effd232b4033e47d90003d41ec34ecaeda94";
const TOKEN: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const COLLECTION: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const UNLIMITED: &str = "0x1111111111111111111111111111111111111111";
const SPENT: &str = "0x2222222222222222222222222222222222222222";
const OPERATOR: &str = "0x3333333333333333333333333333333333333333";
const RESOLVER: &str = "0x4444444444444444444444444444444444444444";

/// One 32-byte ABI word holding `hex`.
fn word(hex: &str) -> String {
    format!("0x{:0>64}", hex.trim_start_matches("0x"))
}

/// An ABI-encoded string.
fn abi_string(text: &str) -> String {
    format!("0x{:064x}{:064x}{:0<64}", 32, text.len(), hex::encode(text))
}

/// What the node and indexer answer, and what they were sent.
#[derive(Default)]
struct Approvals {
    /// The address the wallet's ENS name resolves to.
    ens_address: String,
    submitted: Vec<String>,
}

/// An Ethereum node at `/rpc` and a Blockscout-compatible indexer at
/// `/scout`. The wallet logged four approvals: one spender twice, one whose
/// allowance it has since spent, and an NFT operator's, which names a token
/// id as a fourth topic.
async fn approvals_node(state: Arc<std::sync::Mutex<Approvals>>) -> wiremock::MockServer {
    use serde_json::{Value, json};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
    let server = MockServer::start().await;
    Mock::given(any())
        .respond_with(move |request: &Request| {
            if request.url.path().starts_with("/scout") {
                let query: HashMap<String, String> =
                    request.url.query_pairs().into_owned().collect();
                if query.get("module").map(String::as_str) != Some("logs")
                    || query.get("topic1") != Some(&word(OWNER))
                {
                    return ResponseTemplate::new(400);
                }
                let log = |index: u64, token: &str, spender: &str, fourth: Value| {
                    json!({"address": token, "blockNumber": "0x10", "data": word("0"),
                        "logIndex": format!("0x{index:x}"),
                        "transactionHash": format!("0x{index:064x}"), "timeStamp": "0x1",
                        "topics": ["0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925",
                            word(OWNER), word(spender), fourth]})
                };
                let rows = [
                    log(1, TOKEN, UNLIMITED, Value::Null),
                    log(2, TOKEN, SPENT, Value::Null),
                    log(3, COLLECTION, OPERATOR, json!(word("7"))),
                    log(4, TOKEN, UNLIMITED, Value::Null),
                ];
                return ResponseTemplate::new(200)
                    .set_body_json(json!({"status": "1", "message": "OK", "result": rows}));
            }
            let mut state = state.lock().unwrap();
            let mut answer = |call: &Value| {
                let result = match call["method"].as_str().unwrap_or_default() {
                    "eth_call" => {
                        let data = call["params"][0]["data"].as_str().unwrap_or_default();
                        let to = call["params"][0]["to"].as_str().unwrap_or_default();
                        match &data[2..10] {
                            // allowance(owner, spender)
                            "dd62ed3e" if data.ends_with(&UNLIMITED[2..]) => json!(word(&"f".repeat(64))),
                            "dd62ed3e" => json!(word("0")),
                            "313ce567" => json!(word("12")),
                            "95d89b41" => json!(abi_string("TKA")),
                            "01ffc9a7" => json!(word("0")),
                            // ENS: the registry's resolver, the reverse
                            // record's name, and the name's address.
                            "0178b8bf" => json!(word(RESOLVER)),
                            "691f3431" if to == RESOLVER => json!(abi_string("spectra.eth")),
                            "3b3b57de" if to == RESOLVER => json!(word(&state.ens_address)),
                            _ => Value::Null,
                        }
                    }
                    "eth_sendRawTransaction" => {
                        use sha3::Digest;
                        let raw = call["params"][0].as_str().unwrap_or_default().to_string();
                        let bytes = hex::decode(raw.trim_start_matches("0x")).unwrap();
                        state.submitted.push(raw);
                        json!(format!("0x{}", hex::encode(sha3::Keccak256::digest(bytes))))
                    }
                    "eth_chainId" => json!("0x1"),
                    "eth_getBalance" => json!(format!("0x{:x}", 10u128.pow(19))),
                    "eth_getCode" => json!("0x"),
                    "eth_getTransactionCount" => json!("0x3"),
                    "eth_estimateGas" => json!("0xb000"),
                    "eth_gasPrice" => json!("0xb2d05e00"),
                    "eth_blockNumber" => json!("0x20"),
                    "eth_feeHistory" => {
                        json!({"baseFeePerGas": ["0x3b9aca00"], "reward": [["0x77359400"]]})
                    }
                    _ => Value::Null,
                };
                if result.is_null() {
                    json!({"jsonrpc": "2.0", "id": call["id"],
                        "error": {"code": -32601, "message": "unexpected call"}})
                } else {
                    json!({"jsonrpc": "2.0", "id": call["id"], "result": result})
                }
            };
            let body: Value = request.body_json().unwrap();
            ResponseTemplate::new(200).set_body_json(match &body {
                Value::Array(calls) => Value::Array(calls.iter().map(&mut answer).collect()),
                call => answer(call),
            })
        })
        .mount(&server)
        .await;
    server
}

/// The wallet's approvals are its logged `Approval` events that a live
/// `allowance` read says still stand: a spent one and an NFT operator's are
/// left out, and a network with no indexer is refused rather than answered
/// with none. A revocation of a spent allowance is refused; one that stands
/// is `approve(spender, 0)` on the token, priced from the node's fee
/// history, signed and broadcast as a send, and recorded as a revocation.
/// The ENS name shows only while it resolves back to the wallet.
#[tokio::test]
async fn standing_approvals_are_found_revoked_and_named() {
    use crate::derivation::setup::{WalletSetupMethod, tests::fixture};
    use crate::store::state::{AppSettingUpdate, StateCommand};
    let state = Arc::new(std::sync::Mutex::new(Approvals {
        ens_address: OWNER.into(),
        ..Default::default()
    }));
    let server = approvals_node(state.clone()).await;
    let rpc = format!("{}/rpc", server.uri());
    let endpoints = |chain_id| ChainEndpoints {
        capabilities: EndpointCapability::ALL.to_vec(),
        chain_id,
        endpoints: vec![rpc.clone()],
    };
    let service =
        WalletService::new(vec![endpoints(Chain::Ethereum), endpoints(Chain::BnbChain)]).unwrap();
    service.set_secret_store(Arc::new(
        crate::store::secret_backends::InMemorySecretStore::new(),
    ));
    service
        .open_state(
            std::env::temp_dir()
                .join(format!("approvals-{}.sqlite", crate::store::new_event_id()))
                .to_string_lossy()
                .into(),
        )
        .await
        .unwrap();
    let set = |update| StateCommand::SetAppSetting { update };
    for chain_id in [Chain::Ethereum, Chain::BnbChain] {
        service
            .apply_state_command(set(AppSettingUpdate::CustomEndpointsOnly {
                chain_id,
                value: true,
            }))
            .await
            .unwrap();
    }
    let import = |chain| service.import_wallets(fixture(chain, WalletSetupMethod::ImportPhrase));
    let approver = import(Chain::Ethereum).await.unwrap().wallets.remove(0);
    assert_eq!(approver.primary_address(), Some(OWNER));
    let unindexed = import(Chain::BnbChain).await.unwrap().wallets.remove(0);
    let refused = service
        .wallet_token_approvals(unindexed.id.clone())
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("needs an address indexer to find token approvals"),
        "{refused}"
    );
    service
        .apply_state_command(set(AppSettingUpdate::AddCustomEndpoint {
            capabilities: vec![EndpointCapability::History],
            chain_id: Chain::Ethereum,
            api: "blockscout".into(),
            endpoint: format!("{}/scout", server.uri()),
        }))
        .await
        .unwrap();

    let approvals = service
        .wallet_token_approvals(approver.id.clone())
        .await
        .unwrap();
    assert!(approvals.complete);
    let found: Vec<_> = approvals
        .approvals
        .iter()
        .map(|a| {
            (
                a.token.as_str(),
                a.spender.as_str(),
                a.symbol.as_str(),
                a.unlimited,
            )
        })
        .collect();
    assert_eq!(found, [(TOKEN, UNLIMITED, "TKA", true)]);

    let spent = service
        .build_approval_revocation(approver.id.clone(), TOKEN.into(), SPENT.into())
        .await
        .unwrap_err();
    assert_eq!(
        spent.to_string(),
        "Nothing to revoke: this spender's allowance is already zero."
    );
    let built = service
        .build_approval_revocation(approver.id.clone(), TOKEN.into(), UNLIMITED.into())
        .await
        .unwrap();
    // 0xb000 gas and Ethereum's 20% margin, 54068, at twice the 1 gwei base
    // fee plus the 2 gwei tip.
    assert_eq!(
        built.operation,
        Some(WalletOperation::RevokeApproval {
            token: TOKEN.into(),
            spender: UNLIMITED.into(),
            network_fee: "0.000216272".into(),
        })
    );
    let PreparedPayload::Evm(call) = serde_json::from_str(&built.prepared_details).unwrap() else {
        panic!("an EVM call: {}", built.prepared_details);
    };
    assert_eq!((call.to.as_str(), call.value_wei), (TOKEN, 0));
    assert_eq!(
        call.data,
        crate::send::evm::encode_erc20_approve(UNLIMITED, 0).unwrap()
    );

    let signed = service
        .sign_send(built.id.clone(), built.review_digest.clone(), None)
        .await
        .unwrap();
    service
        .broadcast_send(signed.id.clone(), vec![rpc.clone()])
        .await
        .unwrap();
    assert_eq!(
        state.lock().unwrap().submitted,
        [signed.signed_payload.clone().unwrap()]
    );
    let kinds: Vec<_> = service
        .transactions()
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.kind)
        .collect();
    assert_eq!(
        kinds,
        [crate::store::wallet_domain::TransactionKind::RevokeApproval]
    );

    assert_eq!(
        service.wallet_ens_name(approver.id.clone()).await.unwrap(),
        Some("spectra.eth".into())
    );
    state.lock().unwrap().ens_address = format!("0x{}", "55".repeat(20));
    assert_eq!(service.wallet_ens_name(approver.id).await.unwrap(), None);
}
