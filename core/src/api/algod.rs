//! An Algorand node's REST API (algod v2): the network's genesis, the
//! suggested fee and current round, an account's balance, submission of a
//! signed transaction and a transaction the node still remembers.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::Value;

use crate::api::error::{ApiError, OrDecode};
use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::transaction_status::TransactionStatus;
use crate::registry::Chain;

/// What a transaction is priced and dated from, as `/v2/transactions/params`
/// answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AlgorandParams {
    /// The newest round the node has: a transaction built now is valid from
    /// it.
    pub last_round: u64,
    /// The least fee a transaction pays, in microAlgos.
    pub min_fee: u64,
    /// The fee per byte of signed transaction the node asks while it is
    /// congested; zero otherwise.
    pub fee_per_byte: u64,
}

/// An account as a send reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AlgorandAccount {
    /// Everything it holds, in microAlgos.
    pub amount: u64,
    /// What it must keep while it holds anything: 0.1 ALGO, raised by each
    /// asset and application it opts into. A node answers 0.1 ALGO for an
    /// address that holds nothing.
    pub min_balance: u64,
    /// The address whose key signs for this account when it has been
    /// rekeyed to another; `None` when its own key signs.
    pub auth_address: Option<String>,
}

impl AlgorandAccount {
    /// What it can send, fees included: its amount above its minimum.
    pub(crate) fn spendable(&self) -> u64 {
        self.amount.saturating_sub(self.min_balance)
    }
}

/// Refuse a transaction id that is not 52 letters of base32 naming 32
/// bytes, before it reaches a URL.
pub(crate) fn validate_txid(txid: &str) -> Result<(), ApiError> {
    if txid.len() != 52
        || data_encoding::BASE32_NOPAD
            .decode(txid.as_bytes())
            .map_or(true, |bytes| bytes.len() != 32)
    {
        return Err(ApiError::invalid("Invalid Algorand transaction id"));
    }
    Ok(())
}

/// Whether `value`'s `genesis-id` and base64 `genesis-hash` name `chain`'s
/// network. Refused unless both are there and both match.
pub(crate) fn check_genesis(chain: Chain, value: &Value) -> Result<(), ApiError> {
    let network = chain.algorand_network()?;
    let id = value["genesis-id"]
        .as_str()
        .or_decode("Algorand: missing genesis id")?;
    let hash = value["genesis-hash"]
        .as_str()
        .and_then(|hash| base64::engine::general_purpose::STANDARD.decode(hash).ok())
        .or_decode("Algorand: missing genesis hash")?;
    if id != network.genesis_id || hash != network.genesis_hash {
        return Err(ApiError::invalid(format!(
            "Endpoint is on {id}, not {}",
            network.genesis_id
        )));
    }
    Ok(())
}

/// The message a node's error body carries, or the body itself.
fn node_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| value["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string())
}

pub struct AlgodClient {
    endpoints: Arc<Vec<String>>,
    client: Arc<HttpClient>,
}

impl AlgodClient {
    pub fn new(endpoints: Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    /// The suggested parameters of a node on `chain`'s network: refused
    /// unless the same answer names its genesis id and hash, so the round
    /// and fee a transaction takes come from the network it binds.
    pub(crate) async fn fetch_params(&self, chain: Chain) -> Result<AlgorandParams, ApiError> {
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            async move {
                let value: Value = client
                    .get_json(
                        &format!("{}/v2/transactions/params", base.trim_end_matches('/')),
                        RetryProfile::ChainRead,
                    )
                    .await?;
                params_from(chain, &value)
            }
        })
        .await
    }

    /// Refuse a node that is not on `chain`'s network.
    pub(crate) async fn verify_network(&self, chain: Chain) -> Result<(), ApiError> {
        self.fetch_params(chain).await.map(|_| ())
    }

    /// An account's amount, minimum and signer. An address that holds
    /// nothing answers an amount of zero.
    pub(crate) async fn fetch_account(&self, address: &str) -> Result<AlgorandAccount, ApiError> {
        let address = crate::derivation::algorand::decode_address(address)
            .map(|key| crate::derivation::algorand::address_from_public_key(&key))
            .map_err(|_| ApiError::invalid("Not an Algorand address"))?;
        let value = self
            .get(&format!("/v2/accounts/{address}?exclude=all"))
            .await?;
        account_from(&value, &address)
    }

    /// Everything `address` holds, in microAlgos.
    pub(crate) async fn fetch_balance(&self, address: &str) -> Result<u64, ApiError> {
        Ok(self.fetch_account(address).await?.amount)
    }

    /// Submit a signed transaction's msgpack and answer the id the node
    /// took it under. A node that refuses it answers its reason, which is a
    /// rejection.
    pub(crate) async fn broadcast(&self, signed: &[u8]) -> Result<String, ApiError> {
        let body = signed.to_vec();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}/v2/transactions", base.trim_end_matches('/'));
            let body = body.clone();
            async move {
                let (status, bytes) = client
                    .post_bytes(&url, "application/x-binary", body, RetryProfile::ChainWrite)
                    .await?;
                let text = String::from_utf8_lossy(&bytes);
                if !(200..300).contains(&status) {
                    return Err(ApiError::rejected(format!(
                        "Algorand node refused the transaction: {}",
                        node_message(&text)
                    )));
                }
                let value: Value = serde_json::from_slice(&bytes)?;
                let txid = value["txId"]
                    .as_str()
                    .or_decode("Algorand submit: missing transaction id")?;
                validate_txid(txid)?;
                Ok(txid.to_string())
            }
        })
        .await
    }

    /// Whether the node has committed `txid`, from its pool and its last
    /// 1,000 rounds: `None` when it remembers neither, which an indexer
    /// then answers. A committed transaction always ran: Algorand includes
    /// none that fails.
    pub(crate) async fn fetch_transaction_status(
        &self,
        txid: &str,
    ) -> Result<Option<TransactionStatus>, ApiError> {
        validate_txid(txid)?;
        match self.get(&format!("/v2/transactions/pending/{txid}")).await {
            Ok(value) => Ok(Some(pending_status(&value)?)),
            Err(ApiError::Status { status: 404, .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// The parameters in a `/v2/transactions/params` answer, refused unless it
/// names `chain`'s genesis.
pub(crate) fn params_from(chain: Chain, value: &Value) -> Result<AlgorandParams, ApiError> {
    check_genesis(chain, value)?;
    let field = |name: &str| {
        value[name]
            .as_u64()
            .ok_or_else(|| ApiError::decode(format!("Algorand params: missing {name}")))
    };
    let params = AlgorandParams {
        last_round: field("last-round")?,
        min_fee: field("min-fee")?,
        fee_per_byte: field("fee")?,
    };
    if params.last_round == 0 || params.min_fee == 0 {
        return Err(ApiError::decode("Algorand params: no round or minimum fee"));
    }
    Ok(params)
}

/// An account answer for `address`, refused when it is another's.
pub(crate) fn account_from(value: &Value, address: &str) -> Result<AlgorandAccount, ApiError> {
    if value["address"].as_str() != Some(address) {
        return Err(ApiError::decode("Algorand account: another address"));
    }
    let field = |name: &str| {
        value[name]
            .as_u64()
            .ok_or_else(|| ApiError::decode(format!("Algorand account: missing {name}")))
    };
    Ok(AlgorandAccount {
        amount: field("amount")?,
        min_balance: field("min-balance")?,
        auth_address: value["auth-addr"]
            .as_str()
            .filter(|auth| !auth.is_empty() && *auth != address)
            .map(str::to_string),
    })
}

/// A pending-transaction answer: committed at its `confirmed-round`, or
/// still in the pool. An evicted transaction (`pool-error`) is pending too:
/// another node may still commit it until its last valid round.
pub(crate) fn pending_status(value: &Value) -> Result<TransactionStatus, ApiError> {
    if !value["txn"].is_object() {
        return Err(ApiError::decode("Algorand pending: missing transaction"));
    }
    Ok(
        match value["confirmed-round"].as_u64().filter(|round| *round > 0) {
            Some(round) => TransactionStatus::Confirmed {
                succeeded: true,
                block: Some(round),
            },
            None => TransactionStatus::Pending,
        },
    )
}

#[cfg(test)]
#[path = "tests/algod.rs"]
mod tests;
