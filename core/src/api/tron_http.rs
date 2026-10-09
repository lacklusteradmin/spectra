//! The Tron node HTTP API adapter (`/wallet/...`): balances, TRC-10 assets, TRC-20 reads
//! through constant calls, block references and broadcast. Account history
//! and holdings come from `trongrid_v1`.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::http::{HttpClient, RetryProfile, race};
use sha2::{Digest, Sha256};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronBalance {
    /// SUN (1 TRX = 1_000_000 SUN).
    pub sun: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TronSendResult {
    pub txid: String,
    /// Full signed transaction JSON for rebroadcast. Serialized as a JSON string.
    #[serde(default)]
    pub signed_tx_json: String,
}

/// TRC-20 balance payload. Mirrors `Erc20Balance` so the Swift-side decoder
/// can share a single response type if desired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Balance {
    pub contract: String,
    pub holder: String,
    pub balance_raw: String,
    pub balance_display: String,
    pub decimals: u8,
    pub symbol: String,
}

/// Lightweight TRC-20 metadata (symbol + decimals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trc10Metadata {
    pub asset_id: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trc10Balance {
    pub metadata: Trc10Metadata,
    pub balance_raw: u64,
    pub balance_display: String,
}

pub(crate) struct Trc10TransferState {
    pub token: Trc10Balance,
    pub native_balance: u64,
    /// Upper bound if no issuer, staked or free bandwidth pays for the transfer.
    pub fee_budget_sun: u64,
    pub recipient_balance: u64,
}

pub(crate) fn validate_asset_id(id: &str) -> Result<(), ApiError> {
    if id.starts_with('0')
        || !id.bytes().all(|byte| byte.is_ascii_digit())
        || !id.parse::<i64>().is_ok_and(|id| id > 0)
    {
        return Err(ApiError::invalid(
            "TRC-10 token ID must be canonical positive int64 decimal",
        ));
    }
    Ok(())
}

fn trc10_metadata(value: &Value, id: &str) -> Result<Trc10Metadata, ApiError> {
    if value.get("id").and_then(Value::as_str) != Some(id) {
        return Err(ApiError::decode(
            "TRC-10 metadata is missing or names a different token",
        ));
    }
    let decode_text = |field: &str| -> Result<String, ApiError> {
        let raw = value
            .get(field)
            .and_then(Value::as_str)
            .or_decode("TRC-10 metadata text missing")?;
        let decoded = String::from_utf8(hex::decode(raw).map_err(ApiError::decode)?)
            .map_err(ApiError::decode)?;
        if decoded.trim().is_empty() || decoded.chars().any(char::is_control) {
            return Err(ApiError::decode(
                "TRC-10 metadata text is empty or contains control characters",
            ));
        }
        Ok(decoded)
    };
    let name = decode_text("name")?;
    // Issuance permits an empty abbreviation; protobuf JSON may either omit it
    // or emit the default empty bytes. Both spellings use the asset's name.
    let symbol = if value.get("abbr").is_some_and(|v| v.as_str() != Some("")) {
        decode_text("abbr")?
    } else {
        name.clone()
    };
    // Protobuf omits precision=0. TRC-10 issuance only permits 0..=6.
    let precision = match value.get("precision") {
        None => 0,
        Some(value) => value.as_u64().or_decode("Invalid TRC-10 precision")?,
    };
    let decimals = u8::try_from(precision)
        .ok()
        .filter(|n| *n <= 6)
        .or_decode("TRC-10 precision exceeds its protocol range")?;
    Ok(Trc10Metadata {
        asset_id: id.into(),
        name,
        symbol,
        decimals,
    })
}

fn trc10_account(value: &Value, address: &str) -> Result<(u64, Vec<(String, u64)>), ApiError> {
    let object = value
        .as_object()
        .or_decode("Invalid Tron account response")?;
    if let Some(error) = object.get("Error") {
        return Err(ApiError::rejected(format!("Tron account: {error}")));
    }
    if let Some(actual) = object.get("address") {
        if actual.as_str() != Some(address) {
            return Err(ApiError::decode(
                "Tron account response names a different owner",
            ));
        }
    } else if !object.is_empty() {
        return Err(ApiError::decode(
            "Tron account response is missing its owner",
        ));
    }
    let balance = match object.get("balance") {
        None => 0,
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= i64::MAX as u64)
            .or_decode("Invalid Tron native balance")?,
    };
    let mut held = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(assets) = object.get("assetV2") {
        for asset in assets.as_array().or_decode("Invalid TRC-10 holdings")? {
            let id = asset
                .get("key")
                .and_then(Value::as_str)
                .or_decode("TRC-10 holding has no ID")?;
            validate_asset_id(id)?;
            if !seen.insert(id) {
                return Err(ApiError::decode("Duplicate TRC-10 token balance"));
            }
            let amount = asset
                .get("value")
                .and_then(Value::as_u64)
                .filter(|n| *n <= i64::MAX as u64)
                .or_decode("Invalid TRC-10 balance")?;
            held.push((id.into(), amount));
        }
    }
    Ok((balance, held))
}

// ── Client

use crate::api::tron_metadata_cache::{self as metadata_cache, MetadataCache};

pub struct TronHttpClient {
    metadata_cache: Option<(crate::registry::Chain, std::sync::Arc<MetadataCache>)>,
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

/// An account's resources and their burn prices, as a node reports them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TronAccountResources {
    pub activated: bool,
    pub free_bandwidth_limit: u64,
    pub free_bandwidth_used: u64,
    pub staked_bandwidth_limit: u64,
    pub staked_bandwidth_used: u64,
    pub energy_limit: u64,
    pub energy_used: u64,
    /// Sun burned per bandwidth point a transaction lacks.
    pub bandwidth_price_sun: u64,
    /// Sun burned per energy unit a contract call lacks.
    pub energy_price_sun: u64,
}

/// One `getchainparameters` value by name, refused when absent or repeated.
fn chain_parameter(parameters: &Value, name: &str) -> Result<u64, ApiError> {
    let rows = parameters
        .get("chainParameter")
        .and_then(Value::as_array)
        .or_decode("Tron fee parameters missing")?;
    let mut values = rows
        .iter()
        .filter(|row| row.get("key").and_then(Value::as_str) == Some(name));
    let row = values
        .next()
        .or_decode("Tron required fee parameter missing")?;
    if values.next().is_some() {
        return Err(ApiError::decode("Duplicate Tron fee parameter"));
    }
    // Protobuf omits the default integer zero.
    match row.get("value") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= i64::MAX as u64)
            .or_decode("Invalid Tron fee parameter"),
    }
}

/// What a Tron account's policy read finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TronAccountPolicy {
    pub permissions: TronPermissions,
    pub balance_sun: u64,
    /// What the network charges a transaction signed by more than one key;
    /// 0 when it was not read.
    pub multi_sign_fee_sun: u64,
}

/// `wallet/getaccount`'s permissions (`visible: true`), as java-tron
/// writes them: a default value left out, so the owner's has no `type` or
/// `id`. An empty answer is an account not yet on the network, and one
/// without permissions an account that never set any.
pub(crate) fn account_permissions(
    account: &Value,
    address: &str,
) -> Result<TronPermissions, ApiError> {
    if account
        .as_object()
        .is_some_and(|account| account.is_empty())
    {
        return Ok(TronPermissions::single(address));
    }
    if account.get("address").and_then(Value::as_str) != Some(address) {
        return Err(ApiError::decode(
            "Tron account answer names another account",
        ));
    }
    // An account that never set its permissions holds java-tron's defaults:
    // its own key alone.
    if account.get("owner_permission").is_none() {
        return Ok(TronPermissions::single(address));
    }
    let permission = |value: &Value, owner: bool| -> Result<TronPermission, ApiError> {
        let number = |name: &str| match value.get(name) {
            None => Ok(0),
            Some(value) => value.as_u64().or_decode("Tron permission number"),
        };
        let id = u8::try_from(number("id")?).map_err(|_| ApiError::decode("Tron permission id"))?;
        let keys = value
            .get("keys")
            .and_then(Value::as_array)
            .or_decode("Tron permission keys")?
            .iter()
            .map(|key| {
                let address = key
                    .get("address")
                    .and_then(Value::as_str)
                    .or_decode("Tron permission key address")?;
                tron_base58_to_evm_hex(address).map_err(ApiError::decode)?;
                Ok(TronKey {
                    address: address.to_string(),
                    weight: key.get("weight").and_then(Value::as_u64).unwrap_or(0),
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        let operations = match value.get("operations").and_then(Value::as_str) {
            _ if owner => None,
            Some(hex) if hex.len() == 64 && hex::decode(hex).is_ok() => {
                Some(hex.to_ascii_lowercase())
            }
            _ => return Err(ApiError::decode("Tron active permission operations")),
        };
        Ok(TronPermission {
            id,
            name: value
                .get("permission_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            threshold: number("threshold")?,
            operations,
            keys,
        })
    };
    let owner = permission(
        account
            .get("owner_permission")
            .or_decode("Tron owner permission missing")?,
        true,
    )?;
    if owner.id != 0 {
        return Err(ApiError::decode("Tron owner permission id"));
    }
    let actives = account
        .get("active_permission")
        .and_then(Value::as_array)
        .map(|actives| {
            actives
                .iter()
                .map(|active| permission(active, false))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    TronPermissions { owner, actives }
        .checked()
        .map_err(|error| ApiError::decode(error.to_string()))
}

impl TronHttpClient {
    pub(crate) async fn transfer_reference_for(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<BlockReference, ApiError> {
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            node.transfer_reference().await
        })
        .await
    }

    async fn trc10_metadata_at(&self, id: &str) -> Result<Trc10Metadata, ApiError> {
        validate_asset_id(id)?;
        let value = self
            .post("/wallet/getassetissuebyid", &json!({"value":id}))
            .await?;
        trc10_metadata(&value, id)
    }

    async fn account_at(&self, address: &str) -> Result<Value, ApiError> {
        tron_base58_to_evm_hex(address).map_err(ApiError::invalid)?;
        self.post(
            "/wallet/getaccount",
            &json!({"address":address,"visible":true}),
        )
        .await
    }

    pub async fn fetch_trc10_metadata(
        &self,
        chain: crate::registry::Chain,
        id: &str,
    ) -> Result<Trc10Metadata, ApiError> {
        validate_asset_id(id)?;
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            node.trc10_metadata_at(id).await
        })
        .await
    }

    /// Before ALLOW_SAME_TOKEN_NAME, transfer bytes named an asset. Resolve
    /// through the node's unique-name API, then cross-check its current ID.
    /// A duplicate name, absent asset or mismatched response refuses the page.
    pub(crate) async fn fetch_legacy_trc10_metadata(
        &self,
        chain: crate::registry::Chain,
        name: &str,
    ) -> Result<Trc10Metadata, ApiError> {
        if name.is_empty()
            || name.len() > 32
            || !name.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(ApiError::invalid("Invalid legacy TRC-10 asset name"));
        }
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let value = node
                .post(
                    "/wallet/getassetissuebyname",
                    &json!({"value":hex::encode(name)}),
                )
                .await?;
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .or_decode("Legacy TRC-10 name has no unambiguous canonical ID")?;
            validate_asset_id(id)?;
            let legacy = trc10_metadata(&value, id)?;
            if legacy.name != name || legacy.decimals != 0 {
                return Err(ApiError::decode(
                    "Legacy TRC-10 name or pre-activation precision is inconsistent",
                ));
            }
            let current = node.trc10_metadata_at(id).await?;
            if current != legacy {
                return Err(ApiError::decode(
                    "Legacy TRC-10 name does not match its current asset identity",
                ));
            }
            Ok(current)
        })
        .await
    }

    pub async fn fetch_trc10_balance(
        &self,
        chain: crate::registry::Chain,
        id: &str,
        owner: &str,
    ) -> Result<Trc10Balance, ApiError> {
        validate_asset_id(id)?;
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let (metadata, account) =
                tokio::try_join!(node.trc10_metadata_at(id), node.account_at(owner))?;
            let (_, held) = trc10_account(&account, owner)?;
            let balance_raw = held
                .into_iter()
                .find(|(asset_id, _)| asset_id == id)
                .map_or(0, |(_, balance)| balance);
            let balance_display =
                crate::decimal::from_units(u128::from(balance_raw), u32::from(metadata.decimals));
            Ok(Trc10Balance {
                metadata,
                balance_raw,
                balance_display,
            })
        })
        .await
    }

    /// TRC-10 is native account state; it needs no TronGrid contract indexer.
    pub(crate) async fn fetch_trc10_holdings(
        &self,
        chain: crate::registry::Chain,
        owner: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let (_, held) = trc10_account(&node.account_at(owner).await?, owner)?;
            let reads = held
                .into_iter()
                .filter(|(_, balance)| *balance > 0)
                .map(|(id, raw)| {
                    let node = &node;
                    async move {
                        let metadata = node.trc10_metadata_at(&id).await?;
                        Ok::<_, ApiError>(crate::api::HeldToken {
                            contract: id,
                            balance_raw: u128::from(raw),
                            decimals: Some(metadata.decimals),
                        })
                    }
                });
            use futures::{StreamExt, TryStreamExt};
            futures::stream::iter(reads).buffered(8).try_collect().await
        })
        .await
    }

    pub(crate) async fn fetch_trc10_transfer_state(
        &self,
        chain: crate::registry::Chain,
        id: &str,
        owner: &str,
        receiver: Option<&str>,
        bandwidth_bytes: u64,
    ) -> Result<Trc10TransferState, ApiError> {
        validate_asset_id(id)?;
        if receiver == Some(owner) {
            return Err(ApiError::invalid("TRC-10 cannot transfer to its sender"));
        }
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let empty = json!({});
            let (metadata, account, parameters) = tokio::try_join!(
                node.trc10_metadata_at(id),
                node.account_at(owner),
                node.post("/wallet/getchainparameters", &empty)
            )?;
            let (native_balance, held) = trc10_account(&account, owner)?;
            let balance_raw = held
                .into_iter()
                .find(|(asset_id, _)| asset_id == id)
                .map_or(0, |(_, balance)| balance);
            let parameter = |name: &str| chain_parameter(&parameters, name);
            let recipient = if let Some(receiver) = receiver {
                let value = node.account_at(receiver).await?;
                let (_, recipient_held) = trc10_account(&value, receiver)?;
                let current = recipient_held
                    .iter()
                    .find(|(asset_id, _)| asset_id == id)
                    .map_or(0, |(_, n)| *n);
                Some((value, current))
            } else {
                None
            };
            let bandwidth_fee = parameter("getTransactionFee")?
                .checked_mul(bandwidth_bytes)
                .or_decode("Tron bandwidth fee overflow")?;
            let new_account = recipient.as_ref().is_none_or(|(account, _)| {
                account
                    .as_object()
                    .is_some_and(|account| account.is_empty())
            });
            let fee_budget_sun = if new_account {
                parameter("getCreateAccountFee")?
                    .checked_add(parameter("getCreateNewAccountFeeInSystemContract")?)
                    .or_decode("Tron activation fee overflow")?
            } else {
                bandwidth_fee
            };
            let fee_budget_sun = fee_budget_sun.max(bandwidth_fee);
            if recipient.as_ref().is_some_and(|(account, _)| {
                account.get("type").and_then(Value::as_str) == Some("Contract")
            }) && parameter("getForbidTransferToContract")? == 1
            {
                return Err(ApiError::invalid(
                    "TRC-10 transfer to a contract account is forbidden on this network",
                ));
            }
            let recipient_balance = recipient.map_or(0, |(_, amount)| amount);
            let balance_display =
                crate::decimal::from_units(u128::from(balance_raw), u32::from(metadata.decimals));
            Ok(Trc10TransferState {
                token: Trc10Balance {
                    metadata,
                    balance_raw,
                    balance_display,
                },
                native_balance,
                fee_budget_sun,
                recipient_balance,
            })
        })
        .await
    }

    /// An account's bandwidth and energy and the prices the network burns
    /// TRX at without them, read from one verified node.
    pub(crate) async fn fetch_account_resources(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<TronAccountResources, ApiError> {
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let resource_query = json!({"address": address, "visible": true});
            let empty = json!({});
            let (account, resources, parameters) = tokio::try_join!(
                node.account_at(address),
                node.post("/wallet/getaccountresource", &resource_query),
                node.post("/wallet/getchainparameters", &empty)
            )?;
            // Protobuf omits a zero.
            let count = |name: &str| resources.get(name).and_then(Value::as_u64).unwrap_or(0);
            Ok(TronAccountResources {
                activated: account
                    .as_object()
                    .is_some_and(|account| !account.is_empty()),
                free_bandwidth_limit: count("freeNetLimit"),
                free_bandwidth_used: count("freeNetUsed"),
                staked_bandwidth_limit: count("NetLimit"),
                staked_bandwidth_used: count("NetUsed"),
                energy_limit: count("EnergyLimit"),
                energy_used: count("EnergyUsed"),
                bandwidth_price_sun: chain_parameter(&parameters, "getTransactionFee")?,
                energy_price_sun: chain_parameter(&parameters, "getEnergyFee")?,
            })
        })
        .await
    }

    /// An account's owner and active permissions and its balance, read
    /// from one verified node; `multi_sign` also reads the fee the network
    /// charges a transaction signed by several keys. An account not yet on
    /// the network is its own key alone.
    pub(crate) async fn fetch_permissions(
        &self,
        chain: crate::registry::Chain,
        address: &str,
        multi_sign: bool,
    ) -> Result<TronAccountPolicy, ApiError> {
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let account = node.account_at(address).await?;
            let multi_sign_fee_sun = if multi_sign {
                let parameters = node.post("/wallet/getchainparameters", &json!({})).await?;
                chain_parameter(&parameters, "getMultiSignFee")?
            } else {
                0
            };
            Ok(TronAccountPolicy {
                permissions: account_permissions(&account, address)?,
                balance_sun: account.get("balance").and_then(Value::as_u64).unwrap_or(0),
                multi_sign_fee_sun,
            })
        })
        .await
    }

    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            metadata_cache: None,
            endpoints,
            client: HttpClient::shared(),
        }
    }

    /// Balance reads share metadata across wallets; signing uses `new` and fresh metadata.
    pub(crate) fn with_metadata_cache(
        endpoints: std::sync::Arc<Vec<String>>,
        chain: crate::registry::Chain,
        cache: std::sync::Arc<MetadataCache>,
    ) -> Self {
        Self {
            metadata_cache: Some((chain, cache)),
            ..Self::new(endpoints)
        }
    }

    pub(crate) async fn read_metadata(&self, contract: &str) -> Result<Trc20Metadata, ApiError> {
        match &self.metadata_cache {
            Some((chain, cache)) => {
                cache
                    .get_or_fetch(
                        metadata_cache::Key {
                            chain: *chain,
                            endpoints: self.endpoints.clone(),
                            contract: contract.to_owned(),
                        },
                        self.fetch_trc20_metadata(contract),
                    )
                    .await
            }
            None => self.fetch_trc20_metadata(contract).await,
        }
    }

    pub(crate) async fn post(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
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
// Tron fetch paths: balance, latest block, unified TRX+TRC-20 history,
// TRC-20 balance, TRC-20 metadata.

use serde_json::json;

use crate::derivation::tron::tron_base58_to_evm_hex;
use crate::send::tron_multisig::{TronKey, TronPermission, TronPermissions};

impl TronHttpClient {
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let expected = chain
            .tron_genesis_block_id()
            .or_decode("Missing Tron network identity")?;
        let result = self
            .post("/wallet/getblockbynum", &json!({"num":0}))
            .await?;
        if !result
            .get("blockID")
            .and_then(Value::as_str)
            .is_some_and(|id| id.eq_ignore_ascii_case(expected))
        {
            return Err(ApiError::invalid("Tron endpoint is on the wrong network"));
        }
        Ok(())
    }

    /// The solidity receipt is committed; a latest-head receipt can still be reverted.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::invalid("Invalid Tron transaction hash"));
        }
        let result = self
            .post(
                "/walletsolidity/gettransactioninfobyid",
                &json!({"value":hash}),
            )
            .await?;
        tron_transaction_status(&result, hash)
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<TronBalance, ApiError> {
        let resp = self
            .post(
                "/wallet/getaccount",
                &json!({"address": address, "visible": true}),
            )
            .await?;
        let sun = resp.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
        Ok(TronBalance { sun })
    }

    /// Read the live balance and share cached symbol/decimals when this client
    /// belongs to a service read path. A standalone client reads all three.
    pub async fn fetch_trc20_balance(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<Trc20Balance, ApiError> {
        let raw = self
            .fetch_trc20_balance_of(contract_base58, holder_base58)
            .await?;
        let metadata = self.read_metadata(contract_base58).await?;
        let balance_display = crate::decimal::from_units(raw, u32::from(metadata.decimals));
        Ok(Trc20Balance {
            contract: contract_base58.to_string(),
            holder: holder_base58.to_string(),
            balance_raw: raw.to_string(),
            balance_display,
            decimals: metadata.decimals,
            symbol: metadata.symbol,
        })
    }

    /// Raw `balanceOf(holder)` constant call.
    pub async fn fetch_trc20_balance_of(
        &self,
        contract_base58: &str,
        holder_base58: &str,
    ) -> Result<u128, ApiError> {
        // TRC-20 uses the same 4-byte selector as ERC-20, but Tron addresses are
        // passed in their *hex* form (0x41... stripped to the last 20 bytes).
        let holder_hex = tron_base58_to_evm_hex(holder_base58).map_err(ApiError::invalid)?;
        let parameter = format!("{:0>64}", holder_hex);

        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": holder_base58,
                    "contract_address": contract_base58,
                    "function_selector": "balanceOf(address)",
                    "parameter": parameter,
                    "visible": true
                }),
            )
            .await?;

        let hex_str = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .or_decode("triggerconstantcontract balanceOf: missing result")?;

        // The result is a 32-byte big-endian integer hex string.
        parse_abi_u128(hex_str)
    }

    /// Fetch token symbol + decimals.
    pub async fn fetch_trc20_metadata(
        &self,
        contract_base58: &str,
    ) -> Result<Trc20Metadata, ApiError> {
        // decimals()
        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": contract_base58,
                    "contract_address": contract_base58,
                    "function_selector": "decimals()",
                    "parameter": "",
                    "visible": true
                }),
            )
            .await?;
        let decimals_hex = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .or_decode("triggerconstantcontract decimals: missing result")?;
        let decimals = crate::api::checked_token_decimals(parse_abi_u128(decimals_hex)?)?;

        // symbol()
        let resp = self
            .post(
                "/wallet/triggerconstantcontract",
                &json!({
                    "owner_address": contract_base58,
                    "contract_address": contract_base58,
                    "function_selector": "symbol()",
                    "parameter": "",
                    "visible": true
                }),
            )
            .await?;
        let symbol_hex = resp
            .get("constant_result")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
            .or_decode("triggerconstantcontract symbol: missing result")?;
        let symbol = crate::api::evm_json_rpc::decode_abi_string_or_bytes32(symbol_hex)
            .or_decode("TRC20 symbol: malformed ABI string")?;

        Ok(Trc20Metadata { symbol, decimals })
    }
}

fn tron_transaction_status(
    result: &Value,
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    let object = result
        .as_object()
        .or_decode("Tron transaction receipt is not an object")?;
    if object.is_empty() {
        return Ok(TransactionStatus::Pending);
    }
    if let Some(error) = result.get("Error") {
        return Err(ApiError::rejected(format!(
            "Tron transaction receipt: {error}"
        )));
    }
    let actual = result
        .get("id")
        .and_then(Value::as_str)
        .or_decode("Tron receipt: missing id")?;
    if !actual.eq_ignore_ascii_case(hash) {
        return Err(ApiError::decode("Tron returned a different transaction"));
    }
    let block = result
        .get("blockNumber")
        .and_then(Value::as_u64)
        .or_decode("Tron receipt: missing block")?;
    // TransactionInfo.result defaults to SUCESS=0 and java-tron omits the
    // default protobuf field. Plain TRX transfers have no VM result either.
    let transaction_success = match result.get("result") {
        None => true,
        Some(Value::String(code)) if matches!(code.as_str(), "SUCESS" | "SUCCESS") => true,
        Some(Value::String(code)) if code == "FAILED" => false,
        _ => return Err(ApiError::decode("Tron receipt: invalid transaction result")),
    };
    let contract_success = match result.pointer("/receipt/result") {
        None => true,
        Some(Value::String(code)) if code == "SUCCESS" => true,
        Some(Value::String(code))
            if matches!(
                code.as_str(),
                "REVERT"
                    | "BAD_JUMP_DESTINATION"
                    | "OUT_OF_MEMORY"
                    | "PRECOMPILED_CONTRACT"
                    | "STACK_TOO_SMALL"
                    | "STACK_TOO_LARGE"
                    | "ILLEGAL_OPERATION"
                    | "STACK_OVERFLOW"
                    | "OUT_OF_ENERGY"
                    | "OUT_OF_TIME"
                    | "JVM_STACK_OVER_FLOW"
                    | "UNKNOWN"
                    | "TRANSFER_FAILED"
                    | "INVALID_CODE"
            ) =>
        {
            false
        }
        _ => return Err(ApiError::decode("Tron receipt: invalid contract result")),
    };
    Ok(TransactionStatus::Confirmed {
        succeeded: transaction_success && contract_success,
        block: Some(block),
    })
}

// ── TRC-20 helpers

/// A uint256 ABI word must be complete and fit the core's u128 amount type.
pub(crate) fn parse_abi_u128(hex_str: &str) -> Result<u128, ApiError> {
    let word = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    if word.len() != 64 || !word.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::Decode(
            "TRC20 integer: expected one 32-byte hex ABI word".into(),
        ));
    }
    if !word[..32].bytes().all(|b| b == b'0') {
        return Err(ApiError::Decode("TRC20 integer exceeds u128 range".into()));
    }
    u128::from_str_radix(&word[32..], 16)
        .map_err(|e| ApiError::Decode(format!("TRC20 integer: {e}")))
}

/// Only the block reference is supplied by the node, never a transaction/hash.
pub(crate) struct BlockReference {
    pub number: u64,
    pub id: [u8; 32],
    pub timestamp_ms: u64,
}

impl TronHttpClient {
    /// The latest block, which a transfer references; the node supplies
    /// nothing else of it.
    pub(crate) async fn transfer_reference(&self) -> Result<BlockReference, ApiError> {
        let block = self.post("/wallet/getnowblock", &json!({})).await?;
        let number = block
            .pointer("/block_header/raw_data/number")
            .and_then(Value::as_u64)
            .or_decode("missing Tron block number")?;
        let id = hex::decode(
            block["blockID"]
                .as_str()
                .or_decode("missing Tron block id")?,
        )
        .map_err(|_| ApiError::Decode("invalid Tron block id".into()))?
        .try_into()
        .map_err(|_| ApiError::Decode("Tron block id must be 32 bytes".into()))?;
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| ApiError::Decode("clock before epoch".into()))?
            .as_millis()
            .try_into()
            .map_err(|_| ApiError::Decode("clock overflow".into()))?;
        Ok(BlockReference {
            number,
            id,
            timestamp_ms,
        })
    }

    pub async fn broadcast_raw(&self, signed_tx_json: &str) -> Result<TronSendResult, ApiError> {
        let body: Value = serde_json::from_str(signed_tx_json)
            .map_err(|e| ApiError::InvalidInput(format!("invalid signed Tron transaction: {e}")))?;
        let raw = hex::decode(
            body["raw_data_hex"]
                .as_str()
                .or_decode("missing Tron raw bytes")?,
        )
        .map_err(|_| ApiError::InvalidInput("invalid Tron raw bytes".into()))?;
        let txid = hex::encode(Sha256::digest(&raw));
        if body["txID"].as_str() != Some(txid.as_str()) {
            return Err(ApiError::InvalidInput(
                "Tron transaction hash mismatch".into(),
            ));
        }
        let result = self.post("/wallet/broadcasttransaction", &body).await?;
        if result["result"].as_bool() != Some(true) {
            return Err(ApiError::Rejected(format!(
                "Tron broadcast refused: {result}"
            )));
        }
        Ok(TronSendResult {
            txid,
            signed_tx_json: signed_tx_json.into(),
        })
    }
}

#[cfg(test)]
mod trc10_tests {
    use super::*;
    use crate::registry::Chain;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    const OWNER: &str = "TJRabPrwbZy45sbavfcjinPJC18kjpRTv8";

    #[tokio::test]
    async fn legacy_trc10_names_require_unique_exact_current_identity() {
        use wiremock::matchers::body_json;
        for (name, named, current, succeeds) in [
            (
                "Legacy",
                json!({"id":"1000001","name":hex::encode("Legacy")}),
                json!({"id":"1000001","name":hex::encode("Legacy")}),
                true,
            ),
            (
                "1009999",
                json!({"id":"1000001","name":hex::encode("1009999")}),
                json!({"id":"1000001","name":hex::encode("1009999")}),
                true,
            ),
            (
                "Legacy",
                json!({"Error":"NonUniqueObjectException: more than one asset"}),
                json!({}),
                false,
            ),
            (
                "Legacy",
                json!({"id":"1000001","name":hex::encode("Other")}),
                json!({}),
                false,
            ),
            (
                "Legacy",
                json!({"id":"1000001","name":hex::encode("Legacy")}),
                json!({"id":"1000001","name":hex::encode("Other")}),
                false,
            ),
            (
                "Legacy",
                json!({"id":"1000001","name":hex::encode("Legacy")}),
                json!({"id":"1000002","name":hex::encode("Legacy")}),
                false,
            ),
            (
                "Legacy",
                json!({"id":"1000001","name":hex::encode("Legacy"),"precision":2}),
                json!({}),
                false,
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/wallet/getblockbynum"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(
                        json!({"blockID":Chain::Tron.tron_genesis_block_id().unwrap()}),
                    ),
                )
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/wallet/getassetissuebyname"))
                .and(body_json(json!({"value":hex::encode(name)})))
                .respond_with(ResponseTemplate::new(200).set_body_json(named))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/wallet/getassetissuebyid"))
                .and(body_json(json!({"value":"1000001"})))
                .respond_with(ResponseTemplate::new(200).set_body_json(current))
                .mount(&server)
                .await;
            let result = TronHttpClient::new(Arc::new(vec![server.uri()]))
                .fetch_legacy_trc10_metadata(Chain::Tron, name)
                .await;
            assert_eq!(result.is_ok(), succeeds, "name={name}, result={result:?}");
            if let Ok(asset) = result {
                assert_eq!(asset.asset_id, "1000001");
                assert_eq!(asset.decimals, 0);
            }
        }
    }

    #[test]
    fn trc10_precision_identity_and_account_amounts_are_strict() {
        let metadata = json!({"id":"1002000","name":hex::encode("Legacy"),"abbr":hex::encode("T10"),"precision":2});
        assert_eq!(trc10_metadata(&metadata, "1002000").unwrap().decimals, 2);
        assert!(trc10_metadata(&metadata, "1002001").is_err());
        for precision in [json!(-1), json!(7), json!("6")] {
            let mut malformed = metadata.clone();
            malformed["precision"] = precision;
            assert!(trc10_metadata(&malformed, "1002000").is_err());
        }
        let mut zero = metadata.clone();
        zero.as_object_mut().unwrap().remove("precision");
        assert_eq!(trc10_metadata(&zero, "1002000").unwrap().decimals, 0);
        zero["abbr"] = json!("");
        assert_eq!(trc10_metadata(&zero, "1002000").unwrap().symbol, "Legacy");
        let account = json!({"address":OWNER,"balance":1,"assetV2":[{"key":"1002000","value":23}]});
        assert_eq!(
            trc10_account(&account, OWNER).unwrap(),
            (1, vec![("1002000".into(), 23)])
        );
        assert_eq!(trc10_account(&json!({}), OWNER).unwrap(), (0, vec![]));
        for malformed in [
            json!({"Error":"refused"}),
            json!({"balance":1}),
            json!({"address":"wrong"}),
            json!({"address":OWNER,"assetV2":[{"key":"1002000","value":-1}]}),
            json!({"address":OWNER,"assetV2":[{"key":"1002000","value":1},{"key":"1002000","value":2}]}),
        ] {
            assert!(trc10_account(&malformed, OWNER).is_err());
        }
        for id in ["", "0", "001002000", "1002a", "9223372036854775808"] {
            assert!(validate_asset_id(id).is_err());
        }
    }

    #[tokio::test]
    async fn trc10_wrong_network_refuses_before_any_funds_or_metadata_read() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/wallet/getblockbynum"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"blockID":Chain::TronNile.tron_genesis_block_id().unwrap()}),
                ),
            )
            .mount(&server)
            .await;
        let client = TronHttpClient::new(Arc::new(vec![server.uri()]));
        assert!(
            client
                .fetch_trc10_balance(Chain::Tron, "1002000", OWNER)
                .await
                .is_err()
        );
        assert!(
            client
                .fetch_trc10_transfer_state(Chain::Tron, "1002000", OWNER, None, 300)
                .await
                .is_err()
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.url.path() == "/wallet/getblockbynum")
        );
    }
}

#[cfg(test)]
mod integer_tests {
    use super::*;
    #[test]
    fn abi_integers_never_truncate_or_accept_malformed_words() {
        assert_eq!(
            parse_abi_u128(&format!("{:064x}", u128::MAX)).unwrap(),
            u128::MAX
        );
        assert_eq!(parse_abi_u128(&"0".repeat(64)).unwrap(), 0);
        for bad in [
            "01".into(),
            "0".repeat(63),
            "0".repeat(65),
            format!("1{}", "0".repeat(63)),
            format!("{}z", "0".repeat(63)),
            format!("{}é", "0".repeat(62)),
        ] {
            assert!(parse_abi_u128(&bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn invalid_decimals_are_refused_before_reading_the_symbol() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};
        for result in [
            format!("{:064x}", 39),
            format!("{:064x}", 256),
            format!("1{}", "0".repeat(63)),
            "06".into(),
        ] {
            let server = MockServer::start().await;
            Mock::given(body_partial_json(json!({"function_selector":"decimals()"})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"constant_result":[result]})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert!(
                TronHttpClient::new(std::sync::Arc::new(vec![server.uri()]))
                    .fetch_trc20_metadata("contract")
                    .await
                    .is_err()
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }
}

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    #[tokio::test]
    async fn solidity_receipts_cover_native_vm_failure_and_missing_transaction() {
        let hash = "ab".repeat(32);
        for (receipt, expected) in [
            (json!({}), TransactionStatus::Pending),
            (
                json!({"id":hash,"blockNumber":450,"receipt":{"net_usage":280}}),
                TransactionStatus::Confirmed {
                    succeeded: true,
                    block: Some(450),
                },
            ),
            (
                json!({"id":hash,"blockNumber":451,"receipt":{"result":"SUCCESS"}}),
                TransactionStatus::Confirmed {
                    succeeded: true,
                    block: Some(451),
                },
            ),
            (
                json!({"id":hash,"blockNumber":452,"receipt":{"result":"OUT_OF_ENERGY"},"result":"FAILED"}),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(452),
                },
            ),
            (
                json!({"id":hash,"blockNumber":453,"receipt":{"result":"REVERT"}}),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(453),
                },
            ),
            (
                json!({"id":hash,"blockNumber":454,"result":"FAILED"}),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(454),
                },
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/walletsolidity/gettransactioninfobyid"))
                .and(body_json(json!({"value":hash})))
                .respond_with(ResponseTemplate::new(200).set_body_json(receipt))
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                TronHttpClient::new(Arc::new(vec![server.uri()]))
                    .fetch_transaction_status(&hash)
                    .await
                    .unwrap(),
                expected
            );
        }
        for bad in [
            json!({"id":"cd".repeat(32),"blockNumber":450}),
            json!({"id":hash,"receipt":{"result":"SUCCESS"}}),
            json!({"id":hash,"blockNumber":450,"result":"refused"}),
            json!({"id":hash,"blockNumber":450,"receipt":{"result":"DEFAULT"}}),
            json!({"Error":"node refused"}),
            Value::Null,
        ] {
            assert!(tron_transaction_status(&bad, &hash).is_err());
        }
    }

    #[tokio::test]
    async fn genesis_identity_is_bound_to_the_concrete_tron_network() {
        use crate::registry::Chain;
        for selected in [Chain::Tron, Chain::TronNile] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/wallet/getblockbynum"))
                .and(body_json(json!({"num":0})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(
                        json!({"blockID":selected.tron_genesis_block_id().unwrap()}),
                    ),
                )
                .mount(&server)
                .await;
            let client = TronHttpClient::new(Arc::new(vec![server.uri()]));
            client.verify_network(selected).await.unwrap();
            assert!(
                client
                    .verify_network(if selected == Chain::Tron {
                        Chain::TronNile
                    } else {
                        Chain::Tron
                    })
                    .await
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod metadata_cache_rpc_tests {
    use super::*;
    use serde_json::json;
    use std::{sync::Arc, time::Duration};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};

    #[tokio::test]
    async fn concurrent_balance_reads_share_metadata_but_sends_and_new_sources_read_fresh() {
        let server = MockServer::start().await;
        for (selector, result, expected) in [
            ("balanceOf(address)", format!("{:064x}", 1_000_000), 8),
            ("decimals()", format!("{:064x}", 6), 4),
            ("symbol()", format!("{:0<64}", hex::encode("TOKEN")), 4),
        ] {
            Mock::given(body_partial_json(json!({"function_selector":selector})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"constant_result":[result]}))
                        .set_delay(Duration::from_millis(20)),
                )
                .expect(expected)
                .mount(&server)
                .await;
        }
        let cache = Arc::new(MetadataCache::default());
        let endpoints = Arc::new(vec![server.uri()]);
        let contract = "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t";
        let holder = "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7";
        // Separate short-lived clients, just like separate wallet refreshes.
        let results = futures::future::join_all((0..8).map(|_| {
            let client = TronHttpClient::with_metadata_cache(
                endpoints.clone(),
                crate::registry::Chain::Tron,
                cache.clone(),
            );
            async move { client.fetch_trc20_balance(contract, holder).await.unwrap() }
        }))
        .await;
        for balance in results {
            assert_eq!(balance.decimals, 6);
            assert_eq!(balance.symbol, "TOKEN");
            assert_eq!(balance.balance_raw, "1000000");
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 10);
        // Even a cache-enabled client must bypass the cache for the explicit
        // metadata API used by the send builder.
        let reader = TronHttpClient::with_metadata_cache(
            endpoints.clone(),
            crate::registry::Chain::Tron,
            cache.clone(),
        );
        reader.fetch_trc20_metadata(contract).await.unwrap();
        TronHttpClient::with_metadata_cache(
            endpoints.clone(),
            crate::registry::Chain::TronNile,
            cache.clone(),
        )
        .read_metadata(contract)
        .await
        .unwrap();
        // A changed endpoint list is a different source, even for the same chain.
        let changed = Arc::new(vec![format!("{}/", server.uri())]);
        TronHttpClient::with_metadata_cache(changed, crate::registry::Chain::Tron, cache)
            .read_metadata(contract)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 16);
    }
}
