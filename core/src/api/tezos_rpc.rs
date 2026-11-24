//! A Tezos node's RPC: the network's chain id and protocol, the block an
//! operation is branched from, an implicit account's balance, counter and
//! revealed key, a simulation of manager operations, and injection of a
//! signed operation.
//!
//! What is signed is forged locally (`send::tezos`); a node's `forge` RPC is
//! never asked. A simulation is a read: it takes the operations as JSON with
//! a signature of zeros, which it does not check, so no signed operation
//! reaches a node before it is broadcast.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::derivation::tezos::{base58check, decode_base58check, prefix};
use crate::registry::Chain;

/// A block an operation can be branched from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TezosBlock {
    pub hash: String,
    pub level: u64,
}

/// An implicit account as the chain holds it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TezosAccount {
    /// What it can spend, in mutez: tez frozen by staking are not in it.
    pub balance: u128,
    /// The counter of its last manager operation; the next is one more.
    /// An account the chain does not hold answers the global counter, which
    /// is where its own starts once it is created.
    pub counter: u64,
    /// Whether its public key is on chain, so its operations need no
    /// `reveal` first.
    pub revealed: bool,
}

/// A manager operation as a node's simulation takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TezosContent {
    Reveal {
        source: String,
        fee: u64,
        counter: u64,
        gas_limit: u64,
        storage_limit: u64,
        /// The key revealed, as `edpk…`.
        public_key: String,
    },
    Transaction {
        source: String,
        fee: u64,
        counter: u64,
        gas_limit: u64,
        storage_limit: u64,
        amount: u64,
        destination: String,
    },
}

impl TezosContent {
    fn kind(&self) -> &'static str {
        match self {
            Self::Reveal { .. } => "reveal",
            Self::Transaction { .. } => "transaction",
        }
    }

    /// The operation in the node's JSON encoding, numbers as strings.
    fn to_json(&self) -> Value {
        match self {
            Self::Reveal {
                source,
                fee,
                counter,
                gas_limit,
                storage_limit,
                public_key,
            } => json!({
                "kind": "reveal",
                "source": source,
                "fee": fee.to_string(),
                "counter": counter.to_string(),
                "gas_limit": gas_limit.to_string(),
                "storage_limit": storage_limit.to_string(),
                "public_key": public_key,
            }),
            Self::Transaction {
                source,
                fee,
                counter,
                gas_limit,
                storage_limit,
                amount,
                destination,
            } => json!({
                "kind": "transaction",
                "source": source,
                "fee": fee.to_string(),
                "counter": counter.to_string(),
                "gas_limit": gas_limit.to_string(),
                "storage_limit": storage_limit.to_string(),
                "amount": amount.to_string(),
                "destination": destination,
            }),
        }
    }
}

/// What a simulation says one manager operation consumed, its internal
/// operations included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TezosConsumption {
    pub milligas: u64,
    /// Bytes the contracts it called added to their storage.
    pub paid_storage: u64,
    /// Accounts and contracts it created, each of which burns the
    /// protocol's origination size.
    pub allocations: u64,
}

impl TezosConsumption {
    /// The gas it used, in the whole units a gas limit names.
    pub(crate) fn gas(&self) -> u64 {
        self.milligas.div_ceil(1_000)
    }
}

/// The signature a simulated operation carries: 64 zero bytes, which a
/// node does not check, as Taquito sends them.
fn simulation_signature() -> String {
    base58check(&prefix::EDSIG, &[0; 64])
}

pub struct TezosClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl TezosClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    /// Refuse a node that is not on `chain`: its chain id must be the one
    /// every operation is checked against before it is signed.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let expected = chain.tezos_network()?.chain_id;
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            async move {
                let id: Value = client
                    .get_json(
                        &format!("{}/chains/main/chain_id", base.trim_end_matches('/')),
                        RetryProfile::ChainRead,
                    )
                    .await?;
                let id = id.as_str().or_decode("Tezos chain_id: not a string")?;
                if id != expected {
                    return Err(ApiError::invalid(format!(
                        "Endpoint is on {id}, not {expected}"
                    )));
                }
                Ok(())
            }
        })
        .await
    }

    /// The protocol the head was applied with, and the one the next block
    /// will be: different only at the last block before an upgrade.
    pub(crate) async fn fetch_protocols(&self) -> Result<(String, String), ApiError> {
        let answer = self.get("/chains/main/blocks/head/protocols").await?;
        let read = |field: &str| {
            answer[field]
                .as_str()
                .map(str::to_string)
                .or_decode("Tezos protocols: missing protocol")
        };
        Ok((read("protocol")?, read("next_protocol")?))
    }

    /// The block `depth` levels below the head.
    pub(crate) async fn fetch_block(&self, depth: u64) -> Result<TezosBlock, ApiError> {
        let block = match depth {
            0 => "head".to_string(),
            depth => format!("head~{depth}"),
        };
        let header = self
            .get(&format!("/chains/main/blocks/{block}/header"))
            .await?;
        block_from(&header)
    }

    /// The level of the newest block the node has.
    pub(crate) async fn fetch_head_level(&self) -> Result<u64, ApiError> {
        Ok(self.fetch_block(0).await?.level)
    }

    /// An implicit account's balance, counter and whether its key is
    /// revealed.
    pub(crate) async fn fetch_account(&self, address: &str) -> Result<TezosAccount, ApiError> {
        let answer = self
            .get(&format!(
                "/chains/main/blocks/head/context/contracts/{address}"
            ))
            .await?;
        account_from(&answer)
    }

    /// What `address` can spend, in mutez: an implicit account's or a
    /// contract's balance. An account the chain does not hold has none.
    pub(crate) async fn fetch_balance(&self, address: &str) -> Result<u128, ApiError> {
        let answer = self
            .get(&format!(
                "/chains/main/blocks/head/context/contracts/{address}/balance"
            ))
            .await?;
        mutez(&answer).or_decode("Tezos balance: not an amount")
    }

    /// Simulate `contents`, branched from `branch`, on `chain_id`: what each
    /// operation consumed. Refused, with the node's error ids, when the
    /// network would not apply every one of them, or when the node refuses
    /// the operation outright: an HTTP 500 whose body lists its errors.
    pub(crate) async fn simulate(
        &self,
        chain_id: &str,
        branch: &str,
        contents: &[TezosContent],
    ) -> Result<Vec<TezosConsumption>, ApiError> {
        let body = Arc::new(json!({
            "operation": {
                "branch": branch,
                "contents": contents.iter().map(TezosContent::to_json).collect::<Vec<_>>(),
                "signature": simulation_signature(),
            },
            "chain_id": chain_id,
        }));
        let answer: Value = race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = Arc::clone(&body);
            async move {
                client
                    .post_json(
                        &format!(
                            "{}/chains/main/blocks/head/helpers/scripts/simulate_operation",
                            base.trim_end_matches('/')
                        ),
                        &*body,
                        RetryProfile::GatewayRead,
                    )
                    .await
            }
        })
        .await
        .map_err(|error| match error {
            ApiError::Status { status: 500, body } => refusal_from(&body),
            error => error,
        })?;
        consumption_from(&answer, contents)
    }

    /// Inject a signed operation and answer the hash the node accepted it
    /// under.
    pub(crate) async fn broadcast(&self, signed_hex: &str) -> Result<String, ApiError> {
        let body = Arc::new(Value::String(signed_hex.to_string()));
        let answer: Value = race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = Arc::clone(&body);
            async move {
                client
                    .post_json(
                        &format!(
                            "{}/injection/operation?chain=main",
                            base.trim_end_matches('/')
                        ),
                        &*body,
                        RetryProfile::ChainWrite,
                    )
                    .await
            }
        })
        .await?;
        let hash = answer
            .as_str()
            .or_decode("Tezos injection: no operation hash")?;
        decode_base58check(hash, &prefix::OPERATION, 32)
            .or_decode("Tezos injection: not an operation hash")?;
        Ok(hash.to_string())
    }
}

/// A whole number of mutez, written as a JSON string of digits.
fn mutez(value: &Value) -> Option<u128> {
    value
        .as_str()
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))?
        .parse()
        .ok()
}

fn block_from(header: &Value) -> Result<TezosBlock, ApiError> {
    let hash = header["hash"]
        .as_str()
        .filter(|hash| decode_base58check(hash, &prefix::BLOCK, 32).is_some())
        .or_decode("Tezos block: missing hash")?;
    let level = header["level"]
        .as_u64()
        .or_decode("Tezos block: missing level")?;
    Ok(TezosBlock {
        hash: hash.to_string(),
        level,
    })
}

fn account_from(answer: &Value) -> Result<TezosAccount, ApiError> {
    Ok(TezosAccount {
        balance: mutez(&answer["balance"]).or_decode("Tezos account: missing balance")?,
        counter: answer["counter"]
            .as_str()
            .and_then(|counter| counter.parse().ok())
            .or_decode("Tezos account: missing counter")?,
        revealed: answer["revealed"]
            .as_bool()
            .or_decode("Tezos account: missing revealed")?,
    })
}

/// A node's refusal of an operation it would not simulate, from the error
/// list in its HTTP 500 body; the body as it is when it lists none.
fn refusal_from(body: &str) -> ApiError {
    let ids: Vec<String> = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|errors| errors.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|error| error["id"].as_str().map(str::to_string))
        .collect();
    let reason = if ids.is_empty() {
        body.trim().to_string()
    } else {
        ids.join(", ")
    };
    ApiError::rejected(format!(
        "The Tezos network would refuse this operation: {reason}"
    ))
}

/// The error ids an operation result names.
fn error_ids(result: &Value) -> impl Iterator<Item = String> + '_ {
    result["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|error| error["id"].as_str().map(str::to_string))
}

/// What a simulation answer says each of `contents` consumed, in order:
/// refused unless it answers for exactly those operations and the network
/// applied every one, internal operations included.
fn consumption_from(
    answer: &Value,
    contents: &[TezosContent],
) -> Result<Vec<TezosConsumption>, ApiError> {
    let results = answer["contents"]
        .as_array()
        .or_decode("Tezos simulation: missing contents")?;
    if results.len() != contents.len()
        || results
            .iter()
            .zip(contents)
            .any(|(result, content)| result["kind"].as_str() != Some(content.kind()))
    {
        return Err(ApiError::decode(
            "Tezos simulation: answered for other operations",
        ));
    }
    let mut refusals: Vec<String> = Vec::new();
    let mut consumed = Vec::with_capacity(results.len());
    for result in results {
        let metadata = &result["metadata"];
        let mut total = TezosConsumption::default();
        let internal = metadata["internal_operation_results"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|internal| &internal["result"]);
        for outcome in std::iter::once(&metadata["operation_result"]).chain(internal) {
            let status = outcome["status"]
                .as_str()
                .or_decode("Tezos simulation: missing status")?;
            if status != "applied" {
                refusals.extend(error_ids(outcome));
                if refusals.is_empty() {
                    refusals.push(status.to_string());
                }
                continue;
            }
            let number = |field: &str| -> Result<u64, ApiError> {
                match &outcome[field] {
                    Value::Null => Ok(0),
                    value => value
                        .as_str()
                        .and_then(|digits| digits.parse().ok())
                        .or_decode("Tezos simulation: bad number"),
                }
            };
            let created = u64::from(outcome["allocated_destination_contract"] == true)
                + outcome["originated_contracts"]
                    .as_array()
                    .map_or(0, |contracts| contracts.len() as u64);
            total = TezosConsumption {
                milligas: total
                    .milligas
                    .checked_add(number("consumed_milligas")?)
                    .or_decode("Tezos simulation: gas overflows")?,
                paid_storage: total
                    .paid_storage
                    .checked_add(number("paid_storage_size_diff")?)
                    .or_decode("Tezos simulation: storage overflows")?,
                allocations: total.allocations + created,
            };
        }
        consumed.push(total);
    }
    if !refusals.is_empty() {
        refusals.dedup();
        return Err(ApiError::rejected(format!(
            "The Tezos network would refuse this operation: {}",
            refusals.join(", ")
        )));
    }
    Ok(consumed)
}

#[cfg(test)]
#[path = "tests/tezos_rpc.rs"]
mod tests;
