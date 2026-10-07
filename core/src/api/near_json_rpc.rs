//! The NEAR JSON-RPC adapter: balances, access-key nonces, block hashes,
//! NEP-141 views and broadcast. A node keeps no account history; that is
//! `nearblocks`.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearBalance {
    /// yoctoNEAR (1 NEAR = 10^24 yoctoNEAR).
    pub yocto_near: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearSendResult {
    pub txid: String,
    /// Base64-encoded signed transaction — stored for rebroadcast.
    pub signed_tx_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearFtMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

// ── Client

pub struct NearClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

/// One access key on a NEAR account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearAccessKeyEntry {
    /// `ed25519:…`.
    pub public_key: String,
    /// `None` for a full-access key.
    pub function_call: Option<NearFunctionCallPermission>,
}

/// What a function-call key may do: call `receiver_id`, only `method_names`
/// where any are named, paying gas from `allowance` (`None`: unlimited).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearFunctionCallPermission {
    pub receiver_id: String,
    pub method_names: Vec<String>,
    pub allowance: Option<u128>,
}

/// What a NEAR account's balance must cover, in yoctoNEAR and bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearStorageState {
    pub amount: u128,
    /// Stake locked in the account, which counts toward its storage.
    pub locked: u128,
    pub storage_usage: u64,
    pub cost_per_byte: u128,
}

impl NearStorageState {
    /// What the liquid balance must keep for storage: none for an account
    /// within the free allowance, otherwise its bytes' cost beyond the
    /// locked stake.
    pub(crate) fn storage_reserve(&self) -> Result<u128, ApiError> {
        let reserve = u128::from(self.storage_usage)
            .checked_mul(self.cost_per_byte)
            .or_decode("NEAR storage reserve overflow")?;
        Ok(
            if self.storage_usage
                <= crate::registry::Chain::Near
                    .near_zero_balance_storage_limit()
                    .unwrap_or(0)
            {
                0
            } else {
                reserve.saturating_sub(self.locked)
            },
        )
    }
}

impl NearClient {
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let result = self.call("status", json!({})).await?;
        if result["chain_id"].as_str() != Some(chain.near_network_name()?) {
            return Err(ApiError::decode("NEAR endpoint is on the wrong network"));
        }
        Ok(())
    }

    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
        sender: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_base58_hash(hash, 32)?;
        let response = self
            .call(
                "tx",
                json!({"tx_hash":hash,"sender_account_id":sender,"wait_until":"FINAL"}),
            )
            .await?;
        near_transaction_status(&response, hash, sender)
    }

    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::NearJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }
}

fn near_transaction_status(
    response: &Value,
    hash: &str,
    sender: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    if response
        .pointer("/transaction/hash")
        .and_then(Value::as_str)
        != Some(hash)
        || response
            .pointer("/transaction/signer_id")
            .and_then(Value::as_str)
            != Some(sender)
    {
        return Err(ApiError::decode(
            "NEAR status: transaction identity mismatch",
        ));
    }
    match response["final_execution_status"].as_str() {
        Some("NONE" | "INCLUDED" | "EXECUTED_OPTIMISTIC" | "INCLUDED_FINAL" | "EXECUTED") => {
            return Ok(TransactionStatus::Pending);
        }
        Some("FINAL") => {}
        _ => return Err(ApiError::decode("NEAR status: missing finality result")),
    }
    let status = response["status"]
        .as_object()
        .or_decode("NEAR status: missing execution result")?;
    let succeeded = if status.contains_key("SuccessValue") {
        true
    } else if status.contains_key("Failure") {
        false
    } else {
        return Err(ApiError::decode("NEAR status: incomplete execution result"));
    };
    Ok(TransactionStatus::Confirmed {
        succeeded,
        block: None,
    })
}

// NEAR fetch paths: view_account balance, access-key nonce, latest block hash,
// history (indexer), NEP-141 FT balance + metadata, and the UniFFI-exported

impl NearClient {
    pub async fn fetch_balance(&self, account_id: &str) -> Result<NearBalance, ApiError> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_account",
                    "finality": "final",
                    "account_id": account_id
                }),
            )
            .await?;
        let yocto = result
            .get("amount")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        Ok(NearBalance { yocto_near: yocto })
    }

    pub(crate) async fn fetch_full_access_key_nonce(
        &self,
        account_id: &str,
        public_key_b58: &str,
    ) -> Result<u64, ApiError> {
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "view_access_key",
                    "finality": "final",
                    "account_id": account_id,
                    "public_key": format!("ed25519:{public_key_b58}")
                }),
            )
            .await?;
        if result["permission"] != "FullAccess" {
            return Err(ApiError::invalid(
                "NEAR requires the wallet's full-access key",
            ));
        }
        result
            .get("nonce")
            .and_then(|v| v.as_u64())
            .or_decode("view_access_key: missing nonce")
    }

    pub async fn fetch_latest_block_hash(&self) -> Result<String, ApiError> {
        let result = self.call("block", json!({"finality": "final"})).await?;
        result
            .pointer("/header/hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_decode("block: missing hash")
    }

    /// NEAR expires a transaction by canonical block distance, not elapsed
    /// wall-clock time. Pin all reads to one verified node so a provider race
    /// cannot combine headers from different forks or networks.
    pub(crate) async fn transaction_block_is_valid(
        &self,
        chain: crate::registry::Chain,
        reference: &[u8; 32],
    ) -> Result<bool, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let client = Self::new(std::sync::Arc::new(vec![endpoint]));
            client.verify_network(chain).await?;
            let reference_hash = bs58::encode(reference).into_string();
            let base = client
                .call("block", json!({"block_id": reference_hash}))
                .await?;
            let height = base["header"]["height"]
                .as_u64()
                .or_decode("NEAR reference block: missing height")?;
            if base["header"]["hash"].as_str() != Some(reference_hash.as_str()) {
                return Err(ApiError::decode("NEAR reference block hash mismatch"));
            }
            let canonical = client.call("block", json!({"block_id": height})).await?;
            if canonical["header"]["height"].as_u64() != Some(height) {
                return Err(ApiError::decode("NEAR canonical block height mismatch"));
            }
            if canonical["header"]["hash"].as_str() != Some(reference_hash.as_str()) {
                return Ok(false);
            }
            let head = client
                .call("block", json!({"finality": "optimistic"}))
                .await?;
            let head_height = head["header"]["height"]
                .as_u64()
                .or_decode("NEAR head block: missing height")?;
            let head_hash = head["header"]["hash"]
                .as_str()
                .filter(|hash| bs58::decode(hash).into_vec().is_ok_and(|v| v.len() == 32))
                .or_decode("NEAR head block: invalid hash")?;
            let config = client
                .call(
                    "EXPERIMENTAL_protocol_config",
                    json!({"block_id":head_hash}),
                )
                .await?;
            let period = config["transaction_validity_period"]
                .as_u64()
                .filter(|period| *period > 0)
                .or_decode("NEAR protocol: invalid transaction validity period")?;
            // nearcore store/utils.rs permits the exact boundary and rejects
            // a base that is newer than the head or on a different canonical fork.
            Ok(head_height
                .checked_sub(height)
                .is_some_and(|age| age <= period))
        })
        .await
    }

    // ── NEP-141 (fungible token) support

    /// Call a view function on `contract` and return its decoded bytes.
    /// `args` is JSON that will be serialized, base64-encoded, and sent as
    /// `args_base64` per the NEAR `call_function` query type.
    pub(crate) async fn view_function(
        &self,
        contract: &str,
        method: &str,
        args: &Value,
    ) -> Result<Vec<u8>, ApiError> {
        use base64::Engine;
        let args_str = serde_json::to_string(args)
            .map_err(|e| ApiError::InvalidInput(format!("args serialize: {e}")))?;
        let args_b64 = base64::engine::general_purpose::STANDARD.encode(args_str.as_bytes());
        let result = self
            .call(
                "query",
                json!({
                    "request_type": "call_function",
                    "finality": "final",
                    "account_id": contract,
                    "method_name": method,
                    "args_base64": args_b64,
                }),
            )
            .await?;
        // `result.result` is a u8 array.
        let bytes = result
            .get("result")
            .and_then(|v| v.as_array())
            .or_decode("view_function: missing result bytes")?
            .iter()
            .map(|n| {
                n.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .or_decode("view_function: invalid result byte")
            })
            .collect::<Result<Vec<u8>, ApiError>>()?;
        Ok(bytes)
    }

    pub async fn fetch_ft_balance_of(
        &self,
        contract: &str,
        account_id: &str,
    ) -> Result<u128, ApiError> {
        let bytes = self
            .view_function(
                contract,
                "ft_balance_of",
                &json!({ "account_id": account_id }),
            )
            .await?;
        // Response body is a JSON string like `"1000000"`.
        let s: String = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::Decode(format!("ft_balance_of decode: {e}")))?;
        s.parse::<u128>()
            .map_err(|e| ApiError::Decode(format!("ft_balance_of parse: {e}")))
    }

    pub async fn fetch_ft_metadata(&self, contract: &str) -> Result<NearFtMetadata, ApiError> {
        let bytes = self
            .view_function(contract, "ft_metadata", &json!({}))
            .await?;
        #[derive(Deserialize)]
        struct RawMeta {
            spec: String,
            name: String,
            symbol: String,
            decimals: u8,
        }
        let meta: RawMeta = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::Decode(format!("ft_metadata decode: {e}")))?;
        Ok(NearFtMetadata {
            spec: meta.spec,
            name: meta.name,
            symbol: meta.symbol,
            decimals: meta.decimals,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NearPoolAccount {
    pub account_id: String,
    pub unstaked_balance: String,
    pub staked_balance: String,
    pub can_withdraw: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct NearStakingPool {
    pub approved: bool,
    pub owner_id: String,
    pub commission: f64,
    pub paused: bool,
    pub account: NearPoolAccount,
}

impl NearClient {
    pub(crate) async fn staking_pool_approved(
        &self,
        chain: crate::registry::Chain,
        pool: &str,
    ) -> Result<bool, ApiError> {
        Ok(serde_json::from_slice(
            &self
                .view_function(
                    chain.near_staking_whitelist()?,
                    "is_whitelisted",
                    &json!({"staking_pool_account_id":pool}),
                )
                .await?,
        )?)
    }

    pub(crate) async fn fetch_staking_pool(
        &self,
        chain: crate::registry::Chain,
        pool: &str,
        owner: &str,
    ) -> Result<NearStakingPool, ApiError> {
        let allowed = self.staking_pool_approved(chain, pool).await?;
        let owner_id: String =
            serde_json::from_slice(&self.view_function(pool, "get_owner_id", &json!({})).await?)?;
        let account: NearPoolAccount = serde_json::from_slice(
            &self
                .view_function(pool, "get_account", &json!({"account_id":owner}))
                .await?,
        )?;
        let fee: Value = serde_json::from_slice(
            &self
                .view_function(pool, "get_reward_fee_fraction", &json!({}))
                .await?,
        )?;
        let numerator = fee["numerator"]
            .as_u64()
            .or_decode("NEAR pool: missing commission numerator")?;
        let denominator = fee["denominator"]
            .as_u64()
            .filter(|v| *v > 0)
            .or_decode("NEAR pool: missing commission denominator")?;
        if account.account_id != owner
            || numerator > denominator
            || owner_id.is_empty()
            || account.unstaked_balance.parse::<u128>().is_err()
            || account.staked_balance.parse::<u128>().is_err()
        {
            return Err(ApiError::decode(
                "NEAR pool: invalid owner, commission or balance",
            ));
        }
        let paused = serde_json::from_slice(
            &self
                .view_function(pool, "is_staking_paused", &json!({}))
                .await?,
        )?;
        Ok(NearStakingPool {
            approved: allowed,
            owner_id,
            commission: numerator as f64 / denominator as f64,
            paused,
            account,
        })
    }

    /// An account's balance, the stake locked in it, the bytes it stores and
    /// what the network charges a byte, in yoctoNEAR.
    pub(crate) async fn fetch_storage_state(
        &self,
        owner: &str,
    ) -> Result<NearStorageState, ApiError> {
        let account = self
            .call(
                "query",
                json!({"request_type":"view_account","finality":"final","account_id":owner}),
            )
            .await?;
        let amount = account["amount"]
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .or_decode("NEAR account: invalid native balance")?;
        let storage_usage = account["storage_usage"]
            .as_u64()
            .or_decode("NEAR account: missing storage usage")?;
        let locked = account["locked"]
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .or_decode("NEAR account: invalid locked balance")?;
        let config = self
            .call("EXPERIMENTAL_protocol_config", json!({"finality":"final"}))
            .await?;
        let cost_per_byte = config["runtime_config"]["storage_amount_per_byte"]
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .or_decode("NEAR account: missing storage cost")?;
        Ok(NearStorageState {
            amount,
            locked,
            storage_usage,
            cost_per_byte,
        })
    }

    pub(crate) async fn fetch_spendable_balance(&self, owner: &str) -> Result<u128, ApiError> {
        let state = self.fetch_storage_state(owner).await?;
        Ok(state.amount.saturating_sub(state.storage_reserve()?))
    }

    /// The account's access keys, from one verified node.
    pub(crate) async fn fetch_access_keys(
        &self,
        chain: crate::registry::Chain,
        account_id: &str,
    ) -> Result<Vec<NearAccessKeyEntry>, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let result = node
                .call(
                    "query",
                    json!({"request_type": "view_access_key_list", "finality": "final", "account_id": account_id}),
                )
                .await?;
            result["keys"]
                .as_array()
                .or_decode("view_access_key_list: missing keys")?
                .iter()
                .map(parse_access_key)
                .collect()
        })
        .await
    }

    /// The most deleting one of the signer's own keys costs, in yoctoNEAR.
    pub(crate) async fn delete_key_fee_budget(&self) -> Result<u128, ApiError> {
        let (config, price) = self.fetch_fee_inputs().await?;
        let fees = &config["runtime_config"]["transaction_costs"];
        let parts = [
            &fees["action_receipt_creation_config"],
            &fees["action_creation_config"]["delete_key_cost"],
        ];
        let sum = |field: &str| {
            parts.iter().try_fold(0u128, |total, part| {
                total
                    .checked_add(near_protocol_integer(&part[field])?)
                    .or_decode("NEAR protocol gas overflow")
            })
        };
        // The account is its own receiver.
        near_prepayment_fee(&config, price, sum("send_sir")?, sum("execution")?)
    }

    pub(crate) async fn transfer_fee_budget(
        &self,
        signer: &str,
        receiver: &str,
    ) -> Result<u128, ApiError> {
        let (config, price) = self.fetch_fee_inputs().await?;
        near_transfer_fee(&config, price, signer == receiver, receiver)
    }

    pub(crate) async fn function_call_fee_budget(
        &self,
        signer: &str,
        receiver: &str,
        method: &str,
        args_len: usize,
        gas: u64,
    ) -> Result<u128, ApiError> {
        let (config, price) = self.fetch_fee_inputs().await?;
        near_function_call_fee(
            &config,
            price,
            signer == receiver,
            method.len(),
            args_len,
            gas,
        )
    }

    async fn fetch_fee_inputs(&self) -> Result<(Value, u128), ApiError> {
        let price = self.call("gas_price", json!([null])).await?;
        let price = price["gas_price"]
            .as_str()
            .and_then(|s| s.parse::<u128>().ok())
            .filter(|price| *price > 0)
            .or_decode("NEAR function call: missing gas price")?;
        let config = self
            .call("EXPERIMENTAL_protocol_config", json!({"finality":"final"}))
            .await?;
        Ok((config, price))
    }
}

/// One entry of `view_access_key_list`.
fn parse_access_key(entry: &Value) -> Result<NearAccessKeyEntry, ApiError> {
    let public_key = entry["public_key"]
        .as_str()
        .filter(|key| key.starts_with("ed25519:") || key.starts_with("secp256k1:"))
        .or_decode("view_access_key_list: missing public key")?
        .to_string();
    let permission = &entry["access_key"]["permission"];
    let function_call = if permission == "FullAccess" {
        None
    } else {
        let call = permission
            .get("FunctionCall")
            .or_decode("view_access_key_list: unknown permission")?;
        Some(NearFunctionCallPermission {
            receiver_id: call["receiver_id"]
                .as_str()
                .or_decode("view_access_key_list: missing receiver")?
                .to_string(),
            method_names: call["method_names"]
                .as_array()
                .or_decode("view_access_key_list: missing method names")?
                .iter()
                .map(|name| name.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .or_decode("view_access_key_list: invalid method name")?,
            allowance: match &call["allowance"] {
                Value::Null => None,
                allowance => Some(
                    allowance
                        .as_str()
                        .and_then(|value| value.parse().ok())
                        .or_decode("view_access_key_list: invalid allowance")?,
                ),
            },
        })
    };
    Ok(NearAccessKeyEntry {
        public_key,
        function_call,
    })
}

fn near_protocol_integer(v: &Value) -> Result<u128, ApiError> {
    v.as_str()
        .and_then(|s| s.parse::<u128>().ok())
        .or_else(|| v.as_u64().map(u128::from))
        .or_decode("NEAR transfer: missing protocol fee")
}

fn near_prepayment_fee(
    config: &Value,
    price: u128,
    burnt: u128,
    remaining: u128,
) -> Result<u128, ApiError> {
    let minimum = near_protocol_integer(&config["runtime_config"]["min_gas_purchase_price"])?;
    burnt
        .checked_mul(price)
        .and_then(|v| {
            remaining
                .checked_mul(price.max(minimum))
                .and_then(|remaining| v.checked_add(remaining))
        })
        .or_decode("NEAR protocol fee overflow")
}

fn near_transfer_fee(
    config: &Value,
    price: u128,
    sender_is_receiver: bool,
    receiver: &str,
) -> Result<u128, ApiError> {
    let fees = &config["runtime_config"]["transaction_costs"];
    let actions = &fees["action_creation_config"];
    let mut parts = vec![
        &fees["action_receipt_creation_config"],
        &actions["transfer_cost"],
    ];
    // nearcore transfer_send_fee/transfer_exec_fee charge the implicit
    // creation costs by account type even when that account already exists.
    let near_implicit = receiver.len() == 64 && receiver.bytes().all(|b| b.is_ascii_hexdigit());
    let eth_implicit = receiver.len() == 42
        && receiver.starts_with("0x")
        && receiver[2..].bytes().all(|b| b.is_ascii_hexdigit());
    if near_implicit || eth_implicit {
        parts.push(&actions["create_account_cost"]);
    }
    if near_implicit {
        parts.push(&actions["add_key_cost"]["full_access_cost"]);
    }
    let send = if sender_is_receiver {
        "send_sir"
    } else {
        "send_not_sir"
    };
    let sum = |field: &str| {
        parts.iter().try_fold(0u128, |total, part| {
            total
                .checked_add(near_protocol_integer(&part[field])?)
                .or_decode("NEAR protocol gas overflow")
        })
    };
    near_prepayment_fee(config, price, sum(send)?, sum("execution")?)
}

/// The admission charge buys function gas and execution overhead at at least
/// min_gas_purchase_price; the send overhead burns at the current gas price.
/// This is nearcore runtime/config.rs calculate_tx_cost for one Ed25519
/// FunctionCall (classical signature verification adds no gas charge).
fn near_function_call_fee(
    config: &Value,
    price: u128,
    sender_is_receiver: bool,
    method_len: usize,
    args_len: usize,
    gas: u64,
) -> Result<u128, ApiError> {
    let runtime = &config["runtime_config"];
    let fees = &runtime["transaction_costs"];
    let receipt = &fees["action_receipt_creation_config"];
    let base = &fees["action_creation_config"]["function_call_cost"];
    let byte = &fees["action_creation_config"]["function_call_cost_per_byte"];
    let count = u128::try_from(
        method_len
            .checked_add(args_len)
            .or_decode("NEAR argument size overflow")?,
    )
    .map_err(ApiError::decode)?;
    let send = if sender_is_receiver {
        "send_sir"
    } else {
        "send_not_sir"
    };
    let total = |field: &str, prepaid: u128| -> Result<u128, ApiError> {
        let base = near_protocol_integer(&base[field])?;
        let receipt = near_protocol_integer(&receipt[field])?;
        near_protocol_integer(&byte[field])?
            .checked_mul(count)
            .and_then(|v| v.checked_add(prepaid))
            .and_then(|v| v.checked_add(base))
            .and_then(|v| v.checked_add(receipt))
            .or_decode("NEAR protocol gas or fee overflow")
    };
    let burnt = total(send, 0)?;
    let remaining = total("execution", u128::from(gas))?;
    near_prepayment_fee(config, price, burnt, remaining)
}

impl NearClient {
    /// Rebroadcast a pre-signed transaction (base64-encoded).
    pub async fn broadcast_signed_tx_b64(&self, tx_b64: &str) -> Result<NearSendResult, ApiError> {
        let result = self.call("broadcast_tx_commit", json!([tx_b64])).await?;
        let txid = result
            .get("transaction")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(NearSendResult {
            txid,
            signed_tx_b64: tx_b64.to_string(),
        })
    }
}

// Validator-directory response data; staking owns the projection.
#[derive(Deserialize)]
pub struct ValidatorsResult {
    pub current_validators: Vec<NearCurrentValidator>,
}
#[derive(Deserialize, Clone)]
pub struct NearCurrentValidator {
    pub account_id: String,
    pub stake: String, // yoctoNEAR string
    pub is_slashed: bool,
    pub num_produced_blocks: u64,
    pub num_expected_blocks: u64,
}

impl NearClient {
    pub async fn fetch_staking_validators(&self) -> Result<ValidatorsResult, ApiError> {
        let value = self.call("validators", serde_json::json!([null])).await?;
        serde_json::from_value(value).map_err(ApiError::from)
    }
}

#[cfg(test)]
mod transaction_validity_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};

    #[test]
    fn transfers_charge_the_receiver_account_type_and_protocol_price_floor() {
        let config: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/near-staking-fee-protocol86.json"
        ))
        .unwrap();
        assert_eq!(
            near_transfer_fee(&config, 100_000_000, false, "receiver.near").unwrap(),
            245_500_818_750_000_000_000
        );
        assert_eq!(
            near_transfer_fee(&config, 100_000_000, false, &"00".repeat(32)).unwrap(),
            7_607_442_456_250_000_000_000
        );
        assert_eq!(
            near_transfer_fee(
                &config,
                100_000_000,
                false,
                &format!("0x{}", "00".repeat(20))
            )
            .unwrap(),
            7_495_500_818_750_000_000_000
        );
        assert!(near_transfer_fee(&config, u128::MAX, false, "receiver.near").is_err());
        let mut missing = config;
        missing["runtime_config"]["transaction_costs"]["action_creation_config"]
            .as_object_mut()
            .unwrap()
            .remove("create_account_cost");
        assert!(near_transfer_fee(&missing, 100_000_000, false, &"00".repeat(32)).is_err());
    }

    #[tokio::test]
    async fn spendable_balance_retains_storage_stake_and_counts_locked_funds() {
        for (usage, locked, expected) in [
            (770, 0, 20_000u128),
            (771, 0, 12_290),
            (771, 5_000, 17_290),
            (1_000, 30_000, 20_000),
        ] {
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |request:&Request| {
                let request:Value=serde_json::from_slice(&request.body).unwrap();
                let result=match request["method"].as_str().unwrap() {
                    "query"=>json!({"amount":"20000","locked":locked.to_string(),"storage_usage":usage}),
                    "EXPERIMENTAL_protocol_config"=>json!({"runtime_config":{"storage_amount_per_byte":"10"}}),
                    other=>panic!("Unexpected method {other}"),
                };
                ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }).mount(&server).await;
            assert_eq!(
                NearClient::new(Arc::new(vec![server.uri()]))
                    .fetch_spendable_balance("owner.near")
                    .await
                    .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn function_call_budget_includes_protocol_fees_and_minimum_purchase_price() {
        let config: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/near-staking-fee-protocol86.json"
        ))
        .unwrap();
        // 19 bytes: deposit_and_stake plus JSON {}. These admission charges
        // use the independently captured protocol-86 fees and nearcore formula.
        let budget =
            near_function_call_fee(&config, 100_000_000, false, 17, 2, 100_000_000_000_000)
                .unwrap();
        assert_eq!(budget, 100_918_998_531_804_500_000_000);
        assert!(budget > 100_000_000_000_000 * 100_000_000);
        assert_eq!(
            near_function_call_fee(&config, 2_000_000_000, false, 17, 2, 100_000_000_000_000)
                .unwrap(),
            202_394_134_946_662_000_000_000
        );
        let mut missing = config.clone();
        missing["runtime_config"]
            .as_object_mut()
            .unwrap()
            .remove("min_gas_purchase_price");
        assert!(
            near_function_call_fee(&missing, 100_000_000, false, 17, 2, 100_000_000_000_000)
                .is_err()
        );
        assert!(
            near_function_call_fee(&config, u128::MAX, false, 17, 2, 100_000_000_000_000).is_err()
        );
    }

    #[tokio::test]
    async fn canonical_block_distance_and_live_period_decide_validity() {
        // nearcore permits the exact boundary; a future base or canonical-fork
        // mismatch is invalid regardless of a locally recent creation time.
        for (base, head, period, canonical, expected) in [
            (100, 101, 10, true, Some(true)),
            (100, 110, 10, true, Some(true)),
            (100, 111, 10, true, Some(false)),
            (111, 110, 10, true, Some(false)),
            (100, 101, 10, false, Some(false)),
            (100, 101, 0, true, None),
        ] {
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(move |request: &Request| {
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    let reference = bs58::encode([3; 32]).into_string();
                    let result = match body["method"].as_str().unwrap() {
                        "status" => json!({"chain_id":"mainnet"}),
                        "block" if body["params"]["finality"] == "optimistic" => {
                            json!({"header":{"hash":bs58::encode([6;32]).into_string(),"height":head}})
                        }
                        "block" => json!({"header":{
                            "hash":if !canonical && body["params"]["block_id"].is_u64() {bs58::encode([5;32]).into_string()} else {reference},
                            "height":base
                        }}),
                        "EXPERIMENTAL_protocol_config" => json!({"transaction_validity_period":period}),
                        other => panic!("Unexpected validity request: {other}"),
                    };
                    ResponseTemplate::new(200).set_body_json(json!({"jsonrpc":"2.0","id":"1","result":result}))
                })
                .mount(&server)
                .await;
            let result = NearClient::new(Arc::new(vec![server.uri()]))
                .transaction_block_is_valid(crate::registry::Chain::Near, &[3; 32])
                .await;
            match expected {
                Some(expected) => assert_eq!(result.unwrap(), expected),
                None => assert!(
                    result.is_err(),
                    "Missing protocol facts must not waive expiry"
                ),
            }
        }
    }
}
