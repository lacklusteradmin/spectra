//! A Sui GraphQL node's answers, in the shapes the adapter reads, for the
//! tests of core's Sui flows: each function is one operation's `data`.
use serde_json::{Value, json};
use wiremock::{Request, ResponseTemplate};

use crate::registry::Chain;

/// The operation a request names, and its variables.
pub(crate) fn operation(request: &Request) -> (String, Value) {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    (
        body["operationName"].as_str().unwrap().to_string(),
        body["variables"].clone(),
    )
}

/// An answer of `data`.
pub(crate) fn respond(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"data": data}))
}

/// `Chain`: the network's identifier and a latest checkpoint.
pub(crate) fn chain(chain: Chain) -> Value {
    json!({"chainIdentifier": chain.sui_genesis_checkpoint().unwrap(),
        "checkpoint": {"sequenceNumber": 100}})
}

/// `Epoch`.
pub(crate) fn epoch(epoch: u64, gas_price: u64) -> Value {
    json!({"epoch": {"epochId": epoch, "referenceGasPrice": gas_price.to_string()}})
}

/// `Balance`, all of it in coin objects.
pub(crate) fn balance(total: u128) -> Value {
    json!({"address": {"balance": {"totalBalance": total.to_string(),
        "coinBalance": total.to_string()}}})
}

/// `CoinMetadata`.
pub(crate) fn metadata(decimals: u8) -> Value {
    json!({"coinMetadata": {"decimals": decimals}})
}

/// An object with `json` as its contents, as `Objects` lists it.
pub(crate) fn object(id: &str, version: u64, type_: &str, json: Value) -> Value {
    json!({"address": id, "version": version, "digest": "1".repeat(32),
        "contents": {"type": {"repr": type_}, "json": json}})
}

/// A `Coin<coin_type>` object.
pub(crate) fn coin(id: &str, version: u64, coin_type: &str, balance: u64) -> Value {
    object(
        id,
        version,
        &format!("0x2::coin::Coin<{coin_type}>"),
        json!({"id": id, "balance": balance.to_string()}),
    )
}

fn address_owner(owner: &str) -> Value {
    json!({"__typename": "AddressOwner", "address": {"address": owner}})
}

/// `Objects`: one page holding every object, each held by `owner`.
pub(crate) fn objects(owner: &str, nodes: Vec<Value>) -> Value {
    let nodes: Vec<Value> = nodes
        .into_iter()
        .map(|mut node| {
            node["owner"] = address_owner(owner);
            node
        })
        .collect();
    json!({"address": {"objects": {"pageInfo": {"hasNextPage": false, "endCursor": null},
        "nodes": nodes}}})
}

/// `Object`: `listed`, an object as `Objects` lists it, held by `owner`.
pub(crate) fn owned(listed: Value, owner: &str) -> Value {
    json!({"object": {
        "address": listed["address"], "version": listed["version"], "digest": listed["digest"],
        "owner": address_owner(owner), "asMoveObject": {"contents": listed["contents"]}}})
}

/// `Object`: the Sui system state, shared at version 1.
pub(crate) fn system_state() -> Value {
    json!({"object": {"address": "0x5", "version": 9, "digest": "1".repeat(32),
        "owner": {"__typename": "Shared", "initialSharedVersion": 1},
        "asMoveObject": {"contents": {"type": {"repr": "0x3::sui_system::SuiSystemState"},
            "json": {}}}}})
}

/// `Validators`: one page of validators, each `(address, pool id)`.
pub(crate) fn validators(validators: &[(&str, &str)]) -> Value {
    let nodes: Vec<Value> = validators
        .iter()
        .map(|(address, pool)| {
            json!({"contents": {"json": {
                "metadata": {"sui_address": address, "name": "Fixture validator",
                    "description": "", "project_url": ""},
                "commission_rate": "500",
                "staking_pool": {"id": pool, "sui_balance": "2000000000"}}}})
        })
        .collect();
    json!({"epoch": {"validatorSet": {"activeValidators": {
        "pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": nodes}}}})
}

/// A `StakedSui` object.
pub(crate) fn staked(id: &str, version: u64, pool: &str, epoch: u64, principal: u64) -> Value {
    object(
        id,
        version,
        "0x3::staking_pool::StakedSui",
        json!({"id": id, "pool_id": pool, "stake_activation_epoch": epoch.to_string(),
            "principal": principal.to_string()}),
    )
}

/// `Simulate`: success or failure, at these computation, storage and rebate
/// costs.
pub(crate) fn simulated(success: bool, costs: [u64; 3]) -> Value {
    json!({"simulateTransaction": {"effects": {
        "status": if success { "SUCCESS" } else { "FAILURE" },
        "executionError": if success { Value::Null } else { json!({"message": "refused"}) },
        "gasEffects": {"gasSummary": {"computationCost": costs[0], "storageCost": costs[1],
            "storageRebate": costs[2]}}}}})
}

/// `Execute`: `digest` executed and succeeded.
pub(crate) fn executed(digest: &str) -> Value {
    json!({"executeTransaction": {"effects": {"digest": digest, "status": "SUCCESS",
        "executionError": null}}})
}
