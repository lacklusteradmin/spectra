//! Starknet's JSON-RPC (spec 0.9 and later): the node's chain id, an
//! account's class, nonce and signers, ERC-20 balances and decimals read by
//! `starknet_call`, fee estimates, submission of a signed deployment and
//! transfer, and a transaction's status.
//!
//! A node answers a missing contract or transaction with a JSON-RPC error
//! whose code says so (20, 29), which is an answer rather than a failure. A
//! node keeps no index of an address's transactions, so it serves no
//! history.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use starknet_crypto::Felt;

use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::derivation::starknet::{felt_serde, parse_felt, selector};
use crate::registry::Chain;

/// `CONTRACT_NOT_FOUND`: no contract is deployed at the address.
const CONTRACT_NOT_FOUND: i64 = 20;
/// `TXN_HASH_NOT_FOUND`: the node does not know the transaction.
const TRANSACTION_NOT_FOUND: i64 = 29;
/// `DUPLICATE_TX`: the node already holds this transaction.
const DUPLICATE_TRANSACTION: i64 = 59;

/// The version a v3 transaction is submitted with, and the one a fee
/// estimate simulates it with: `2^128 + 3`, which no node executes.
const VERSION: &str = "0x3";
const QUERY_VERSION: &str = "0x100000000000000000000000000000003";

/// The most of one resource a transaction may use, and the most it pays
/// for each unit of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResourceBound {
    pub max_amount: u64,
    #[serde(with = "crate::send::stages::units_u128")]
    pub max_price_per_unit: u128,
}

impl ResourceBound {
    /// The most this bound lets the network charge.
    fn max_fee(self) -> Option<u128> {
        u128::from(self.max_amount).checked_mul(self.max_price_per_unit)
    }
}

/// A v3 transaction's bounds on L1 gas, L2 gas and L1 data gas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResourceBounds {
    pub l1_gas: ResourceBound,
    pub l2_gas: ResourceBound,
    pub l1_data_gas: ResourceBound,
}

impl ResourceBounds {
    /// The bounds that let nothing be charged: what an estimate carries.
    pub(crate) const ZERO: Self = Self {
        l1_gas: ResourceBound {
            max_amount: 0,
            max_price_per_unit: 0,
        },
        l2_gas: ResourceBound {
            max_amount: 0,
            max_price_per_unit: 0,
        },
        l1_data_gas: ResourceBound {
            max_amount: 0,
            max_price_per_unit: 0,
        },
    };

    /// The most a transaction under these bounds can cost, in fri, with no
    /// tip.
    pub(crate) fn max_fee(&self) -> Option<u128> {
        self.l1_gas
            .max_fee()?
            .checked_add(self.l2_gas.max_fee()?)?
            .checked_add(self.l1_data_gas.max_fee()?)
    }

    fn json(&self) -> Value {
        let bound = |bound: &ResourceBound| {
            json!({
                "max_amount": format!("{:#x}", bound.max_amount),
                "max_price_per_unit": format!("{:#x}", bound.max_price_per_unit),
            })
        };
        json!({
            "l1_gas": bound(&self.l1_gas),
            "l2_gas": bound(&self.l2_gas),
            "l1_data_gas": bound(&self.l1_data_gas),
        })
    }
}

/// What a node's estimate says one transaction uses, and at what prices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FeeEstimate {
    pub l1_gas: (u64, u128),
    pub l2_gas: (u64, u128),
    pub l1_data_gas: (u64, u128),
}

impl FeeEstimate {
    /// Bounds `headroom_percent` above every amount and price the estimate
    /// used.
    pub(crate) fn bounds(&self, headroom_percent: u128) -> Option<ResourceBounds> {
        let bound = |(amount, price): (u64, u128)| -> Option<ResourceBound> {
            let amount = u128::from(amount);
            Some(ResourceBound {
                max_amount: u64::try_from(amount.checked_add(amount * headroom_percent / 100)?)
                    .ok()?,
                max_price_per_unit: price
                    .checked_add(price.checked_mul(headroom_percent)? / 100)?,
            })
        };
        Some(ResourceBounds {
            l1_gas: bound(self.l1_gas)?,
            l2_gas: bound(self.l2_gas)?,
            l1_data_gas: bound(self.l1_data_gas)?,
        })
    }
}

/// A felt as the RPC spec writes one: `0x` and no leading zeros.
pub(crate) fn rpc_felt(felt: &Felt) -> String {
    let digits = hex::encode(felt.to_bytes_be());
    let trimmed = digits.trim_start_matches('0');
    format!("0x{}", if trimmed.is_empty() { "0" } else { trimmed })
}

fn rpc_felts(felts: &[Felt]) -> Vec<String> {
    felts.iter().map(rpc_felt).collect()
}

/// An `INVOKE` v3 from an account: its calls as `__execute__` calldata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StarknetInvoke {
    #[serde(with = "felt_serde")]
    pub sender_address: Felt,
    #[serde(with = "felt_serde::list")]
    pub calldata: Vec<Felt>,
    #[serde(with = "felt_serde::list")]
    pub signature: Vec<Felt>,
    pub nonce: u64,
    pub resource_bounds: ResourceBounds,
}

impl StarknetInvoke {
    /// The transaction as `starknet_addInvokeTransaction` takes it, or as a
    /// fee estimate simulates it (`query`).
    pub(crate) fn request_body(&self, query: bool) -> Value {
        json!({
            "type": "INVOKE",
            "sender_address": rpc_felt(&self.sender_address),
            "calldata": rpc_felts(&self.calldata),
            "signature": rpc_felts(&self.signature),
            "nonce": format!("{:#x}", self.nonce),
            "resource_bounds": self.resource_bounds.json(),
            "tip": "0x0",
            "paymaster_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "account_deployment_data": [],
            "version": if query { QUERY_VERSION } else { VERSION },
        })
    }
}

/// A `DEPLOY_ACCOUNT` v3: the account deploying itself at the address its
/// class, salt and constructor arguments make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StarknetDeployAccount {
    #[serde(with = "felt_serde")]
    pub class_hash: Felt,
    #[serde(with = "felt_serde")]
    pub contract_address_salt: Felt,
    #[serde(with = "felt_serde::list")]
    pub constructor_calldata: Vec<Felt>,
    #[serde(with = "felt_serde::list")]
    pub signature: Vec<Felt>,
    pub resource_bounds: ResourceBounds,
}

impl StarknetDeployAccount {
    /// The transaction as `starknet_addDeployAccountTransaction` takes it,
    /// or as a fee estimate simulates it (`query`). Its nonce is zero.
    pub(crate) fn request_body(&self, query: bool) -> Value {
        json!({
            "type": "DEPLOY_ACCOUNT",
            "constructor_calldata": rpc_felts(&self.constructor_calldata),
            "class_hash": rpc_felt(&self.class_hash),
            "contract_address_salt": rpc_felt(&self.contract_address_salt),
            "signature": rpc_felts(&self.signature),
            "nonce": "0x0",
            "resource_bounds": self.resource_bounds.json(),
            "tip": "0x0",
            "paymaster_data": [],
            "nonce_data_availability_mode": "L1",
            "fee_data_availability_mode": "L1",
            "version": if query { QUERY_VERSION } else { VERSION },
        })
    }
}

/// A signed transfer as the network takes it: the account's deployment
/// first when it has none yet, then the transfer, each under its hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StarknetSignedTransfer {
    pub deploy_account: Option<StarknetSignedDeployment>,
    pub invoke: StarknetInvoke,
    #[serde(with = "felt_serde")]
    pub invoke_hash: Felt,
}

/// A signed deployment and the hash it lands under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StarknetSignedDeployment {
    pub transaction: StarknetDeployAccount,
    #[serde(with = "felt_serde")]
    pub hash: Felt,
}

impl StarknetSignedTransfer {
    /// The transactions as their submissions carry them, in order.
    pub(crate) fn request_bodies(&self) -> Value {
        let mut bodies = Vec::new();
        if let Some(deployment) = &self.deploy_account {
            bodies.push(json!({
                "deploy_account_transaction": deployment.transaction.request_body(false),
            }));
        }
        bodies.push(json!({ "invoke_transaction": self.invoke.request_body(false) }));
        Value::Array(bodies)
    }
}

/// A JSON-RPC error a node answered with.
#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

pub struct StarknetClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl StarknetClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// `method` asked of every endpoint at once; the first answer wins, an
    /// error object among them. `profile` is a read's or a submission's.
    async fn request(
        &self,
        method: &'static str,
        params: Value,
        profile: RetryProfile,
    ) -> Result<Result<Value, RpcError>, ApiError> {
        let body = Arc::new(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}));
        race(&self.endpoints, |url| {
            let client = Arc::clone(&self.client);
            let body = Arc::clone(&body);
            async move {
                let response: Value = match client.post_json(&url, &*body, profile).await {
                    Ok(response) => response,
                    // A node may answer an error object with a failing
                    // status; the object is still its answer.
                    Err(ApiError::Status { body, .. })
                        if serde_json::from_str::<Value>(&body)
                            .is_ok_and(|value| value["error"].is_object()) =>
                    {
                        serde_json::from_str(&body)?
                    }
                    Err(error) => return Err(error),
                };
                decode(method, response)
            }
        })
        .await
    }

    /// `method`'s result; an error object is the node's refusal.
    async fn result(&self, method: &'static str, params: Value) -> Result<Value, ApiError> {
        self.request(method, params, RetryProfile::ChainRead)
            .await?
            .map_err(|error| refusal(method, &error))
    }

    /// Refuse a node that is not on `chain`: its `starknet_chainId` must be
    /// the chain id every transaction hash binds.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        let expected = chain.starknet_network()?.chain_id;
        let answer = self.result("starknet_chainId", json!([])).await?;
        let network = answer
            .as_str()
            .and_then(parse_felt)
            .or_decode("Starknet chain id: not a felt")?;
        if network != Felt::from_bytes_be_slice(expected.as_bytes()) {
            return Err(ApiError::invalid(format!(
                "Endpoint is on chain {}, not {expected}",
                rpc_felt(&network)
            )));
        }
        Ok(())
    }

    /// The class of the contract at `address`, or `None` while nothing is
    /// deployed there: an account that has never sent.
    pub(crate) async fn fetch_class_hash(&self, address: &Felt) -> Result<Option<Felt>, ApiError> {
        match self
            .request(
                "starknet_getClassHashAt",
                json!(["latest", rpc_felt(address)]),
                RetryProfile::ChainRead,
            )
            .await?
        {
            Ok(class) => class
                .as_str()
                .and_then(parse_felt)
                .map(Some)
                .or_decode("Starknet class hash: not a felt"),
            Err(error) if error.code == CONTRACT_NOT_FOUND => Ok(None),
            Err(error) => Err(refusal("starknet_getClassHashAt", &error)),
        }
    }

    /// The account's next nonce. Zero where no account is deployed yet: its
    /// deployment is the transaction that spends it.
    pub(crate) async fn fetch_nonce(&self, address: &Felt) -> Result<u64, ApiError> {
        match self
            .request(
                "starknet_getNonce",
                json!(["latest", rpc_felt(address)]),
                RetryProfile::ChainRead,
            )
            .await?
        {
            Ok(nonce) => nonce
                .as_str()
                .and_then(parse_felt)
                .and_then(|nonce| u64::try_from(nonce.to_biguint()).ok())
                .or_decode("Starknet nonce: not a number"),
            Err(error) if error.code == CONTRACT_NOT_FOUND => Ok(0),
            Err(error) => Err(refusal("starknet_getNonce", &error)),
        }
    }

    /// What a view function of `contract` returns for `calldata`.
    async fn call(
        &self,
        contract: &Felt,
        entry_point: &str,
        calldata: &[Felt],
    ) -> Result<Vec<Felt>, ApiError> {
        let answer = self
            .result(
                "starknet_call",
                json!([{
                    "contract_address": rpc_felt(contract),
                    "entry_point_selector": rpc_felt(&selector(entry_point)),
                    "calldata": rpc_felts(calldata),
                }, "latest"]),
            )
            .await?;
        felts(&answer).or_decode("Starknet call: not a list of felts")
    }

    /// `holder`'s balance of the ERC-20 `token`, in its base units: the
    /// `u256` `balanceOf` answers, refused above `u128`.
    pub(crate) async fn fetch_token_balance(
        &self,
        token: &Felt,
        holder: &Felt,
    ) -> Result<u128, ApiError> {
        u256_value(&self.call(token, "balanceOf", &[*holder]).await?)
    }

    /// `address`'s balance of `chain`'s own coin, STRK, whose contract the
    /// registry names.
    pub(crate) async fn fetch_balance(
        &self,
        chain: Chain,
        address: &Felt,
    ) -> Result<u128, ApiError> {
        let token = parse_felt(chain.starknet_network()?.fee_token)
            .or_decode("Starknet fee token: not a felt")?;
        self.fetch_token_balance(&token, address).await
    }

    /// The places the ERC-20 `token` states its amounts in.
    pub(crate) async fn fetch_decimals(&self, token: &Felt) -> Result<u8, ApiError> {
        let answer = self.call(token, "decimals", &[]).await?;
        let [decimals] = answer.as_slice() else {
            return Err(ApiError::decode("Starknet decimals: not one felt"));
        };
        u8::try_from(decimals.to_biguint())
            .map_err(|_| ApiError::decode("Starknet decimals: too large"))
    }

    /// The owner key and the guardian (zero where there is none) of a
    /// deployed Argent account.
    pub(crate) async fn fetch_signers(&self, account: &Felt) -> Result<(Felt, Felt), ApiError> {
        let (owner, guardian) = tokio::try_join!(
            self.call(account, "get_owner", &[]),
            self.call(account, "get_guardian", &[]),
        )?;
        match (owner.as_slice(), guardian.as_slice()) {
            ([owner], [guardian]) => Ok((*owner, *guardian)),
            _ => Err(ApiError::decode(
                "Starknet account signers: not one felt each",
            )),
        }
    }

    /// What each transaction uses, simulated in order on the latest state
    /// with its signature checked as an estimate checks it.
    pub(crate) async fn estimate_fees(
        &self,
        transactions: &[Value],
    ) -> Result<Vec<FeeEstimate>, ApiError> {
        let answer = self
            .result(
                "starknet_estimateFee",
                json!({
                    "request": transactions,
                    "simulation_flags": [],
                    "block_id": "latest",
                }),
            )
            .await?;
        let estimates = answer
            .as_array()
            .or_decode("Starknet fee estimate: not a list")?
            .iter()
            .map(fee_estimate)
            .collect::<Result<Vec<_>, _>>()?;
        if estimates.len() != transactions.len() {
            return Err(ApiError::decode(
                "Starknet fee estimate: one estimate per transaction",
            ));
        }
        Ok(estimates)
    }

    /// Submit a signed transfer, its account's deployment first. A
    /// deployment the node already holds, or one the chain has already
    /// run, is not sent again; nor is a transfer the node already holds.
    /// Answers the transfer's hash.
    pub(crate) async fn broadcast(
        &self,
        signed: &StarknetSignedTransfer,
    ) -> Result<String, ApiError> {
        if let Some(deployment) = &signed.deploy_account
            && self
                .fetch_class_hash(&signed.invoke.sender_address)
                .await?
                .is_none()
            && !self.is_known(&deployment.hash).await?
        {
            self.submit(
                "starknet_addDeployAccountTransaction",
                json!({ "deploy_account_transaction": deployment.transaction.request_body(false) }),
                &deployment.hash,
            )
            .await?;
        }
        self.submit(
            "starknet_addInvokeTransaction",
            json!({ "invoke_transaction": signed.invoke.request_body(false) }),
            &signed.invoke_hash,
        )
        .await?;
        Ok(crate::derivation::starknet::felt_hex(&signed.invoke_hash))
    }

    /// Whether the node knows the transaction `hash` at all, received or
    /// further.
    async fn is_known(&self, hash: &Felt) -> Result<bool, ApiError> {
        match self
            .request(
                "starknet_getTransactionStatus",
                json!([rpc_felt(hash)]),
                RetryProfile::ChainRead,
            )
            .await?
        {
            Ok(_) => Ok(true),
            Err(error) if error.code == TRANSACTION_NOT_FOUND => Ok(false),
            Err(error) => Err(refusal("starknet_getTransactionStatus", &error)),
        }
    }

    /// Submit one transaction and require the node to take it under
    /// `expected`; a node that already holds it has taken it.
    async fn submit(
        &self,
        method: &'static str,
        params: Value,
        expected: &Felt,
    ) -> Result<(), ApiError> {
        let answer = match self
            .request(method, params, RetryProfile::ChainWrite)
            .await?
        {
            Ok(answer) => answer,
            Err(error) if error.code == DUPLICATE_TRANSACTION => return Ok(()),
            Err(error) => return Err(refusal(method, &error)),
        };
        let hash = answer["transaction_hash"]
            .as_str()
            .and_then(parse_felt)
            .or_decode("Starknet submission: missing transaction hash")?;
        if hash != *expected {
            return Err(ApiError::decode(format!(
                "Starknet node took the transaction under {}, not {}",
                rpc_felt(&hash),
                rpc_felt(expected)
            )));
        }
        Ok(())
    }

    /// Whether the transaction `hash` is in an accepted block, and whether
    /// it ran. One the node does not know, or holds unconfirmed, is still
    /// pending; one an older node reports rejected never ran.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &Felt,
    ) -> Result<TransactionStatus, ApiError> {
        let status = match self
            .request(
                "starknet_getTransactionStatus",
                json!([rpc_felt(hash)]),
                RetryProfile::ChainRead,
            )
            .await?
        {
            Ok(status) => status,
            Err(error) if error.code == TRANSACTION_NOT_FOUND => {
                return Ok(TransactionStatus::Pending);
            }
            Err(error) => return Err(refusal("starknet_getTransactionStatus", &error)),
        };
        transaction_status(&status)
    }
}

/// A node's answer as its result or its error object.
fn decode(method: &str, response: Value) -> Result<Result<Value, RpcError>, ApiError> {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Ok(Err(RpcError {
            code: error["code"].as_i64().unwrap_or_default(),
            message: match &error["data"] {
                Value::Null => error["message"].as_str().unwrap_or_default().to_string(),
                data => format!(
                    "{}: {}",
                    error["message"].as_str().unwrap_or_default(),
                    data.as_str()
                        .map_or_else(|| data.to_string(), str::to_string)
                ),
            },
        }));
    }
    response
        .get("result")
        .cloned()
        .map(Ok)
        .ok_or_else(|| ApiError::decode(format!("Starknet {method}: missing result")))
}

fn refusal(method: &str, error: &RpcError) -> ApiError {
    ApiError::rejected(format!(
        "Starknet node refused {method} (code {}): {}",
        error.code, error.message
    ))
}

/// A list of felts as a node writes them.
fn felts(value: &Value) -> Option<Vec<Felt>> {
    value
        .as_array()?
        .iter()
        .map(|felt| felt.as_str().and_then(parse_felt))
        .collect()
}

/// A Cairo `u256` (`low`, `high`) as a `u128`, refused when it is larger.
fn u256_value(words: &[Felt]) -> Result<u128, ApiError> {
    let [low, high] = words else {
        return Err(ApiError::decode("Starknet u256: not two felts"));
    };
    if *high != Felt::ZERO {
        return Err(ApiError::decode("Starknet u256: exceeds u128"));
    }
    u128::try_from(low.to_biguint())
        .map_err(|_| ApiError::decode("Starknet u256: low word exceeds u128"))
}

/// A hex quantity of an estimate.
fn quantity(value: &Value, what: &str) -> Result<u128, ApiError> {
    value
        .as_str()
        .and_then(parse_felt)
        .and_then(|felt| u128::try_from(felt.to_biguint()).ok())
        .or_decode(what)
}

/// One transaction's estimate, priced in fri: the STRK a v3 transaction
/// pays in.
fn fee_estimate(estimate: &Value) -> Result<FeeEstimate, ApiError> {
    if estimate["unit"].as_str() != Some("FRI") {
        return Err(ApiError::decode("Starknet fee estimate: not priced in fri"));
    }
    let pair = |amount: &str, price: &str| -> Result<(u64, u128), ApiError> {
        Ok((
            u64::try_from(quantity(
                &estimate[amount],
                "Starknet fee estimate: bad amount",
            )?)
            .map_err(|_| ApiError::decode("Starknet fee estimate: amount exceeds u64"))?,
            quantity(&estimate[price], "Starknet fee estimate: bad price")?,
        ))
    };
    Ok(FeeEstimate {
        l1_gas: pair("l1_gas_consumed", "l1_gas_price")?,
        l2_gas: pair("l2_gas_consumed", "l2_gas_price")?,
        l1_data_gas: pair("l1_data_gas_consumed", "l1_data_gas_price")?,
    })
}

/// A `starknet_getTransactionStatus` answer: confirmed once a block on L2
/// or L1 accepted it, succeeded unless it reverted.
fn transaction_status(status: &Value) -> Result<TransactionStatus, ApiError> {
    let finality = status["finality_status"]
        .as_str()
        .or_decode("Starknet status: missing finality")?;
    Ok(match finality {
        "ACCEPTED_ON_L2" | "ACCEPTED_ON_L1" => TransactionStatus::Confirmed {
            succeeded: match status["execution_status"].as_str() {
                Some("SUCCEEDED") => true,
                Some("REVERTED") => false,
                _ => {
                    return Err(ApiError::decode(
                        "Starknet status: missing execution status",
                    ));
                }
            },
            block: None,
        },
        "REJECTED" => TransactionStatus::Confirmed {
            succeeded: false,
            block: None,
        },
        "RECEIVED" | "CANDIDATE" | "PRE_CONFIRMED" => TransactionStatus::Pending,
        other => {
            return Err(ApiError::decode(format!(
                "Starknet status: unknown finality {other}"
            )));
        }
    })
}

#[cfg(test)]
#[path = "tests/starknet_json_rpc.rs"]
mod tests;
