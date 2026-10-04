//! Nomination Pools requests, pinned to one verified Asset Hub state.
use super::*;
use parity_scale_codec::{Decode, Encode};
use std::sync::Arc;

/// Runtime facts needed by the local nomination-pool call builder.
#[derive(Clone)]
pub struct PoolCallMetadata {
    pub types: scale_info::PortableRegistry,
    pub pool_pallet: u8,
    pub pool_call_type: u32,
    pub utility_pallet: u8,
    pub batch_all_call: u8,
}
#[derive(Debug, Clone)]
pub struct NominationPool {
    pub id: u32,
    pub name: String,
    pub is_open: bool,
    pub commission: Option<f64>,
    pub active_balance: u128,
    pub needs_migration: bool,
    pub minimum_join: u128,
}
#[derive(Debug, Clone)]
pub struct PoolMember {
    pub pool_id: u32,
    pub points: u128,
    pub active_balance: u128,
    pub pending_rewards: u128,
    pub unbonding_balance: u128,
    pub withdrawable_balance: u128,
    pub next_unlock_era: Option<u32>,
    pub needs_migration: bool,
    pub pool_needs_migration: bool,
    pub pending_slash: u128,
}
pub struct PoolsSnapshot {
    pub context: PolkadotContext,
    pub minimum_join: u128,
    pub member: Option<PoolMember>,
}

fn invalid() -> ApiError {
    ApiError::Decode("Invalid Nomination Pools state".into())
}
fn number(value: &Value) -> Result<u128, ApiError> {
    value
        .as_str()
        .ok_or_else(invalid)?
        .parse()
        .map_err(ApiError::decode)
}
fn u32_number(value: &Value) -> Result<u32, ApiError> {
    u32::try_from(number(value)?).map_err(ApiError::decode)
}
fn ratio(points: u128, balance: u128, total: u128) -> Result<u128, ApiError> {
    if points == 0 {
        return Ok(0);
    }
    if total == 0 || points > total {
        return Err(invalid());
    }
    ((num_bigint::BigUint::from(points) * num_bigint::BigUint::from(balance))
        / num_bigint::BigUint::from(total))
    .try_into()
    .map_err(ApiError::decode)
}

impl SubstrateClient {
    async fn pools_metadata(
        &self,
        context: &PolkadotContext,
    ) -> Result<metadata::Metadata, ApiError> {
        let value = self
            .rpc_call("state_getMetadata", json!([context.block_hash]))
            .await?;
        metadata::Metadata::decode(&decode_hex(value.as_str().ok_or_else(invalid)?)?)
    }
    async fn pool_storage(
        &self,
        metadata: &metadata::Metadata,
        pallet: &str,
        item: &str,
        key: &[u8],
        at: &str,
    ) -> Result<Option<Value>, ApiError> {
        let key = metadata.storage_key(pallet, item, key)?;
        let raw = self.rpc_call("state_getStorage", json!([key, at])).await?;
        if raw.is_null() {
            return Ok(None);
        }
        Ok(Some(metadata.decode_storage(
            pallet,
            item,
            &decode_hex(raw.as_str().ok_or_else(invalid)?)?,
        )?))
    }
    async fn pool_runtime<T: Decode>(
        &self,
        name: &str,
        input: &[u8],
        at: &str,
    ) -> Result<T, ApiError> {
        let result = self
            .rpc_call(
                "state_call",
                json!([
                    format!("NominationPoolsApi_{name}"),
                    format!("0x{}", hex::encode(input)),
                    at
                ]),
            )
            .await?;
        let bytes = decode_hex(result.as_str().ok_or_else(invalid)?)?;
        let mut input = bytes.as_slice();
        let result = T::decode(&mut input).map_err(ApiError::decode)?;
        if !input.is_empty() {
            return Err(invalid());
        }
        Ok(result)
    }
    async fn pool_at(
        &self,
        metadata: &metadata::Metadata,
        id: u32,
        at: &str,
    ) -> Result<NominationPool, ApiError> {
        let pool = self
            .pool_storage(metadata, "NominationPools", "BondedPools", &id.encode(), at)
            .await?
            .ok_or_else(invalid)?;
        let name = self
            .pool_storage(metadata, "NominationPools", "Metadata", &id.encode(), at)
            .await?
            .and_then(|value| value.as_array().cloned())
            .and_then(|bytes| {
                bytes
                    .iter()
                    .map(|v| u8::try_from(number(v).ok()?).ok())
                    .collect::<Option<Vec<_>>>()
            })
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Pool {id}"));
        let commission = pool
            .pointer("/commission/current")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_array()
                    .and_then(|value| value.first())
                    .ok_or_else(invalid)
                    .and_then(number)
            })
            .transpose()?;
        if commission.is_some_and(|commission| commission > 1_000_000_000) {
            return Err(invalid());
        }
        let active_balance = self
            .pool_runtime::<u128>(
                "points_to_balance",
                &(id, number(&pool["points"])?).encode(),
                at,
            )
            .await?;
        let needs_migration = self
            .pool_runtime::<bool>("pool_needs_delegate_migration", &id.encode(), at)
            .await?;
        Ok(NominationPool {
            id,
            name,
            is_open: pool["state"] == "Open",
            commission: commission.map(|c| c as f64 / 1_000_000_000.0),
            active_balance,
            needs_migration,
            minimum_join: number(
                &self
                    .pool_storage(metadata, "NominationPools", "MinJoinBond", &[], at)
                    .await?
                    .ok_or_else(invalid)?,
            )?,
        })
    }
    pub async fn fetch_nomination_pools(&self) -> Result<Vec<NominationPool>, ApiError> {
        crate::api::http::race(&self.rpc_endpoints, |endpoint| async move {
            let node = Self::new(Arc::new(vec![endpoint]));
            let context = node.polkadot_context(Chain::Polkadot).await?;
            let metadata = node.pools_metadata(&context).await?;
            let prefix = format!(
                "0x{}",
                hex::encode(metadata.storage_prefix("NominationPools", "BondedPools")?)
            );
            let mut ids = Vec::new();
            let mut start: Option<String> = None;
            loop {
                let keys = node
                    .rpc_call(
                        "state_getKeysPaged",
                        json!([prefix, 100, start, context.block_hash]),
                    )
                    .await?;
                let keys = keys.as_array().ok_or_else(invalid)?;
                if keys.is_empty() {
                    break;
                }
                for key in keys {
                    let key = key.as_str().ok_or_else(invalid)?;
                    let bytes = decode_hex(key)?;
                    if !key.starts_with(&prefix) || bytes.len() < 4 {
                        return Err(invalid());
                    }
                    let id = u32::from_le_bytes(
                        bytes[bytes.len() - 4..]
                            .try_into()
                            .map_err(ApiError::decode)?,
                    );
                    if metadata.storage_key("NominationPools", "BondedPools", &id.encode())? != key
                    {
                        return Err(invalid());
                    }
                    ids.push(id);
                }
                let next = keys
                    .last()
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?
                    .to_string();
                if start.as_ref() == Some(&next) || ids.len() > 10_000 {
                    return Err(invalid());
                }
                start = Some(next);
                if keys.len() < 100 {
                    break;
                }
            }
            use futures::{StreamExt, TryStreamExt};
            let mut pools: Vec<_> = futures::stream::iter(
                ids.into_iter()
                    .map(|id| node.pool_at(&metadata, id, &context.block_hash)),
            )
            .buffer_unordered(8)
            .try_collect()
            .await?;
            pools.sort_by_key(|pool| pool.id);
            Ok(pools)
        })
        .await
    }
    pub async fn nomination_pool_snapshot(
        &self,
        owner: &[u8; 32],
    ) -> Result<PoolsSnapshot, ApiError> {
        let context = self.polkadot_context(Chain::Polkadot).await?;
        let metadata = self.pools_metadata(&context).await?;
        metadata.require_async_staking()?;
        let at = &context.block_hash;
        let minimum_join = number(
            &self
                .pool_storage(&metadata, "NominationPools", "MinJoinBond", &[], at)
                .await?
                .ok_or_else(invalid)?,
        )?;
        let Some(member) = self
            .pool_storage(&metadata, "NominationPools", "PoolMembers", owner, at)
            .await?
        else {
            return Ok(PoolsSnapshot {
                context,
                minimum_join,
                member: None,
            });
        };
        let pool_id = u32_number(&member["pool_id"])?;
        let points = number(&member["points"])?;
        let active_balance = self
            .pool_runtime::<u128>("points_to_balance", &(pool_id, points).encode(), at)
            .await?;
        let pending_rewards = self
            .pool_runtime::<Option<u128>>("pending_rewards", owner, at)
            .await?
            .ok_or_else(invalid)?;
        let needs_migration = self
            .pool_runtime::<bool>("member_needs_delegate_migration", owner, at)
            .await?;
        let pool_needs_migration = self
            .pool_runtime::<bool>("pool_needs_delegate_migration", &pool_id.encode(), at)
            .await?;
        let pending_slash = self
            .pool_runtime::<u128>("member_pending_slash", owner, at)
            .await?;
        let era = self
            .pool_storage(&metadata, "Staking", "ActiveEra", &[], at)
            .await?
            .ok_or_else(invalid)?;
        let era = u32_number(&era["index"])?;
        let eras = member["unbonding_eras"].as_array().ok_or_else(invalid)?;
        let mut unbonding_balance = 0u128;
        let mut withdrawable_balance = 0u128;
        let mut next_unlock_era = None;
        if !eras.is_empty() {
            let subpools = self
                .pool_storage(
                    &metadata,
                    "NominationPools",
                    "SubPoolsStorage",
                    &pool_id.encode(),
                    at,
                )
                .await?
                .ok_or_else(invalid)?;
            let with_era = subpools["with_era"].as_array().ok_or_else(invalid)?;
            for pair in eras {
                let pair = pair
                    .as_array()
                    .filter(|pair| pair.len() == 2)
                    .ok_or_else(invalid)?;
                let unlock = u32_number(&pair[0])?;
                let pool = with_era
                    .iter()
                    .find(|p| {
                        p.as_array()
                            .and_then(|p| p.first())
                            .is_some_and(|p| u32_number(p).ok() == Some(unlock))
                    })
                    .map(|p| p.get(1).ok_or_else(invalid))
                    .transpose()?
                    .unwrap_or(&subpools["no_era"]);
                let value = ratio(
                    number(&pair[1])?,
                    number(&pool["balance"])?,
                    number(&pool["points"])?,
                )?;
                if unlock <= era {
                    withdrawable_balance = withdrawable_balance
                        .checked_add(value)
                        .ok_or_else(invalid)?;
                } else {
                    unbonding_balance = unbonding_balance.checked_add(value).ok_or_else(invalid)?;
                    next_unlock_era =
                        Some(next_unlock_era.map_or(unlock, |previous: u32| previous.min(unlock)));
                }
            }
        }
        Ok(PoolsSnapshot {
            context,
            minimum_join,
            member: Some(PoolMember {
                pool_id,
                points,
                active_balance,
                pending_rewards,
                unbonding_balance,
                withdrawable_balance,
                next_unlock_era,
                needs_migration,
                pool_needs_migration,
                pending_slash,
            }),
        })
    }
    pub async fn pool_call_metadata(&self, at: &str) -> Result<PoolCallMetadata, ApiError> {
        let value = self.rpc_call("state_getMetadata", json!([at])).await?;
        metadata::Metadata::decode(&decode_hex(value.as_str().ok_or_else(invalid)?)?)?
            .pool_call_metadata()
    }
    pub async fn pool_at_snapshot(&self, id: u32, at: &str) -> Result<NominationPool, ApiError> {
        let value = self.rpc_call("state_getMetadata", json!([at])).await?;
        let metadata =
            metadata::Metadata::decode(&decode_hex(value.as_str().ok_or_else(invalid)?)?)?;
        self.pool_at(&metadata, id, at).await
    }
    pub async fn pool_points_for_balance(
        &self,
        pool: u32,
        amount: u128,
        at: &str,
    ) -> Result<u128, ApiError> {
        self.pool_runtime("balance_to_points", &(pool, amount).encode(), at)
            .await
    }
}

#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod ratio_tests {
    use super::*;
    #[test]
    fn ratio_accounts_for_slashing_without_overflow() {
        assert_eq!(ratio(100, 90, 100).unwrap(), 90);
        assert_eq!(ratio(u128::MAX, u128::MAX, u128::MAX).unwrap(), u128::MAX);
        assert!(ratio(1, 1, 0).is_err());
    }
}
