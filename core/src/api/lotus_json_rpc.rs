//! The Lotus JSON-RPC adapter (`/rpc/v1`, the API Lotus and its gateways
//! serve): the node's network, an address's balance and next nonce, an
//! actor's Ethereum address, gas estimation, submission of a signed message
//! and its status.

use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::endpoint::EndpointApi;
use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::registry::Chain;

/// A message as Lotus writes it, which `send::filecoin` builds: amounts in
/// attoFIL as decimal digits, and no parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FilecoinMessage {
    pub to: String,
    pub from: String,
    pub nonce: u64,
    pub value: String,
    pub gas_limit: u64,
    pub gas_fee_cap: String,
    pub gas_premium: String,
    pub method: u64,
}

impl FilecoinMessage {
    /// The message in Lotus's JSON.
    fn lotus_json(&self) -> Value {
        json!({
            "Version": 0,
            "To": self.to,
            "From": self.from,
            "Nonce": self.nonce,
            "Value": self.value,
            "GasLimit": self.gas_limit,
            "GasFeeCap": self.gas_fee_cap,
            "GasPremium": self.gas_premium,
            "Method": self.method,
            "Params": null,
        })
    }
}

/// A message signed with a secp256k1 key, and the CID it lands under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FilecoinSignedMessage {
    pub message: FilecoinMessage,
    /// The recoverable signature: `r`, `s` and the recovery id, 65 bytes.
    pub signature: Vec<u8>,
}

/// `Signature.Type` of a secp256k1 signature.
const SIGNATURE_SECP256K1: u8 = 1;

impl FilecoinSignedMessage {
    /// The one parameter `Filecoin.MpoolPush` takes: the message and its
    /// signature in Lotus's JSON.
    pub(crate) fn lotus_json(&self) -> Value {
        json!({
            "Message": self.message.lotus_json(),
            "Signature": {
                "Type": SIGNATURE_SECP256K1,
                "Data": base64::engine::general_purpose::STANDARD.encode(&self.signature),
            },
        })
    }
}

/// What a message pays for gas: its limit, the most it pays per unit, and
/// the share of that the block's miner takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FilecoinGas {
    pub limit: u64,
    pub fee_cap: u128,
    pub premium: u128,
}

/// An actor in the state tree, as much of it as a send reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilecoinActor {
    /// The next nonce the actor signs with, counting only messages the chain
    /// has executed.
    pub nonce: u64,
    /// Its `f410f` address, when the Ethereum Address Manager created it:
    /// an Ethereum-style account or contract.
    pub delegated_address: Option<String>,
}

pub struct LotusClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl LotusClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            EndpointApi::LotusJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }

    /// Refuse a node that is not on `chain`: it must name the network and
    /// the genesis block. A message binds neither, so nothing else would
    /// tell a signature for mainnet from one for a test network.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let name = chain.filecoin_network_name()?;
        let genesis = chain.filecoin_genesis_cid()?;
        race(&self.endpoints, |base| async move {
            let node = LotusClient::new(Arc::new(vec![base]));
            let network = node.call("Filecoin.StateNetworkName", json!([])).await?;
            let network = network
                .as_str()
                .or_decode("Lotus network name: not a string")?;
            if network != name {
                return Err(ApiError::invalid(format!(
                    "Endpoint is on {network}, not {name}"
                )));
            }
            let block = node.call("Filecoin.ChainGetGenesis", json!([])).await?;
            if block["Cids"][0]["/"].as_str() != Some(genesis) {
                return Err(ApiError::invalid("Endpoint has another genesis block"));
            }
            Ok(())
        })
        .await
    }

    /// The address's balance in attoFIL: zero where no actor exists yet.
    pub(crate) async fn fetch_balance(&self, address: &str) -> Result<u128, ApiError> {
        let balance = self
            .call("Filecoin.WalletBalance", json!([address]))
            .await?;
        attofil(&balance).or_decode("Lotus balance: not a whole number of attoFIL")
    }

    /// The nonce the address's next message takes, counting the messages
    /// the node's pool holds for it.
    pub(crate) async fn fetch_next_nonce(&self, address: &str) -> Result<u64, ApiError> {
        self.call("Filecoin.MpoolGetNonce", json!([address]))
            .await?
            .as_u64()
            .or_decode("Lotus nonce: not a number")
    }

    /// The actor at `address`, or `None` where the state tree has none.
    pub(crate) async fn fetch_actor(
        &self,
        address: &str,
    ) -> Result<Option<FilecoinActor>, ApiError> {
        let actor = match self
            .call("Filecoin.StateGetActor", json!([address, []]))
            .await
        {
            Ok(actor) => actor,
            Err(error) if is_actor_not_found(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        actor_from(&actor)
    }

    /// The gas `message` needs, as the node estimates it: a limit from
    /// executing it, a fee cap and a premium, held to `max_fee` in all.
    pub(crate) async fn estimate_gas(
        &self,
        message: &FilecoinMessage,
        max_fee: u128,
    ) -> Result<FilecoinGas, ApiError> {
        let estimated = self
            .call(
                "Filecoin.GasEstimateMessageGas",
                json!([message.lotus_json(), {"MaxFee": max_fee.to_string()}, []]),
            )
            .await?;
        gas_from(&estimated, message)
    }

    /// Submit a signed message and answer the CID the node took it under.
    pub(crate) async fn broadcast(
        &self,
        signed: &FilecoinSignedMessage,
    ) -> Result<String, ApiError> {
        let body = Arc::new(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "Filecoin.MpoolPush",
            "params": [signed.lotus_json()],
        }));
        race(&self.endpoints, |url| {
            let client = Arc::clone(&self.client);
            let body = Arc::clone(&body);
            async move {
                let response: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainWrite)
                    .await?;
                pushed_cid(&response)
            }
        })
        .await
    }

    /// Whether the message `cid` has executed, and whether it succeeded. A
    /// message the chain does not hold is still pending; one another
    /// message with its nonce replaced is never found, and its nonce, not
    /// its status, says it can no longer land.
    pub(crate) async fn fetch_transaction_status(
        &self,
        cid: &str,
    ) -> Result<TransactionStatus, ApiError> {
        if !crate::derivation::filecoin::is_message_cid(cid) {
            return Err(ApiError::invalid("Invalid message CID"));
        }
        let found = self
            .call(
                "Filecoin.StateSearchMsg",
                json!([[], {"/": cid}, -1, false]),
            )
            .await?;
        message_status(&found, cid)
    }
}

/// A non-negative whole number of attoFIL, as Lotus writes one: digits in a
/// string.
fn attofil(value: &Value) -> Option<u128> {
    let digits = value.as_str()?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// Whether a node answered that no actor exists at the address.
fn is_actor_not_found(error: &ApiError) -> bool {
    matches!(error, ApiError::Rejected(message) if message.contains("actor not found"))
}

/// The actor a `StateGetActor` answer describes; `null` is none.
fn actor_from(actor: &Value) -> Result<Option<FilecoinActor>, ApiError> {
    if actor.is_null() {
        return Ok(None);
    }
    let nonce = actor["Nonce"]
        .as_u64()
        .or_decode("Lotus actor: missing nonce")?;
    let delegated_address = match &actor["DelegatedAddress"] {
        Value::Null => None,
        Value::String(address) => Some(address.clone()),
        _ => {
            return Err(ApiError::decode(
                "Lotus actor: delegated address is not a string",
            ));
        }
    };
    Ok(Some(FilecoinActor {
        nonce,
        delegated_address,
    }))
}

/// The gas a `GasEstimateMessageGas` answer fills in, from an answer about
/// `asked` and no other message.
fn gas_from(estimated: &Value, asked: &FilecoinMessage) -> Result<FilecoinGas, ApiError> {
    if estimated["To"].as_str() != Some(asked.to.as_str())
        || estimated["From"].as_str() != Some(asked.from.as_str())
        || estimated["Value"].as_str() != Some(asked.value.as_str())
        || estimated["Method"].as_u64() != Some(asked.method)
    {
        return Err(ApiError::decode("Lotus gas estimate: another message"));
    }
    Ok(FilecoinGas {
        limit: estimated["GasLimit"]
            .as_u64()
            .filter(|limit| *limit > 0)
            .or_decode("Lotus gas estimate: missing gas limit")?,
        fee_cap: attofil(&estimated["GasFeeCap"])
            .or_decode("Lotus gas estimate: missing fee cap")?,
        premium: attofil(&estimated["GasPremium"])
            .or_decode("Lotus gas estimate: missing premium")?,
    })
}

/// The CID an `MpoolPush` answer names. An error object is the node's
/// refusal: the message is not in its pool.
fn pushed_cid(response: &Value) -> Result<String, ApiError> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(ApiError::rejected(format!(
            "Lotus node refused the message: {}",
            error["message"].as_str().unwrap_or_default()
        )));
    }
    let cid = response["result"]["/"]
        .as_str()
        .or_decode("Lotus push: missing message CID")?;
    if !crate::derivation::filecoin::is_message_cid(cid) {
        return Err(ApiError::decode("Lotus push: not a message CID"));
    }
    Ok(cid.to_string())
}

/// What a `StateSearchMsg` answer says of the message `cid`.
fn message_status(found: &Value, cid: &str) -> Result<TransactionStatus, ApiError> {
    if found.is_null() {
        return Ok(TransactionStatus::Pending);
    }
    if found["Message"]["/"].as_str() != Some(cid) {
        return Err(ApiError::decode("Lotus status: another message"));
    }
    let exit_code = found["Receipt"]["ExitCode"]
        .as_i64()
        .or_decode("Lotus status: missing exit code")?;
    let height = found["Height"]
        .as_u64()
        .filter(|height| *height > 0)
        .or_decode("Lotus status: missing height")?;
    Ok(TransactionStatus::Confirmed {
        succeeded: exit_code == 0,
        block: Some(height),
    })
}

#[cfg(test)]
#[path = "tests/lotus_json_rpc.rs"]
mod tests;
