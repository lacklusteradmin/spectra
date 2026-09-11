//! Real stored mnemonic -> identity -> params -> signer -> mock submission.
use super::*;
use crate::store::{
    secret_backends::InMemorySecretStore, state::WalletSummary, wallet_secrets::store_seed_phrase,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

#[tokio::test]
async fn audit_stored_wallets_reach_solana_sui_aptos_and_tron_submission() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../testdata/send-audit-vectors.json")).unwrap();
    for (chain, token) in [
        (Chain::Solana, false),
        (Chain::Solana, true),
        (Chain::Sui, false),
        (Chain::Aptos, false),
        (Chain::Tron, false),
        (Chain::Tron, true),
    ] {
        let server = MockServer::start().await;
        let v = fixture.clone();
        Mock::given(any()).respond_with(move |request: &Request| {
            let body:Value=serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let path=request.url.path();
            let result = match body["method"].as_str().unwrap_or(path) {
                "getLatestBlockhash" => json!({"value":{"blockhash":v["solana"]["blockhash"]}}),
                "sendTransaction" => {
                    let tx=STANDARD.decode(body["params"][0].as_str().unwrap()).unwrap();
                    let public:[u8;32]=tx[69..101].try_into().unwrap();
                    ed25519_dalek::VerifyingKey::from_bytes(&public).unwrap().verify_strict(&tx[65..],&ed25519_dalek::Signature::from_slice(&tx[1..65]).unwrap()).unwrap();
                    json!(bs58::encode(&tx[1..65]).into_string())
                },
                "suix_getReferenceGasPrice" => json!("1000"),
                "suix_getCoins" => json!({"data":[{"coinObjectId":format!("0x{}","33".repeat(32)),"version":"7","digest":"11111111111111111111111111111111","balance":"200000000"}],"hasNextPage":false,"nextCursor":null}),
                "sui_executeTransactionBlock" => {
                    let bytes=STANDARD.decode(body["params"][0].as_str().unwrap()).unwrap();
                    assert_eq!(hex::encode(&bytes),v["sui"]["raw"]);
                    assert_eq!(body["params"][1][0],v["sui"]["signature"]);
                    json!({"digest":"11111111111111111111111111111111","effects":{"status":{"status":"success"}}})
                },
                "/" => json!({"chain_id":1,"ledger_version":"1"}),
                "/estimate_gas_price" => json!({"gas_estimate":100}),
                "/transactions" => {
                    assert_eq!(body["sender"],v["aptos"]["address"]);
                    assert_eq!(body["signature"]["public_key"],format!("0x{}",v["aptos"]["public_key"].as_str().unwrap()));
                    json!({"hash":format!("0x{}","ab".repeat(32))})
                },
                "/wallet/getnowblock" => json!({"blockID":format!("0000000000000007{}","33".repeat(24)),"block_header":{"raw_data":{"number":7}}}),
                "/wallet/broadcasttransaction" => {
                    assert_eq!(body["raw_data"]["contract"][0]["parameter"]["value"]["owner_address"],v["tron"]["transactions"][0]["raw_data"]["contract"][0]["parameter"]["value"]["owner_address"]);
                    json!({"result":true})
                },
                "/wallet/triggerconstantcontract" => {
                    let result=if body["function_selector"]=="decimals()" {format!("{:064x}",6)} else {format!("{:064x}{:064x}{:0<64}",32,4,hex::encode("TEST"))};
                    json!({"result":{"result":true},"constant_result":[result]})
                },
                p if p.starts_with("/accounts/") => json!({"sequence_number":"7"}),
                other => panic!("unexpected provider request: {other} ({body})"),
            };
            ResponseTemplate::new(200).set_body_json(if body["method"].is_string(){json!({"jsonrpc":"2.0","id":body["id"],"result":result})}else{result})
        }).mount(&server).await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: chain.str_id().into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let secrets = Arc::new(InMemorySecretStore::new());
        service.set_secret_store(secrets.clone());
        let (address, path) = match chain {
            Chain::Solana => (
                fixture["solana"]["address"].as_str().unwrap(),
                fixture["solana"]["path"].as_str().unwrap(),
            ),
            Chain::Sui => (
                fixture["sui"]["address"].as_str().unwrap(),
                "m/44'/784'/0'/0'/0'",
            ),
            Chain::Aptos => (
                fixture["aptos"]["address"].as_str().unwrap(),
                "m/44'/637'/0'/0'/0'",
            ),
            _ => (
                fixture["tron"]["from"].as_str().unwrap(),
                "m/44'/195'/0'/0/0",
            ),
        };
        service
            .apply_state_command(StateCommand::UpsertWallet {
                wallet: WalletSummary::single_address(
                    "w",
                    "Test",
                    chain.chain_display_name(),
                    address,
                    Some(path.into()),
                    false,
                ),
            })
            .await
            .unwrap();
        store_seed_phrase(&*secrets, "w", fixture["mnemonic"].as_str().unwrap(), None).unwrap();
        let destination = match chain {
            Chain::Solana => bs58::encode([0x22; 32]).into_string(),
            Chain::Tron => fixture["tron"]["to"].as_str().unwrap().into(),
            _ => format!("0x{}", "22".repeat(32)),
        };
        let request = crate::send::SendExecutionRequest {
            wallet_id: "w".into(),
            chain_id: chain.str_id().into(),
            password: None,
            to_address: destination,
            amount_str: if token || chain == Chain::Tron {
                "123.456789".into()
            } else if chain == Chain::Aptos {
                "1.23456789".into()
            } else {
                "0.123456789".into()
            },
            contract_address: if token {
                Some(if chain == Chain::Tron {
                    fixture["tron"]["contract"].as_str().unwrap().into()
                } else {
                    bs58::encode([0x44; 32]).into_string()
                })
            } else {
                None
            },
            token_decimals: token.then_some(6),
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            monero_priority: None,
            sign_only: false,
        };
        let result = service
            .execute_send(request)
            .await
            .unwrap_or_else(|e| panic!("{chain:?} token={token}: {e}"));
        assert!(!result.transaction_hash.is_empty(), "{chain:?}");
        let requests = server.received_requests().await.unwrap();
        assert!(!requests
            .iter()
            .any(|r| r.url.path().contains("encode_submission")
                || r.url.path().contains("createtransaction")
                || r.url.path().contains("triggersmartcontract")
                || String::from_utf8_lossy(&r.body).contains("unsafe_transferSui")));
    }
}
