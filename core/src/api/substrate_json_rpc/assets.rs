//! `Assets` pallet reads, pinned to one verified Asset Hub state: an asset's
//! rules, an account's holding of it, and its decimals. Each value is decoded
//! by the runtime's own type for it.
use super::*;
use parity_scale_codec::Encode;

/// Whether an asset moves at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetStatus {
    Live,
    /// Its admin froze it: no transfer moves it.
    Frozen,
    /// Being destroyed: nothing moves it again.
    Destroying,
}

/// Whether one account's holding of an asset moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetAccountStatus {
    Liquid,
    /// It may receive, not send.
    Frozen,
    /// It may neither send nor receive.
    Blocked,
}

/// What a transfer of an asset depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetDetails {
    /// A holding below this does not exist: a new one must start at it, and
    /// one left with less is removed.
    pub min_balance: u128,
    /// Whether a holding of it keeps an account alive by itself. One that is
    /// not needs the account to exist in its own right.
    pub is_sufficient: bool,
    pub status: AssetStatus,
}

/// One account's holding of an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssetHolding {
    pub balance: u128,
    pub status: AssetAccountStatus,
}

/// An asset and both sides of a transfer of it, at one block.
#[derive(Debug)]
pub struct AssetTransferState {
    pub context: PolkadotContext,
    pub call: AssetTransferCall,
    pub details: AssetDetails,
    pub decimals: u8,
    pub sender: Option<AssetHolding>,
    pub recipient: Option<AssetHolding>,
    /// The recipient's own balance of the network's coin.
    pub recipient_native: SubstrateBalance,
}

fn invalid() -> ApiError {
    ApiError::Decode("Invalid Assets pallet state".into())
}

fn number(value: &Value) -> Result<u128, ApiError> {
    value
        .as_str()
        .ok_or_else(invalid)?
        .parse()
        .map_err(ApiError::decode)
}

impl SubstrateClient {
    async fn asset_details(
        &self,
        metadata: &metadata::Metadata,
        id: u32,
        at: &str,
    ) -> Result<AssetDetails, ApiError> {
        let value = self
            .storage_value(metadata, "Assets", "Asset", &id.encode(), at)
            .await?
            .ok_or_else(|| ApiError::invalid(format!("No asset {id} on this network")))?;
        Ok(AssetDetails {
            min_balance: number(&value["min_balance"])?,
            is_sufficient: value["is_sufficient"].as_bool().ok_or_else(invalid)?,
            status: match value["status"].as_str() {
                Some("Live") => AssetStatus::Live,
                Some("Frozen") => AssetStatus::Frozen,
                Some("Destroying") => AssetStatus::Destroying,
                _ => return Err(invalid()),
            },
        })
    }

    async fn asset_holding(
        &self,
        metadata: &metadata::Metadata,
        id: u32,
        account: &[u8; 32],
        at: &str,
    ) -> Result<Option<AssetHolding>, ApiError> {
        let Some(value) = self
            .storage_value(metadata, "Assets", "Account", &(id, account).encode(), at)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(AssetHolding {
            balance: number(&value["balance"])?,
            status: match value["status"].as_str() {
                Some("Liquid") => AssetAccountStatus::Liquid,
                Some("Frozen") => AssetAccountStatus::Frozen,
                Some("Blocked") => AssetAccountStatus::Blocked,
                _ => return Err(invalid()),
            },
        }))
    }

    /// The decimals an asset's metadata states. An asset without metadata
    /// has none to read, and its amounts cannot be scaled: refused.
    async fn asset_decimals_at(
        &self,
        metadata: &metadata::Metadata,
        id: u32,
        at: &str,
    ) -> Result<u8, ApiError> {
        let value = self
            .storage_value(metadata, "Assets", "Metadata", &id.encode(), at)
            .await?
            .ok_or_else(|| ApiError::invalid(format!("Asset {id} states no decimals")))?;
        u8::try_from(number(&value["decimals"])?).map_err(ApiError::decode)
    }

    /// The asset `id` and both sides of a transfer of it, from one pinned
    /// state of this endpoint.
    pub async fn asset_transfer_state(
        &self,
        chain: Chain,
        id: u32,
        sender: &[u8; 32],
        recipient: &[u8; 32],
    ) -> Result<AssetTransferState, ApiError> {
        let (context, metadata) = self.pinned_runtime(chain).await?;
        let call = metadata.asset_transfer_call()?;
        let at = context.block_hash.clone();
        Ok(AssetTransferState {
            call,
            details: self.asset_details(&metadata, id, &at).await?,
            decimals: self.asset_decimals_at(&metadata, id, &at).await?,
            sender: self.asset_holding(&metadata, id, sender, &at).await?,
            recipient: self.asset_holding(&metadata, id, recipient, &at).await?,
            recipient_native: self.fetch_balance_at(chain, recipient, &at).await?,
            context,
        })
    }

    /// `holder`'s balance of each asset in `ids` and its decimals, from one
    /// verified state; an account that holds none has a balance of zero.
    /// Each asset answers on its own, so an unreadable one is that asset's
    /// error and not the others'.
    pub async fn fetch_asset_balances(
        &self,
        chain: Chain,
        ids: &[u32],
        holder: &[u8; 32],
    ) -> Result<Vec<Result<(u128, u8), ApiError>>, ApiError> {
        crate::api::http::race(&self.rpc_endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            let (context, metadata) = node.pinned_runtime(chain).await?;
            let at = &context.block_hash;
            let mut balances = Vec::with_capacity(ids.len());
            for &id in ids {
                balances.push(
                    async {
                        node.asset_details(&metadata, id, at).await?;
                        let decimals = node.asset_decimals_at(&metadata, id, at).await?;
                        let balance = node
                            .asset_holding(&metadata, id, holder, at)
                            .await?
                            .map_or(0, |holding| holding.balance);
                        Ok((balance, decimals))
                    }
                    .await,
                );
            }
            Ok(balances)
        })
        .await
    }

    /// The decimals asset `id`'s metadata states, from one verified state.
    pub async fn fetch_asset_decimals(&self, chain: Chain, id: u32) -> Result<u8, ApiError> {
        crate::api::http::race(&self.rpc_endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            let (context, metadata) = node.pinned_runtime(chain).await?;
            node.asset_details(&metadata, id, &context.block_hash)
                .await?;
            node.asset_decimals_at(&metadata, id, &context.block_hash)
                .await
        })
        .await
    }
}
