//! The Substrate JSON-RPC adapter, for Polkadot and Bittensor alike: the
//! `System.Account` balance, what signing needs, and extrinsic submission. A
//! node keeps no account history, and no keyless indexer is configured.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;
use crate::registry::Chain;

mod metadata;
pub mod pools;
pub use metadata::{PolkadotExtension, PolkadotRuntime};

const SYSTEM_EVENTS_KEY: &str =
    "0x26aa394eea5630e07c48ae0c9558cef780d41e5e16056765bc8461851072c9d7";

#[derive(Debug)]
pub struct PolkadotContext {
    pub runtime: PolkadotRuntime,
    pub block_hash: String,
    pub finalized_number: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SubstrateFinalizedOutcome {
    pub succeeded: bool,
    pub block_number: u64,
}

/// `twox128("System") ++ twox128("Account")`: the storage prefix of every
/// account record, fixed by the pallet and item names.
const SYSTEM_ACCOUNT_PREFIX: [u8; 32] = [
    0x26, 0xaa, 0x39, 0x4e, 0xea, 0x56, 0x30, 0xe0, 0x7c, 0x48, 0xae, 0x0c, 0x95, 0x58, 0xce, 0xf7,
    0xb9, 0x9d, 0x88, 0x0e, 0xc6, 0x81, 0x79, 0x9c, 0x0c, 0xf3, 0x0e, 0x88, 0x86, 0x37, 0x1d, 0xa9,
];

/// An account's `AccountData`, in the chain's smallest unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubstrateBalance {
    pub free: u128,
    pub reserved: u128,
    pub frozen: u128,
}

impl SubstrateBalance {
    /// What the account can move: `free` less the part `frozen` holds beyond
    /// what is already reserved. Staked funds are frozen, so they are not in it.
    pub fn transferable(self) -> u128 {
        self.free
            .saturating_sub(self.frozen.saturating_sub(self.reserved))
    }

    /// `Balances::reducible_balance(Preserve, Polite)`: retain the larger
    /// of the existential deposit and the freeze not covered by reserves.
    pub fn keep_alive_spendable(self, existential_deposit: u128) -> u128 {
        self.free
            .saturating_sub(existential_deposit.max(self.frozen.saturating_sub(self.reserved)))
    }
}

/// `System.Account`'s key for `account`: the prefix, `blake2_128(account)`,
/// then the account itself (`Blake2_128Concat`).
fn system_account_key(account: &[u8; 32]) -> String {
    use blake2::digest::consts::U16;
    use blake2::{Blake2b, Digest};
    let mut key = SYSTEM_ACCOUNT_PREFIX.to_vec();
    key.extend_from_slice(&Blake2b::<U16>::digest(account));
    key.extend_from_slice(account);
    format!("0x{}", hex::encode(key))
}

/// `AccountInfo`: four `u32` counters (nonce, consumers, providers,
/// sufficients), then `free`, `reserved` and `frozen` of `balance_bytes` each
/// and a `u128` of flags, all little-endian. Any other length is refused.
fn decode_account_info(bytes: &[u8], balance_bytes: usize) -> Result<SubstrateBalance, ApiError> {
    if balance_bytes > 16 || bytes.len() != 16 + 3 * balance_bytes + 16 {
        return Err(ApiError::Decode(format!(
            "Substrate account record is {} bytes, not the expected layout",
            bytes.len()
        )));
    }
    let read = |index: usize| {
        let start = 16 + index * balance_bytes;
        let mut word = [0u8; 16];
        word[..balance_bytes].copy_from_slice(&bytes[start..start + balance_bytes]);
        u128::from_le_bytes(word)
    };
    Ok(SubstrateBalance {
        free: read(0),
        reserved: read(1),
        frozen: read(2),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubstrateSendResult {
    pub txid: String,
    /// Hex-encoded signed extrinsic (0x-prefixed) — stored for rebroadcast.
    pub extrinsic_hex: String,
}

pub struct SubstrateClient {
    pub(crate) rpc_endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl SubstrateClient {
    pub fn new(rpc_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            rpc_endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn rpc_call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::SubstrateJsonRpc,
            &self.client,
            &self.rpc_endpoints,
            method,
            params,
        )
        .await
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u32, ApiError> {
        let result = self
            .rpc_call("system_accountNextIndex", json!([address]))
            .await?;
        let value = result
            .as_u64()
            .or_decode("system_accountNextIndex: expected number")?;
        u32::try_from(value).map_err(ApiError::decode)
    }

    /// Submit a signed extrinsic, fresh or saved for rebroadcast.
    pub async fn submit_extrinsic_hex(&self, hex: &str) -> Result<SubstrateSendResult, ApiError> {
        let result = self
            .rpc_call("author_submitExtrinsic", json!([hex]))
            .await?;
        let txid = decode_hash(&result)?;
        let expected = blake2_hash(&decode_hex(hex)?);
        if txid != expected {
            return Err(ApiError::Decode(
                "Node returned a different extrinsic hash".into(),
            ));
        }
        Ok(SubstrateSendResult {
            txid,
            extrinsic_hex: hex.to_string(),
        })
    }

    pub async fn verify_substrate_genesis(&self, chain: Chain) -> Result<String, ApiError> {
        let expected = chain
            .substrate_genesis_hash()
            .or_decode("Missing Substrate network identity")?;
        let actual = decode_hash(&self.rpc_call("chain_getBlockHash", json!([0])).await?)?;
        if actual != expected {
            return Err(ApiError::invalid(
                "Endpoint is on the wrong Substrate network",
            ));
        }
        Ok(actual)
    }

    /// Call on a single endpoint so the metadata, storage and quote share one
    /// network and one explicitly pinned state, even while a runtime upgrades.
    pub async fn polkadot_context(&self, chain: Chain) -> Result<PolkadotContext, ApiError> {
        let genesis_hash = self.verify_substrate_genesis(chain).await?;
        let block_hash = decode_hash(&self.rpc_call("chain_getBlockHash", json!([])).await?)?;
        let version = self
            .rpc_call("state_getRuntimeVersion", json!([block_hash]))
            .await?;
        let (spec_version, transaction_version) = decode_runtime_version(&version)?;
        let raw = self
            .rpc_call("state_getMetadata", json!([block_hash]))
            .await?;
        let bytes = decode_hex(raw.as_str().or_decode("Missing runtime metadata")?)?;
        let metadata = metadata::Metadata::decode(&bytes)?;
        let (transfer_pallet, transfer_call, existential_deposit, extensions) =
            metadata.contract(chain)?;
        let (_, finalized_number) = self.finalized_head().await?;
        Ok(PolkadotContext {
            runtime: PolkadotRuntime {
                spec_version,
                transaction_version,
                genesis_hash,
                metadata_hash: blake2_hash(&bytes),
                transfer_pallet,
                transfer_call,
                existential_deposit,
                extensions,
            },
            block_hash,
            finalized_number,
        })
    }

    pub async fn fetch_balance_at(
        &self,
        chain: Chain,
        account: &[u8; 32],
        block_hash: &str,
    ) -> Result<SubstrateBalance, ApiError> {
        let value = self
            .rpc_call(
                "state_getStorage",
                json!([system_account_key(account), block_hash]),
            )
            .await?;
        match value {
            Value::Null => Ok(SubstrateBalance {
                free: 0,
                reserved: 0,
                frozen: 0,
            }),
            Value::String(hex) => decode_account_info(
                &decode_hex(&hex)?,
                chain
                    .substrate_balance_bytes()
                    .or_decode("Unsupported Substrate balance layout")?,
            ),
            _ => Err(ApiError::Decode("Invalid System.Account storage".into())),
        }
    }

    /// A verified runtime and storage snapshot from one concrete network.
    pub async fn fetch_balance(
        &self,
        chain: Chain,
        account: &[u8; 32],
    ) -> Result<SubstrateBalance, ApiError> {
        crate::api::http::race(&self.rpc_endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            let context = node.polkadot_context(chain).await?;
            node.fetch_balance_at(chain, account, &context.block_hash)
                .await
        })
        .await
    }

    pub async fn query_fee(&self, extrinsic: &[u8], block_hash: &str) -> Result<u128, ApiError> {
        let value = self
            .rpc_call(
                "payment_queryInfo",
                json!([format!("0x{}", hex::encode(extrinsic)), block_hash]),
            )
            .await?;
        let fee = value["partialFee"]
            .as_str()
            .or_decode("Missing Asset Hub fee quote")?
            .parse::<u128>()
            .map_err(ApiError::decode)?;
        if fee == 0 {
            return Err(ApiError::Decode("Invalid zero Substrate fee quote".into()));
        }
        Ok(fee)
    }

    pub async fn finalized_head(&self) -> Result<(String, u64), ApiError> {
        let hash = decode_hash(&self.rpc_call("chain_getFinalizedHead", json!([])).await?)?;
        let header = self.rpc_call("chain_getHeader", json!([hash])).await?;
        let number = parse_block_number(&header["number"])?;
        Ok((hash, number))
    }

    /// Scan a bounded contiguous range of finalized blocks. The returned
    /// cursor is safe to persist; an error never skips unverified blocks.
    pub async fn finalized_outcome(
        &self,
        chain: Chain,
        transaction_hash: &str,
        after: u64,
        limit: u64,
    ) -> Result<(Option<SubstrateFinalizedOutcome>, u64), ApiError> {
        self.verify_substrate_genesis(chain).await?;
        let expected = decode_hash(&Value::String(transaction_hash.to_string()))?;
        let (_, finalized) = self.finalized_head().await?;
        let end = finalized.min(after.saturating_add(limit));
        if end <= after {
            return Ok((None, after));
        }
        for number in after + 1..=end {
            let hash = decode_hash(&self.rpc_call("chain_getBlockHash", json!([number])).await?)?;
            let block = self.rpc_call("chain_getBlock", json!([hash])).await?;
            if parse_block_number(&block["block"]["header"]["number"])? != number {
                return Err(ApiError::Decode(
                    "Finalized block number does not match request".into(),
                ));
            }
            let extrinsics = block["block"]["extrinsics"]
                .as_array()
                .or_decode("Missing finalized extrinsics")?;
            let mut found = None;
            for (index, extrinsic) in extrinsics.iter().enumerate() {
                if blake2_hash(&decode_hex(
                    extrinsic
                        .as_str()
                        .or_decode("Invalid finalized extrinsic")?,
                )?) == expected
                {
                    found = Some(u32::try_from(index).map_err(ApiError::decode)?);
                    break;
                }
            }
            if let Some(index) = found {
                // The parent state contains the runtime which executed this
                // block, including a block that installs a runtime upgrade.
                let parent = decode_hash(&block["block"]["header"]["parentHash"])?;
                let raw = self.rpc_call("state_getMetadata", json!([parent])).await?;
                let metadata = metadata::Metadata::decode(&decode_hex(
                    raw.as_str().or_decode("Missing block runtime metadata")?,
                )?)?;
                let events = self
                    .rpc_call("state_getStorage", json!([SYSTEM_EVENTS_KEY, hash]))
                    .await?;
                let succeeded = metadata.dispatch_outcome(
                    &decode_hex(
                        events
                            .as_str()
                            .or_decode("Missing finalized dispatch events")?,
                    )?,
                    index,
                )?;
                return Ok((
                    Some(SubstrateFinalizedOutcome {
                        succeeded,
                        block_number: number,
                    }),
                    number,
                ));
            }
        }
        Ok((None, end))
    }
}

fn decode_runtime_version(value: &Value) -> Result<(u32, u32), ApiError> {
    let number = |field| {
        let value = value[field]
            .as_u64()
            .filter(|v| *v != 0)
            .or_decode("Invalid Substrate runtime version")?;
        u32::try_from(value).map_err(ApiError::decode)
    };
    Ok((number("specVersion")?, number("transactionVersion")?))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, ApiError> {
    Ok(hex::decode(
        value
            .strip_prefix("0x")
            .or_decode("Missing SCALE hex prefix")?,
    )?)
}

fn decode_hash(value: &Value) -> Result<String, ApiError> {
    let bytes = decode_hex(
        value
            .as_str()
            .or_decode("Missing Substrate block/transaction hash")?,
    )?;
    if bytes.len() != 32 {
        return Err(ApiError::Decode("Invalid Substrate hash length".into()));
    }
    Ok(format!("0x{}", hex::encode(bytes)))
}

fn parse_block_number(value: &Value) -> Result<u64, ApiError> {
    let text = value
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .or_decode("Invalid Substrate block number")?;
    u64::from_str_radix(text, 16).map_err(ApiError::decode)
}

fn blake2_hash(bytes: &[u8]) -> String {
    use blake2::{Blake2b, Digest, digest::consts::U32};
    format!("0x{}", hex::encode(Blake2b::<U32>::digest(bytes)))
}

#[cfg(test)]
mod account_tests {
    use super::*;

    /// Records read from `rpc.polkadot.io` (the treasury, `u128` balances)
    /// and `entrypoint-finney.opentensor.ai` (`u64` balances), 2026-09-29.
    #[test]
    fn account_records_decode_at_each_chains_balance_width() {
        let treasury: [u8; 32] =
            hex::decode("6d6f646c70792f74727372790000000000000000000000000000000000000000")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(
            system_account_key(&treasury),
            "0x26aa394eea5630e07c48ae0c9558cef7b99d880ec681799c0cf30e8886371da95ecffd7b6c0f78751baa9d281e0bfa3a6d6f646c70792f74727372790000000000000000000000000000000000000000"
        );
        let polkadot = hex::decode("000000000000000001000000000000001a8ea401a31900000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000080").unwrap();
        assert_eq!(
            decode_account_info(&polkadot, 16).unwrap(),
            SubstrateBalance {
                free: 28_187_897_925_146,
                reserved: 0,
                frozen: 0
            }
        );
        let bittensor = hex::decode("88ff1000000000000100000000000000a006fe05000000000000000000000000000000000000000000000000000000000000000000000080").unwrap();
        assert_eq!(
            decode_account_info(&bittensor, 8).unwrap(),
            SubstrateBalance {
                free: 100_533_920,
                reserved: 0,
                frozen: 0
            }
        );
        // Read at the wrong width, either record is refused, not misread.
        assert!(decode_account_info(&polkadot, 8).is_err());
        assert!(decode_account_info(&bittensor, 16).is_err());
    }

    #[test]
    fn frozen_funds_beyond_the_reserve_are_not_transferable() {
        let staked = SubstrateBalance {
            free: 100,
            reserved: 10,
            frozen: 40,
        };
        assert_eq!(staked.transferable(), 70);
        let covered = SubstrateBalance {
            free: 100,
            reserved: 50,
            frozen: 40,
        };
        assert_eq!(covered.transferable(), 100);
    }
}

#[cfg(test)]
pub(crate) mod tests;
