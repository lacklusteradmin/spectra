//! A token send on each account network reads the token's own precision and
//! balances, carries the token in its own protocol's payload, and is signed
//! only while what was reviewed still holds: the account's sequence or the
//! coin objects it spends, the native coin for its fee, the token itself,
//! and on TON the jetton wallet it moves the token from.
use super::send_stage_support::{Secret, Wallet, node, node_at, refusal};
use super::*;
use crate::send::stages::{SendArtifact, SendStage};
use serde_json::{Value, json};
use std::sync::Mutex;

const KEY: &str = "0101010101010101010101010101010101010101010101010101010101010101";
const TOKEN_BALANCE: u128 = 223_456_789;
/// The token's raw amount `123.456789` sends at six places.
const AMOUNT: &str = "123.456789";

/// What the nodes read: the token's precision, the version of the token's
/// Sui coin object, the account's sequence, whether TON's indexer names
/// another owner for the jetton wallet, and the balances.
struct Live {
    decimals: u8,
    version: u64,
    seqno: u64,
    wrong_owner: bool,
    native: u128,
    token: u128,
}

impl Default for Live {
    fn default() -> Self {
        Self {
            decimals: 6,
            version: 8,
            seqno: 7,
            wrong_owner: false,
            native: 10_000_000_000,
            token: TOKEN_BALANCE,
        }
    }
}

/// One loopback host for every network's API: Sui and EVM JSON-RPC, the
/// Aptos REST API, TON Center v2 and, under `/v3`, TON Center v3.
async fn nodes(contract: String, live: Arc<Mutex<Live>>) -> wiremock::MockServer {
    node_at(move |url, body| {
        let live = live.lock().unwrap();
        let rpc = |call: &Value| {
            let params = &call["params"];
            let result = match call["method"].as_str().unwrap() {
                "sui_getChainIdentifier" => json!("35834a8a"),
                "sui_getCheckpoint" => json!({"sequenceNumber": "0",
                    "digest": "4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S"}),
                "suix_getCoinMetadata" => json!({"decimals": live.decimals}),
                "suix_getReferenceGasPrice" => json!("1000"),
                "suix_getBalance" => json!({"totalBalance": if params[1] == "0x2::sui::SUI" {
                    live.native.to_string() } else { live.token.to_string() }}),
                "suix_getCoins" => {
                    let native = params[1] == "0x2::sui::SUI";
                    json!({"data": [{
                        "coinObjectId": format!("0x{}", if native { "33" } else { "44" }.repeat(32)),
                        "version": if native { 7 } else { live.version }.to_string(),
                        "digest": "1".repeat(32),
                        "balance": if native { live.native } else { live.token }.to_string()}],
                        "hasNextPage": false, "nextCursor": null})
                }
                "eth_call" => {
                    let data = params[0]["data"].as_str().unwrap();
                    match &data[..10] {
                        "0x313ce567" => json!(format!("0x{:x}", live.decimals)),
                        "0x95d89b41" => json!(format!("0x{:0<64}", hex::encode("TEST"))),
                        // No ERC-165: a fungible token, not a collection.
                        "0x01ffc9a7" => json!(format!("0x{}", "0".repeat(64))),
                        _ => json!(format!("0x{:x}", live.token)),
                    }
                }
                "eth_chainId" => json!("0x3d"),
                "eth_getBalance" => json!(format!("0x{:x}", live.native * 1_000_000_000)),
                "eth_estimateGas" => json!("0x186a0"),
                "eth_getCode" => json!("0x6000"),
                "eth_getTransactionCount" => json!(format!("0x{:x}", live.seqno)),
                "eth_blockNumber" => json!("0x123"),
                "eth_gasPrice" => json!("0x3b9aca00"),
                "eth_feeHistory" => {
                    json!({"baseFeePerGas": ["0x3b9aca00"], "reward": [["0x77359400"]]})
                }
                other => panic!("unexpected JSON-RPC request {other}"),
            };
            json!({"jsonrpc": "2.0", "id": call["id"], "result": result})
        };
        if let Some(batch) = body.as_array() {
            return Some(json!(batch.iter().map(rpc).collect::<Vec<_>>()));
        }
        // TON Center v2 names its get-method `method` too; JSON-RPC is posted
        // to the root.
        if url.path() == "/" && body.get("method").is_some() {
            return Some(rpc(body));
        }
        let query = |key: &str| {
            url.query_pairs()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.into_owned())
        };
        Some(match url.path() {
            "/" => json!({"chain_id": 1, "ledger_version": "1"}),
            "/estimate_gas_price" => json!({"gas_estimate": 100}),
            path if path.starts_with("/accounts/") => {
                json!({"sequence_number": live.seqno.to_string()})
            }
            "/view" if body["function"].as_str().unwrap().ends_with("::decimals") => {
                json!([live.decimals])
            }
            "/view" if body["type_arguments"] == json!(["0x1::aptos_coin::AptosCoin"]) => {
                json!([live.native.to_string()])
            }
            "/view" => json!([live.token.to_string()]),
            "/getAddressBalance" => json!({"ok": true, "result": live.native.to_string()}),
            "/getMasterchainInfo" => {
                let (root, file) = Chain::Ton.ton_zero_state().unwrap();
                json!({"ok": true, "result": {"init": {"workchain": -1, "seqno": 0,
                    "root_hash": root, "file_hash": file}}})
            }
            "/getAddressInformation" => json!({"ok": true, "result": {"state": "active"}}),
            "/runGetMethod" => json!({"ok": true, "result": {"exit_code": 0,
                "stack": [["num", format!("0x{:x}", live.seqno)]]}}),
            "/v3/masterchainInfo" => json!({"first": {"workchain": -1,
                "shard": "8000000000000000", "seqno": 1, "global_id": -239,
                "root_hash": "8GYhhrigd8CwZGrRT59iulLDcgiTYuvOAzFJxugc0Ts=",
                "file_hash": "V+XzykEwun4yePZhAEPZk77RbMfMOgS/S4GiJkSKY6s="}}),
            "/v3/jetton/masters" => json!({"jetton_masters":
                [{"jetton_content": {"decimals": live.decimals.to_string()}}]}),
            "/v3/jetton/wallets" => {
                let owner = if live.wrong_owner {
                    format!("0:{}", "55".repeat(32))
                } else {
                    query("owner_address").unwrap()
                };
                json!({"jetton_wallets": [{"address": format!("0:{}", "33".repeat(32)),
                    "owner": owner, "jetton": contract, "balance": live.token.to_string()}],
                    "metadata": {contract.as_str(): {"token_info": [{"type": "jetton_masters",
                        "extra": {"decimals": live.decimals.to_string()}}]}}})
            }
            other => panic!("unexpected request {other}"),
        })
    })
    .await
}

async fn token_wallet(
    chain: Chain,
    contract: &str,
) -> (Wallet, wiremock::MockServer, Arc<Mutex<Live>>) {
    let live = Arc::new(Mutex::new(Live::default()));
    let server = nodes(contract.into(), live.clone()).await;
    let wallet = Wallet::import(chain, &server.uri(), Secret::Key(KEY)).await;
    if chain == Chain::Ton {
        wallet
            .use_only("toncenter-v3", &format!("{}/v3", server.uri()))
            .await;
    }
    (wallet, server, live)
}

fn destination(chain: Chain) -> String {
    match chain {
        Chain::Ton => format!("0:{}", "22".repeat(32)),
        chain if chain.is_evm() => format!("0x{}", "22".repeat(20)),
        _ => format!("0x{}", "22".repeat(32)),
    }
}

fn send(wallet: &Wallet, contract: &str) -> crate::send::SendExecutionRequest {
    wallet.token_request(&destination(wallet.chain), AMOUNT, contract, 6)
}

type Change = fn(&mut Live);

/// Under each change the build is refused with its words and stores
/// nothing.
async fn refused_at_build(
    wallet: &Wallet,
    contract: &str,
    live: &Mutex<Live>,
    cases: &[(Change, &str)],
) {
    for (change, words) in cases {
        *live.lock().unwrap() = Live::default();
        change(&mut live.lock().unwrap());
        let error = refusal(wallet.build(send(wallet, contract)).await);
        assert!(error.contains(words), "{:?} {words}: {error}", wallet.chain);
        assert!(wallet.built_nothing().await, "{words}");
    }
    *live.lock().unwrap() = Live::default();
}

/// Under each change signing is refused with its words and leaves the
/// reviewed transaction unsigned; with none, it signs.
async fn refused_at_signing(
    wallet: &Wallet,
    built: &SendArtifact,
    live: &Mutex<Live>,
    cases: &[(Change, &str)],
) {
    for (change, words) in cases {
        *live.lock().unwrap() = Live::default();
        change(&mut live.lock().unwrap());
        let error = refusal(wallet.sign(built).await);
        assert!(error.contains(words), "{:?} {words}: {error}", wallet.chain);
        assert_eq!(wallet.stored(built).await.stage, SendStage::Prepared);
    }
    *live.lock().unwrap() = Live::default();
    assert_eq!(wallet.sign(built).await.unwrap().stage, SendStage::Signed);
}

fn prepared(built: &SendArtifact) -> Value {
    serde_json::from_str(&built.prepared_details).unwrap()
}

/// A Sui token send spends the token's coin object and pays gas from a SUI
/// one; either object changing since the review is a different transaction.
#[tokio::test]
async fn a_sui_token_send_signs_only_the_coin_objects_it_was_reviewed_with() {
    let contract = format!("0x{}::coins::USD", "44".repeat(32));
    let (wallet, _server, live) = token_wallet(Chain::Sui, &contract).await;
    refused_at_build(
        &wallet,
        &contract,
        &live,
        &[(|l| l.decimals = 7, "Token decimals changed")],
    )
    .await;
    let built = wallet.build(send(&wallet, &contract)).await.unwrap();
    assert_eq!(
        prepared(&built)["Sui"]["objects"].as_array().unwrap().len(),
        2
    );
    refused_at_signing(
        &wallet,
        &built,
        &live,
        &[
            (|l| l.version += 1, "Sui objects or gas changed"),
            (|l| l.native = 0, "Insufficient 0x2::sui::SUI coin objects"),
            (|l| l.token = 0, "::coins::USD coin objects"),
        ],
    )
    .await;
}

/// An Aptos legacy coin moves through `coin::transfer`, a fungible asset
/// through its primary store; the account's sequence, its APT for gas and
/// the token are each read again before signing.
#[tokio::test]
async fn an_aptos_token_send_calls_its_standards_transfer_and_rechecks_the_account() {
    for (contract, function) in [
        (
            format!("0x{}::coins::USD", "44".repeat(32)),
            "0x1::coin::transfer",
        ),
        (
            format!("0x{}", "44".repeat(32)),
            "0x1::primary_fungible_store::transfer",
        ),
    ] {
        let (wallet, _server, live) = token_wallet(Chain::Aptos, &contract).await;
        refused_at_build(
            &wallet,
            &contract,
            &live,
            &[(|l| l.decimals = 7, "Token decimals changed")],
        )
        .await;
        let built = wallet.build(send(&wallet, &contract)).await.unwrap();
        let payload = &prepared(&built)["Aptos"]["body"]["payload"];
        assert_eq!(payload["function"], function);
        assert_eq!(
            payload["arguments"].as_array().unwrap().last().unwrap(),
            "123456789"
        );
        refused_at_signing(
            &wallet,
            &built,
            &live,
            &[
                (|l| l.seqno += 1, "Aptos sequence or expiration is stale"),
                (
                    |l| l.native = 0,
                    "Insufficient APT for the reviewed gas budget",
                ),
                (|l| l.token = 0, "Insufficient Aptos token balance"),
            ],
        )
        .await;
    }
}

/// A jetton moves from the sender's jetton wallet, which the indexer must
/// name as the sender's for the reviewed master; its preview quotes the
/// network fee and the 0.1 TON attached to that wallet.
#[tokio::test]
async fn a_jetton_send_moves_from_the_senders_own_jetton_wallet() {
    let contract = format!("0:{}", "44".repeat(32));
    let (wallet, _server, live) = token_wallet(Chain::Ton, &contract).await;
    let holding = wallet
        .hold_token("TEP-74", &contract, 6, "223.456789")
        .await;
    let preview = wallet
        .service
        .preview_owned_send(
            wallet.id.clone(),
            holding,
            AMOUNT.into(),
            String::new(),
            None,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(preview.network_fee.as_deref(), Some("0.107"));
    assert_eq!(
        preview
            .details
            .as_ref()
            .and_then(|details| details.max_sendable.as_deref()),
        Some("223.456789")
    );
    let mismatch = "Jetton source wallet owner or master mismatch";
    refused_at_build(
        &wallet,
        &contract,
        &live,
        &[
            (|l| l.decimals = 7, "Token decimals changed"),
            (|l| l.wrong_owner = true, mismatch),
        ],
    )
    .await;
    let built = wallet.build(send(&wallet, &contract)).await.unwrap();
    assert_eq!(prepared(&built)["Ton"]["jetton"]["master"], contract);
    refused_at_signing(
        &wallet,
        &built,
        &live,
        &[
            (|l| l.seqno += 1, "TON sequence or expiration changed"),
            (
                |l| l.native = 0,
                "Insufficient TON for the reviewed transfer and network fee",
            ),
            (|l| l.token = 0, "Insufficient jetton balance"),
            (|l| l.wrong_owner = true, mismatch),
        ],
    )
    .await;
}

/// An ERC-20 send reads the contract's precision, and is signed only at the
/// reviewed nonce while the account holds the token and the gas for it.
#[tokio::test]
async fn an_erc20_send_is_signed_only_while_the_reviewed_nonce_and_funds_hold() {
    let contract = format!("0x{}", "44".repeat(20));
    let (wallet, _server, live) = token_wallet(Chain::EthereumClassic, &contract).await;
    refused_at_build(
        &wallet,
        &contract,
        &live,
        &[(|l| l.decimals = 7, "Token decimals do not match")],
    )
    .await;
    let built = wallet.build(send(&wallet, &contract)).await.unwrap();
    refused_at_signing(
        &wallet,
        &built,
        &live,
        &[
            (|l| l.seqno += 1, "Prepared nonce or network is stale"),
            (|l| l.native = 0, "Insufficient funds"),
            (|l| l.token = 0, "Insufficient funds"),
        ],
    )
    .await;
}

/// A Token-2022 mint's transfer fee is part of the reviewed transfer: raised
/// before signing, what the recipient receives is no longer what was
/// reviewed.
#[tokio::test]
async fn a_token_2022_fee_raised_after_review_refuses_signing() {
    let t22: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/solana-token-2022.json"
    ))
    .unwrap();
    let program = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
    let basis_points = Arc::new(Mutex::new(50u64));
    let fee = basis_points.clone();
    let fixture = t22.clone();
    let server = node(move |_, body| {
        let params = &body["params"];
        let text = |key: &str| fixture[key].as_str().unwrap().to_string();
        let result = match body["method"].as_str().unwrap() {
            "getGenesisHash" => json!(Chain::Solana.solana_genesis_hash().unwrap()),
            "getAccountInfo" => {
                let address = params[0].as_str().unwrap();
                let raw = params[1]["encoding"] == "base64";
                let bps = *fee.lock().unwrap();
                let rate = json!({"epoch": 0, "transferFeeBasisPoints": bps, "maximumFee": 1_000_000_000_000u64});
                let value = if address == text("mint") && !raw {
                    json!({"owner": program, "data": {"parsed": {"type": "mint", "info": {
                        "isInitialized": true, "decimals": 6, "extensions": [{
                            "extension": "transferFeeConfig", "state": {"withheldAmount": 0,
                            "olderTransferFee": rate, "newerTransferFee": rate}}]}}}})
                } else if address == text("source") && raw {
                    json!({"owner": program, "data": [fixture["source_data"], "base64"]})
                } else if address == text("source") {
                    json!({"owner": program, "data": {"parsed": {"type": "account", "info": {
                        "mint": fixture["mint"], "owner": fixture["owner"], "state": "initialized",
                        "extensions": [], "tokenAmount": {"amount": "5000000", "decimals": 6}}}}})
                } else {
                    Value::Null
                };
                json!({"context": {"slot": 1}, "value": value})
            }
            "getEpochInfo" => json!({"epoch": 700, "slotIndex": 1_000, "slotsInEpoch": 432_000,
                "absoluteSlot": 1}),
            "getLatestBlockhash" => json!({"value": {"blockhash": bs58::encode([5u8; 32]).into_string(),
                "lastValidBlockHeight": 100}}),
            "simulateTransaction" => json!({"value": {"err": null, "logs": []}}),
            "isBlockhashValid" => json!({"value": true}),
            other => panic!("unexpected Solana request {other}"),
        };
        Some(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
    })
    .await;
    let wallet = Wallet::import(Chain::Solana, &server.uri(), Secret::Key(KEY)).await;
    assert_eq!(wallet.address, t22["owner"].as_str().unwrap());
    let built = wallet
        .build(wallet.token_request(
            t22["recipient"].as_str().unwrap(),
            "1",
            t22["mint"].as_str().unwrap(),
            6,
        ))
        .await
        .unwrap();
    let terms = built.review.transfer_terms.clone().unwrap();
    assert_eq!(
        (
            terms.debited.as_str(),
            terms.received.as_str(),
            terms.fee.as_str()
        ),
        ("1", "0.995", "0.005")
    );
    *basis_points.lock().unwrap() = 100;
    let error = refusal(wallet.sign(&built).await);
    assert!(
        error.contains("transfer fee or hook accounts changed"),
        "{error}"
    );
    assert_eq!(wallet.stored(&built).await.stage, SendStage::Prepared);
    *basis_points.lock().unwrap() = 50;
    assert_eq!(wallet.sign(&built).await.unwrap().stage, SendStage::Signed);
}
