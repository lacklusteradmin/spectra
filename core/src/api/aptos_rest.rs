//! The Aptos REST adapter: account resources, token balances and decimals
//! through view functions, gas price, history, simulation and submission of
//! a signed body.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosBalance {
    /// Octas (1 APT = 100_000_000 octas).
    pub octas: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AptosSendResult {
    pub txid: String,
    pub version: Option<u64>,
    /// JSON-encoded signed transaction body — stored for rebroadcast.
    pub signed_body_json: String,
}

/// What an Aptos token identifier names.
///
/// A fungible asset (AIP-21), which every catalog token is, is named by the
/// address of its metadata object; a legacy coin by its Move type,
/// `0xADDR::module::Name`. Those are the two shapes `aptosTokenType` accepts,
/// and an address never contains `::`. The catalog's `standard` cannot tell
/// them apart: the network has one, and a custom token takes it either way.
#[derive(Debug, PartialEq)]
enum AptosToken<'a> {
    Asset { metadata: &'a str },
    Coin { coin_type: &'a str },
}

impl<'a> AptosToken<'a> {
    fn parse(identifier: &'a str) -> Self {
        let identifier = identifier.trim();
        if identifier.contains("::") {
            AptosToken::Coin {
                coin_type: identifier,
            }
        } else {
            AptosToken::Asset {
                metadata: identifier,
            }
        }
    }
}

/// The type argument naming a fungible asset by its metadata object.
const FUNGIBLE_ASSET_METADATA: &str = "0x1::fungible_asset::Metadata";

/// A view function's first return value as a number. The node writes a
/// `u64` as a decimal string and a `u8` as a JSON number.
fn returned_number(values: &[Value]) -> Option<u64> {
    match values.first()? {
        Value::String(s) => s.parse().ok(),
        other => other.as_u64(),
    }
}

// ── Client

pub struct AptosClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

impl AptosClient {
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_hex_hash(hash)?;
        let response: Value = match self.get(&format!("/transactions/by_hash/{hash}")).await {
            Ok(response) => response,
            Err(ApiError::Status { status: 404, .. }) => {
                return Ok(crate::api::transaction_status::TransactionStatus::Pending);
            }
            Err(error) => return Err(error),
        };
        aptos_transaction_status(&response, hash)
    }

    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    pub(crate) async fn post_val(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        let path = path.to_string();
        let body = std::sync::Arc::new(body.clone());
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body = std::sync::Arc::clone(&body);
            async move {
                client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await
            }
        })
        .await
    }
}

fn aptos_transaction_status(
    response: &Value,
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    if !response["hash"]
        .as_str()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
    {
        return Err(ApiError::decode("Aptos status: transaction hash mismatch"));
    }
    match response["type"].as_str() {
        Some("pending_transaction") => Ok(TransactionStatus::Pending),
        Some("user_transaction") => {
            response["version"]
                .as_str()
                .and_then(|value| value.parse::<u64>().ok())
                .or_decode("Aptos status: missing committed ledger version")?;
            Ok(TransactionStatus::Confirmed {
                succeeded: response["success"]
                    .as_bool()
                    .or_decode("Aptos status: missing execution result")?,
                block: None,
            })
        }
        _ => Err(ApiError::decode("Aptos status: invalid transaction type")),
    }
}
// Aptos fetch paths: balance, token balance and decimals, account info,
// ledger info, gas price, history.

impl AptosClient {
    /// APT is a coin like any other to `0x1::coin::balance`. An account that
    /// holds it only as the migrated fungible asset, as accounts created
    /// since the migration do, has no `CoinStore<AptosCoin>` to read.
    pub async fn fetch_balance(&self, address: &str) -> Result<AptosBalance, ApiError> {
        let octas = self
            .fetch_token_balance(address, "0x1::aptos_coin::AptosCoin")
            .await?;
        Ok(AptosBalance { octas })
    }

    /// The values a Move view function returns, in order.
    async fn view(
        &self,
        function: &str,
        type_arguments: &[&str],
        arguments: &[&str],
    ) -> Result<Vec<Value>, ApiError> {
        let body = serde_json::json!({
            "function": function,
            "type_arguments": type_arguments,
            "arguments": arguments,
        });
        match self.post_val("/view", &body).await? {
            Value::Array(values) => Ok(values),
            other => Err(ApiError::Decode(format!(
                "view {function}: expected an array, got {other}"
            ))),
        }
    }

    /// What `owner` holds of a token, in its smallest unit.
    ///
    /// A fungible asset is read from the owner's primary store, which reads
    /// as zero when the owner has none. A coin is read through
    /// `0x1::coin::balance`, which adds the `CoinStore<T>` and the store of
    /// the asset the coin was migrated to: an account that holds the coin
    /// only as an asset has no `CoinStore` to read.
    pub async fn fetch_token_balance(
        &self,
        owner: &str,
        identifier: &str,
    ) -> Result<u64, ApiError> {
        let values = match AptosToken::parse(identifier) {
            AptosToken::Asset { metadata } => {
                self.view(
                    "0x1::primary_fungible_store::balance",
                    &[FUNGIBLE_ASSET_METADATA],
                    &[owner, metadata],
                )
                .await?
            }
            AptosToken::Coin { coin_type } => {
                self.view("0x1::coin::balance", &[coin_type], &[owner])
                    .await?
            }
        };
        returned_number(&values)
            .ok_or_else(|| ApiError::Decode(format!("aptos: no balance returned for {identifier}")))
    }

    /// A token's own decimals, from its metadata object or its `CoinInfo<T>`.
    /// `None` when it is unreadable.
    pub async fn fetch_token_decimals(&self, identifier: &str) -> Option<u8> {
        let values = match AptosToken::parse(identifier) {
            AptosToken::Asset { metadata } => {
                self.view(
                    "0x1::fungible_asset::decimals",
                    &[FUNGIBLE_ASSET_METADATA],
                    &[metadata],
                )
                .await
            }
            AptosToken::Coin { coin_type } => {
                self.view("0x1::coin::decimals", &[coin_type], &[]).await
            }
        };
        crate::api::checked_token_decimals(u128::from(returned_number(&values.ok()?)?)).ok()
    }

    pub async fn fetch_account_info(&self, address: &str) -> Result<(u64, u64), ApiError> {
        let resp: Value = self.get(&format!("/accounts/{address}")).await?;
        let sequence: u64 = resp
            .get("sequence_number")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("account: missing sequence_number")?;
        Ok((sequence, 0))
    }

    /// An account's next sequence number and its authentication key, or
    /// `None` for an account not yet on the network.
    pub(crate) async fn fetch_account_auth(
        &self,
        address: &str,
    ) -> Result<Option<(u64, String)>, ApiError> {
        let resp: Value = match self.get(&format!("/accounts/{address}")).await {
            Err(ApiError::Status { status: 404, .. }) => return Ok(None),
            read => read?,
        };
        let sequence: u64 = resp
            .get("sequence_number")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("account: missing sequence_number")?;
        let key = resp
            .get("authentication_key")
            .and_then(|v| v.as_str())
            .or_decode("account: missing authentication_key")?
            .to_ascii_lowercase();
        Ok(Some((sequence, key)))
    }

    pub async fn fetch_ledger_info(&self) -> Result<(u64, String), ApiError> {
        let resp: Value = self.get("/").await?;
        let chain_id: u64 = resp
            .get("chain_id")
            .and_then(|v| v.as_u64())
            .or_decode("ledger: missing chain_id")?;
        let ledger_version: String = resp
            .get("ledger_version")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        Ok((chain_id, ledger_version))
    }

    pub async fn fetch_gas_price(&self) -> Result<u64, ApiError> {
        let resp: Value = self.get("/estimate_gas_price").await?;
        resp.get("gas_estimate")
            .and_then(|v| v.as_u64())
            .or_decode("estimate_gas_price: missing gas_estimate")
    }

    /// A committed ledger transaction, independent of who sent it or whether
    /// it used a sequence number. The indexer supplies the relevant versions.
    pub(crate) async fn fetch_transaction_version(&self, version: u64) -> Result<Value, ApiError> {
        self.get(&format!("/transactions/by_version/{version}"))
            .await
    }
}

impl AptosClient {
    /// Submit a signed transaction in BCS, as the REST API takes one whose
    /// authenticator its JSON form does not name; the hash it answers.
    pub(crate) async fn submit_signed_bcs(
        &self,
        signed: &[u8],
    ) -> Result<AptosSendResult, ApiError> {
        let body = signed.to_vec();
        let response: Value = race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            let body = body.clone();
            async move {
                let (status, bytes) = client
                    .post_bytes(
                        &url,
                        "application/x.aptos.signed_transaction+bcs",
                        body,
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                if !(200..300).contains(&status) {
                    return Err(ApiError::Status {
                        status,
                        body: String::from_utf8_lossy(&bytes).into_owned(),
                    });
                }
                serde_json::from_slice(&bytes)
                    .map_err(|e| ApiError::Decode(format!("Aptos submit: {e}")))
            }
        })
        .await?;
        let txid = response["hash"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_decode("Aptos submit: missing hash")?
            .to_string();
        Ok(AptosSendResult {
            txid,
            version: None,
            signed_body_json: hex::encode(signed),
        })
    }

    pub async fn submit_signed_body(&self, signed_json: &str) -> Result<AptosSendResult, ApiError> {
        let body: Value = serde_json::from_str(signed_json)
            .map_err(|e| ApiError::InvalidInput(format!("invalid Aptos transaction: {e}")))?;
        let response = self.post_val("/transactions", &body).await?;
        let txid = response["hash"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_decode("Aptos submit: missing hash")?
            .to_string();
        let version = response["version"].as_str().and_then(|s| s.parse().ok());
        Ok(AptosSendResult {
            txid,
            version,
            signed_body_json: signed_json.into(),
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct AptosValidator {
    pub addr: String,
    pub voting_power: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AptosDelegation {
    pub active: u64,
    pub inactive: u64,
    pub pending_inactive: u64,
    pub withdrawable: u64,
    pub locked_until: u64,
    pub allowlisted: bool,
}

impl AptosClient {
    pub(crate) async fn fetch_delegation_commission(
        &self,
        pool: &str,
    ) -> Result<Option<u64>, ApiError> {
        let exists = self
            .view("0x1::delegation_pool::delegation_pool_exists", &[], &[pool])
            .await?;
        match exists.as_slice() {
            [Value::Bool(false)] => return Ok(None),
            [Value::Bool(true)] => {}
            _ => return Err(ApiError::decode("Invalid delegation-pool existence result")),
        }
        let commission = self
            .view(
                "0x1::delegation_pool::operator_commission_percentage",
                &[],
                &[pool],
            )
            .await?;
        let amount = returned_number(&commission)
            .filter(|v| *v <= 10_000)
            .or_decode("Invalid delegation-pool commission")?;
        Ok(Some(amount))
    }

    pub(crate) async fn fetch_add_stake_fee(
        &self,
        pool: &str,
        amount: u64,
    ) -> Result<u64, ApiError> {
        returned_number(
            &self
                .view(
                    "0x1::delegation_pool::get_add_stake_fee",
                    &[],
                    &[pool, &amount.to_string()],
                )
                .await?,
        )
        .filter(|v| *v <= amount)
        .or_decode("Invalid delegation-pool add-stake fee")
    }

    /// No usable signature is sent. The node executes this exact entry function
    /// at its current ledger state and reports the gas needed and resulting events.
    pub(crate) async fn simulate_staking(
        &self,
        body: &Value,
        public: &[u8; 32],
        estimate: bool,
        amount: u64,
    ) -> Result<(u64, u64), ApiError> {
        let mut body = body.clone();
        body["signature"] = serde_json::json!({"type":"ed25519_signature","public_key":format!("0x{}",hex::encode(public)),"signature":format!("0x{}",hex::encode([0u8;64]))});
        let path = if estimate {
            "/transactions/simulate?estimate_max_gas_amount=true"
        } else {
            "/transactions/simulate"
        };
        let result = self.post_val(path, &body).await?;
        let tx = result
            .as_array()
            .filter(|v| v.len() == 1)
            .and_then(|v| v.first())
            .or_decode("Aptos staking simulation: invalid response")?;
        if tx["success"].as_bool() != Some(true) {
            return Err(ApiError::invalid(format!(
                "Aptos staking simulation refused: {}",
                tx["vm_status"]
            )));
        }
        let number = |key: &str| {
            tx[key]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_decode("Aptos simulation: invalid gas")
        };
        let gas = number("gas_used")?;
        let maximum = number("max_gas_amount")?;
        let function = body["payload"]["function"]
            .as_str()
            .or_decode("Missing delegation entry function")?;
        let (event, field) = if function.ends_with("::add_stake") {
            ("AddStake", "amount_added")
        } else if function.ends_with("::unlock") {
            ("UnlockStake", "amount_unlocked")
        } else {
            ("WithdrawStake", "amount_withdrawn")
        };
        let matches = tx["events"]
            .as_array()
            .or_decode("Aptos staking simulation: missing events")?
            .iter()
            .filter(|e| {
                e["type"]
                    .as_str()
                    .is_some_and(|s| s.ends_with(&format!("::delegation_pool::{event}")))
            })
            .collect::<Vec<_>>();
        if matches.len() != 1
            || matches[0]["data"][field]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                != Some(amount)
        {
            return Err(ApiError::invalid(
                "Aptos delegation would adjust the reviewed amount; select an exact supported amount",
            ));
        }
        Ok((gas, maximum))
    }

    pub(crate) async fn fetch_delegation(
        &self,
        pool: &str,
        owner: &str,
    ) -> Result<Option<AptosDelegation>, ApiError> {
        let exists = self
            .view("0x1::delegation_pool::delegation_pool_exists", &[], &[pool])
            .await?;
        match exists.as_slice() {
            [Value::Bool(false)] => return Ok(None),
            [Value::Bool(true)] => {}
            _ => {
                return Err(ApiError::decode(
                    "delegation pool: invalid existence result",
                ));
            }
        }
        let stake = self
            .view("0x1::delegation_pool::get_stake", &[], &[pool, owner])
            .await?;
        let withdrawal = self
            .view(
                "0x1::delegation_pool::get_pending_withdrawal",
                &[],
                &[pool, owner],
            )
            .await?;
        let commission = self
            .view(
                "0x1::delegation_pool::operator_commission_percentage",
                &[],
                &[pool],
            )
            .await?;
        let allowed = self
            .view(
                "0x1::delegation_pool::delegator_allowlisted",
                &[],
                &[pool, owner],
            )
            .await?;
        let resource: Value = self
            .get(&format!("/accounts/{pool}/resource/0x1::stake::StakePool"))
            .await?;
        let unit = |value: &Value| {
            value
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_decode("delegation pool: invalid balance")
        };
        if stake.len() != 3 || withdrawal.len() != 2 || commission.len() != 1 || allowed.len() != 1
        {
            return Err(ApiError::decode("delegation pool: invalid view shape"));
        }
        let active = unit(&stake[0])?;
        let inactive = unit(&stake[1])?;
        let pending_inactive = unit(&stake[2])?;
        let withdrawal_ready = withdrawal[0]
            .as_bool()
            .or_decode("delegation pool: invalid unlock state")?;
        let withdrawable = if withdrawal_ready {
            unit(&withdrawal[1])?
        } else {
            0
        };
        let total_unlocked = inactive
            .checked_add(pending_inactive)
            .or_decode("delegation pool balance overflow")?;
        if withdrawable > total_unlocked {
            return Err(ApiError::decode(
                "delegation pool: inconsistent withdrawal balance",
            ));
        }
        let commission_bps = unit(&commission[0])?;
        if commission_bps > 10_000 {
            return Err(ApiError::decode("delegation pool: invalid commission"));
        }
        Ok(Some(AptosDelegation {
            active,
            inactive,
            pending_inactive,
            withdrawable,
            locked_until: unit(&resource["data"]["locked_until_secs"])?,
            allowlisted: allowed[0]
                .as_bool()
                .or_decode("delegation pool: invalid allowlist state")?,
        }))
    }
}

impl AptosClient {
    /// Active consensus validators, not a claim that each accepts delegation.
    pub async fn fetch_staking_validators(&self) -> Result<Vec<AptosValidator>, ApiError> {
        #[derive(Deserialize)]
        struct Resource {
            data: ValidatorSet,
        }
        #[derive(Deserialize)]
        struct ValidatorSet {
            active_validators: Vec<AptosValidator>,
        }
        let response: Resource = self
            .get("/accounts/0x1/resource/0x1::stake::ValidatorSet")
            .await?;
        Ok(response.data.active_validators)
    }
}
