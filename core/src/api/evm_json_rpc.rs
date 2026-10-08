//! The EVM JSON-RPC adapter, for every EVM chain: balances, nonces, fee
//! history, gas estimates, ERC-20 reads through `eth_call`, receipts and raw
//! transaction broadcast. Address history is not a node method; that is
//! `blockscout`.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::evm_nft::NftStandard;
use crate::api::http::{HttpClient, RetryProfile, race};

/// The ERC-165 calls that ask whether `contract` is an NFT collection: does
/// it implement ERC-721, ERC-1155, and ERC-165's own invalid interface.
fn nft_interface_calls(contract: &str) -> Vec<(&'static str, Value)> {
    [
        NftStandard::Erc721.interface_id(),
        NftStandard::Erc1155.interface_id(),
        crate::api::evm_nft::INVALID_INTERFACE,
    ]
    .into_iter()
    .map(|interface| {
        let data = crate::api::evm_nft::supports_interface_call(interface);
        (
            "eth_call",
            json!([{"to": contract, "data": format!("0x{}", hex::encode(data))}, "latest"]),
        )
    })
    .collect()
}

/// What the three answers to [`nft_interface_calls`] claim: ERC-721 and
/// ERC-1155. A revert, an error or anything but a single true word claims
/// nothing, and a contract that claims the invalid interface claims every
/// one, so none.
fn nft_claims(answers: &[Result<Value, ApiError>]) -> (bool, bool) {
    let claimed = |answer: &Result<Value, ApiError>| {
        answer
            .as_ref()
            .ok()
            .and_then(Value::as_str)
            .and_then(|hex| decode_hex(hex).ok())
            .is_some_and(|word| crate::api::evm_nft::is_abi_true(&word))
    };
    if claimed(&answers[2]) {
        return (false, false);
    }
    (claimed(&answers[0]), claimed(&answers[1]))
}

/// The refusal for reading an NFT collection as a fungible token.
fn not_fungible() -> ApiError {
    ApiError::invalid("This contract is an NFT collection, not a fungible token")
}

// ── ERC-20 4-byte function selectors  (keccak256(signature)[..4])
pub(crate) const SEL_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31]; // balanceOf(address)
pub(crate) const SEL_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67]; // decimals()
pub(crate) const SEL_SYMBOL: [u8; 4] = [0x95, 0xd8, 0x9b, 0x41]; // symbol()
const SEL_NAME: [u8; 4] = [0x06, 0xfd, 0xde, 0x03]; // name()
pub(crate) const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)
pub(crate) const SEL_APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3]; // approve(address,uint256)
pub(crate) const SEL_ALLOWANCE: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e]; // allowance(address,address)

// ── ENS: the registry and the resolver methods a primary name is read with
const ENS_REGISTRY: &str = "0x00000000000c2e074ec69a0dfb2997ba6c7d2e1e";
const SEL_ENS_RESOLVER: [u8; 4] = [0x01, 0x78, 0xb8, 0xbf]; // resolver(bytes32)
const SEL_ENS_NAME: [u8; 4] = [0x69, 0x1f, 0x34, 0x31]; // name(bytes32)
const SEL_ENS_ADDR: [u8; 4] = [0x3b, 0x3b, 0x57, 0xde]; // addr(bytes32)

// ── Internal helpers shared by derive/fetch/send

/// Strip the `0x` prefix from a hex string and decode to bytes.
pub(crate) fn decode_hex(s: &str) -> Result<Vec<u8>, ApiError> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(stripped).map_err(|e| ApiError::Decode(format!("hex decode: {e}")))
}

/// Parse a `0x`-prefixed hex integer (as returned by JSON-RPC) into u128.
pub(crate) fn parse_hex_u128(s: &str) -> Result<u128, ApiError> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u128::from_str_radix(stripped, 16).map_err(|e| ApiError::Decode(format!("hex u128 parse: {e}")))
}

/// Parse a `0x`-prefixed hex integer into u64.
pub(crate) fn parse_hex_u64(s: &str) -> Result<u64, ApiError> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(stripped, 16).map_err(|e| ApiError::Decode(format!("hex u64 parse: {e}")))
}

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmBalance {
    /// Native token balance in the chain's smallest unit (wei for ETH).
    pub balance_wei: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmFeeEstimate {
    /// EIP-1559 base fee (wei).
    pub base_fee_wei: u128,
    /// Suggested priority fee / miner tip (wei).
    pub priority_fee_wei: u128,
    /// Max total fee per gas to set on the transaction.
    pub max_fee_per_gas_wei: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmSendResult {
    pub txid: String,
    /// Nonce actually used for this transaction. Set by the signing path
    /// (either the caller's override or the fetched pending nonce).
    #[serde(default)]
    pub nonce: u64,
    /// Signed transaction bytes as a 0x-prefixed hex string — callers that
    /// need to re-broadcast or log the raw envelope can use this directly.
    /// Empty on `broadcast_raw` paths where the raw hex was already supplied.
    #[serde(default)]
    pub raw_tx_hex: String,
    /// Gas limit used for the transaction.
    #[serde(default)]
    pub gas_limit: u64,
    /// EIP-1559 max fee per gas (wei, decimal string).
    #[serde(default)]
    pub max_fee_per_gas_wei: String,
    /// EIP-1559 max priority fee per gas (wei, decimal string).
    #[serde(default)]
    pub max_priority_fee_per_gas_wei: String,
}

/// Balance of an ERC-20 token held at a given address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Erc20Balance {
    /// Token contract (checksummed lowercase hex, 0x-prefixed).
    pub contract: String,
    /// Holder address.
    pub holder: String,
    /// Raw balance in the token's smallest unit (u256 encoded as decimal string).
    pub balance_raw: String,
    /// The balance scaled by `decimals`, exactly.
    pub balance_display: String,
    /// Token decimals (cached from the contract).
    pub decimals: u8,
    /// Token symbol.
    pub symbol: String,
}

/// Lightweight ERC-20 metadata (symbol + decimals).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Erc20Metadata {
    pub symbol: String,
    pub decimals: u8,
}

/// Transaction receipt returned by `eth_getTransactionReceipt`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvmReceipt {
    pub tx_hash: String,
    /// Block number where the transaction was included, or `None` if still pending.
    pub block_number: Option<u64>,
    /// `"0x1"` = success, `"0x0"` = reverted. `None` = legacy (pre-Byzantium) chains.
    pub status: Option<String>,
    /// Actual gas consumed (decimal string).
    pub gas_used: Option<String>,
    /// Effective gas price in wei (decimal string).
    pub effective_gas_price_wei: Option<String>,
    /// Actual L1 data charge, read from the receipt rather than an estimate.
    pub l1_fee_wei: Option<String>,
    /// Actual operator charge at the receipt's historical block. Missing is unknown.
    pub operator_fee_wei: Option<String>,
    /// `true` when the transaction has been included in a block.
    pub is_confirmed: bool,
    /// `true` when status == "0x0" (execution failed / reverted).
    pub is_failed: bool,
}

// ── EVM client

pub struct EvmClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) chain_id: u64,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl EvmClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>, chain_id: u64) -> Self {
        Self {
            endpoints,
            chain_id,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::EvmJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }

    /// A JSON-RPC 2.0 batch whose requests answer independently, in the
    /// order of `requests` however the server orders its answer: the outer
    /// error is the batch's (transport, shape), the inner ones each request's
    /// own (a revert, an RPC error object).
    pub(crate) async fn call_batch_each(
        &self,
        requests: Vec<(&str, Value)>,
    ) -> Result<Vec<Result<Value, ApiError>>, ApiError> {
        let n = requests.len();
        if n == 0 {
            return Ok(vec![]);
        }
        let batch: Value = Value::Array(
            requests
                .into_iter()
                .enumerate()
                .map(|(i, (method, params))| {
                    json!({
                        "jsonrpc": "2.0",
                        "id": (i + 1) as u64,
                        "method": method,
                        "params": params,
                    })
                })
                .collect(),
        );
        let body = std::sync::Arc::new(batch);
        race(&self.endpoints, |url| {
            let client = self.client.clone();
            let body = std::sync::Arc::clone(&body);
            async move {
                let resp: Value = client
                    .post_json(&url, &*body, RetryProfile::ChainRead)
                    .await?;
                let arr = resp
                    .as_array()
                    .or_decode("batch: expected array response")?;
                // JSON-RPC 2.0 allows out-of-order batch responses; re-order by id.
                let mut indexed: Vec<(usize, Value)> = arr
                    .iter()
                    .map(|v| {
                        let id = v["id"].as_u64().unwrap_or(0) as usize;
                        (id, v.clone())
                    })
                    .collect();
                indexed.sort_unstable_by_key(|(id, _)| *id);
                if indexed.len() != n {
                    return Err(ApiError::Decode(format!(
                        "batch: expected {n} responses, got {}",
                        indexed.len()
                    )));
                }
                Ok(indexed
                    .into_iter()
                    .map(|(_, v)| {
                        if let Some(err) = v.get("error") {
                            return Err(ApiError::Rejected(format!("rpc error: {err}")));
                        }
                        v.get("result")
                            .cloned()
                            .or_decode("batch: missing result field")
                    })
                    .collect())
            }
        })
        .await
    }
}

// EVM fetch paths: native balance, nonce, fee estimate, gas estimate, code,
// receipts, tx nonce by hash, ENS resolution, ERC-20 balance + metadata,
// Keyless explorer history + token transfers.

fn parse_fee_history(result: &Value) -> Result<EvmFeeEstimate, ApiError> {
    // Base fee of the *next* block is the last entry in baseFeePerGas.
    let base_fees = result
        .get("baseFeePerGas")
        .and_then(|v| v.as_array())
        .or_decode("feeHistory: missing baseFeePerGas")?;
    let base_fee_hex = base_fees
        .last()
        .and_then(|v| v.as_str())
        .or_decode("feeHistory: empty baseFeePerGas")?;
    let base_fee_wei = parse_hex_u128(base_fee_hex)?;

    // 25th-percentile reward from the most recent block as priority fee.
    //
    // Two different things look alike here and only one of them is a failure.
    // A node that answers without a `reward` array did not answer the question
    // this call asked — the request always names percentiles — and there is
    // nothing to infer from that, so it is an error. A node that answers with
    // an *empty* sample list for the block did answer: no transaction in it
    // paid a priority fee, which is the normal state of a quiet L2 or testnet.
    // The priority fee there is zero, not a number picked to stand in for one:
    // `max_fee` still covers the base fee, so the transaction lands, and a
    // fabricated gwei would only overpay. Refusing that case instead left the
    // send screen unable to quote a fee at all on those chains.
    let samples = result
        .get("reward")
        .and_then(|r| r.as_array())
        .and_then(|blocks| blocks.last())
        .and_then(|block| block.as_array())
        .or_decode("feeHistory: missing reward")?;
    let priority_fee_wei: u128 = match samples.first() {
        Some(sample) => parse_hex_u128(
            sample
                .as_str()
                .or_decode("feeHistory: reward sample is not a hex string")?,
        )?,
        None => 0,
    };

    // maxFeePerGas = 2 * baseFee + priorityFee (EIP-1559 recommended).
    let max_fee_per_gas_wei = base_fee_wei
        .checked_mul(2)
        .and_then(|base| base.checked_add(priority_fee_wei))
        .or_decode("feeHistory: fee exceeds u128")?;

    Ok(EvmFeeEstimate {
        base_fee_wei,
        priority_fee_wei,
        max_fee_per_gas_wei,
    })
}

impl EvmClient {
    pub(crate) fn chain(&self) -> Result<crate::registry::Chain, ApiError> {
        crate::registry::Chain::all()
            .find(|chain| chain.evm_chain_id().ok() == Some(self.chain_id))
            .ok_or_else(|| ApiError::invalid("unknown EVM network"))
    }

    /// Estimate the complete reviewed transaction, including fee fields and access list.
    pub(crate) async fn estimate_transaction_gas(
        &self,
        transaction: Value,
    ) -> Result<u64, ApiError> {
        let result = self.call("eth_estimateGas", json!([transaction])).await?;
        let gas = parse_hex_u64(
            result
                .as_str()
                .or_decode("eth_estimateGas: expected string")?,
        )?;
        if gas == 0 {
            return Err(ApiError::Decode("gas estimate must be positive".into()));
        }
        Ok(gas)
    }

    /// OP Stack's oracle prices L1 data separately from EIP-1559 execution gas.
    /// The upper bound includes the signature; operator fees use the reviewed gas limit.
    pub(crate) async fn fetch_rollup_fee(
        &self,
        unsigned_transaction: &[u8],
        gas: u64,
    ) -> Result<u128, ApiError> {
        use crate::registry::OpStackFeeModel;
        let Some(model) = self.chain()?.evm_rollup_fee_model() else {
            return Ok(0);
        };
        let oracle = "0x420000000000000000000000000000000000000F";
        let read = async |data: String| {
            let result = self
                .call(
                    "eth_call",
                    json!([{
                "to": oracle, "data": data,
            }, "latest"]),
                )
                .await?;
            let bytes = decode_hex(result.as_str().or_decode("gas oracle: expected hex")?)?;
            if bytes.len() != 32 || bytes[..16].iter().any(|byte| *byte != 0) {
                return Err(ApiError::Decode(
                    "gas oracle fee is not a u128 ABI word".into(),
                ));
            }
            Ok(u128::from_be_bytes(
                bytes[16..].try_into().expect("ABI word"),
            ))
        };
        let size = unsigned_transaction.len();
        let upper_bound = format!("0xf1c7a58b{size:064x}");
        match model {
            OpStackFeeModel::FjordWithOperator => {
                let (data, operator) =
                    tokio::try_join!(read(upper_bound), read(format!("0x275aedd2{gas:064x}")),)?;
                data.checked_add(operator)
                    .ok_or_else(|| ApiError::Decode("rollup fee overflow".into()))
            }
            OpStackFeeModel::Fjord => read(upper_bound).await,
            OpStackFeeModel::Bedrock => {
                use sha3::Digest;
                let selector = hex::encode(&sha3::Keccak256::digest(b"getL1Fee(bytes)")[..4]);
                let padded = size.div_ceil(32) * 64;
                let data = format!(
                    "0x{selector}{:064x}{size:064x}{:0<padded$}",
                    32,
                    hex::encode(unsigned_transaction)
                );
                read(data).await
            }
        }
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<EvmBalance, ApiError> {
        let result = self
            .call("eth_getBalance", json!([address, "latest"]))
            .await?;
        let hex = result
            .as_str()
            .or_decode("eth_getBalance: expected string")?;
        let wei = parse_hex_u128(hex)?;
        Ok(EvmBalance {
            balance_wei: wei.to_string(),
        })
    }

    pub async fn fetch_nonce(&self, address: &str) -> Result<u64, ApiError> {
        let result = self
            .call("eth_getTransactionCount", json!([address, "pending"]))
            .await?;
        let hex = result
            .as_str()
            .or_decode("eth_getTransactionCount: expected string")?;
        parse_hex_u64(hex)
    }

    pub async fn fetch_fee_estimate(&self) -> Result<EvmFeeEstimate, ApiError> {
        let result = self
            .call("eth_feeHistory", json!([4, "latest", [25, 75]]))
            .await?;
        parse_fee_history(&result)
    }

    /// Fetch an ERC-20 `balanceOf(holder)` and normalize to display form.
    pub async fn fetch_erc20_balance(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<Erc20Balance, ApiError> {
        let raw = self.fetch_erc20_balance_of(contract, holder).await?;
        let metadata = self.fetch_erc20_metadata(contract).await?;
        let balance_display = crate::decimal::from_units(raw, u32::from(metadata.decimals));
        Ok(Erc20Balance {
            contract: contract.to_lowercase(),
            holder: holder.to_lowercase(),
            balance_raw: raw.to_string(),
            balance_display,
            decimals: metadata.decimals,
            symbol: metadata.symbol,
        })
    }

    /// `balanceOf(holder)` and `decimals()` for each contract, in input order.
    ///
    /// Every read rides in a JSON-RPC batch, a few contracts per request so a
    /// node's batch limit is not the wallet's token limit, and the requests
    /// run together. Each contract's answer is its own: one that reverts
    /// (a self-destructed token) does not take the others with it.
    ///
    /// A contract that holds something is then asked, through ERC-165,
    /// whether it is an NFT collection: an ERC-721 `balanceOf` is a count of
    /// tokens, never a balance to scale by decimals, so such a contract
    /// answers a refusal. A zero is the same either way and costs no call.
    pub async fn fetch_erc20_balances(
        &self,
        holder: &str,
        contracts: &[String],
    ) -> Vec<Result<(u128, u8), ApiError>> {
        let mut reads = self.fetch_erc20_balance_words(holder, contracts).await;
        let held: Vec<usize> = (0..reads.len())
            .filter(|&index| matches!(reads[index], Ok((raw, _)) if raw > 0))
            .collect();
        /// Contracts per batch: three calls each.
        const CONTRACTS_PER_BATCH: usize = 13;
        let checks = held.chunks(CONTRACTS_PER_BATCH).map(|chunk| {
            let requests = chunk
                .iter()
                .flat_map(|&index| nft_interface_calls(&contracts[index]))
                .collect();
            async move { (chunk, self.call_batch_each(requests).await) }
        });
        for (chunk, answers) in futures::future::join_all(checks).await {
            for (position, &index) in chunk.iter().enumerate() {
                match &answers {
                    Ok(answers) => {
                        if nft_claims(&answers[3 * position..3 * position + 3]) != (false, false) {
                            reads[index] = Err(not_fungible());
                        }
                    }
                    // Unchecked is unread: the balance waits for an answer.
                    Err(error) => reads[index] = Err(error.clone()),
                }
            }
        }
        reads
    }

    /// The balance and decimals words of [`Self::fetch_erc20_balances`].
    async fn fetch_erc20_balance_words(
        &self,
        holder: &str,
        contracts: &[String],
    ) -> Vec<Result<(u128, u8), ApiError>> {
        /// Contracts per batch: two calls each, well inside the batch limit
        /// public nodes enforce.
        const CONTRACTS_PER_BATCH: usize = 20;
        let balance_of = match encode_erc20_balance_of(holder) {
            Ok(data) => format!("0x{}", hex::encode(data)),
            Err(error) => return contracts.iter().map(|_| Err(error.clone())).collect(),
        };
        let decimals = format!("0x{}", hex::encode(SEL_DECIMALS));
        let batches = contracts.chunks(CONTRACTS_PER_BATCH).map(|chunk| {
            let requests = chunk
                .iter()
                .flat_map(|contract| {
                    [
                        (
                            "eth_call",
                            json!([{"to": contract, "data": balance_of}, "latest"]),
                        ),
                        (
                            "eth_call",
                            json!([{"to": contract, "data": decimals}, "latest"]),
                        ),
                    ]
                })
                .collect();
            async move {
                match self.call_batch_each(requests).await {
                    Ok(answers) => answers
                        .chunks(2)
                        .map(|pair| {
                            let word = |answer: &Result<Value, ApiError>| {
                                answer.clone().and_then(|value| {
                                    parse_hex_u128(
                                        value.as_str().or_decode("eth_call: expected string")?,
                                    )
                                })
                            };
                            let raw = word(&pair[0])?;
                            let decimals = crate::api::checked_token_decimals(word(&pair[1])?)?;
                            Ok((raw, decimals))
                        })
                        .collect::<Vec<_>>(),
                    Err(error) => chunk.iter().map(|_| Err(error.clone())).collect(),
                }
            }
        });
        futures::future::join_all(batches)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Raw `balanceOf` call — cheapest way to refresh a known-token balance.
    pub async fn fetch_erc20_balance_of(
        &self,
        contract: &str,
        holder: &str,
    ) -> Result<u128, ApiError> {
        let data = encode_erc20_balance_of(holder)?;
        let result = self
            .call(
                "eth_call",
                json!([
                    {
                        "to": contract,
                        "data": format!("0x{}", hex::encode(&data)),
                    },
                    "latest"
                ]),
            )
            .await?;
        let hex_str = result
            .as_str()
            .or_decode("eth_call balanceOf: expected string")?;
        parse_hex_u128(hex_str)
    }

    /// `eth_call` at the latest block: the returned bytes, or `None` when
    /// the call reverts.
    async fn call_or_revert(&self, to: &str, data: &[u8]) -> Result<Option<Vec<u8>>, ApiError> {
        match self
            .call(
                "eth_call",
                json!([{"to": to, "data": format!("0x{}", hex::encode(data))}, "latest"]),
            )
            .await
        {
            Ok(result) => Ok(Some(decode_hex(
                result.as_str().or_decode("eth_call: expected string")?,
            )?)),
            // A node's error object for a call is the call reverting.
            Err(ApiError::Rejected(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The NFT standard `contract` reports through ERC-165, or `None` for a
    /// contract that reports neither, or claims every interface at once.
    /// One that reports both is refused: which transfer it means is not
    /// known.
    pub(crate) async fn fetch_nft_standard(
        &self,
        contract: &str,
    ) -> Result<Option<NftStandard>, ApiError> {
        let answers = self.call_batch_each(nft_interface_calls(contract)).await?;
        match nft_claims(&answers) {
            (false, false) => Ok(None),
            (true, true) => Err(ApiError::rejected(
                "The contract reports both ERC-721 and ERC-1155",
            )),
            (true, false) => Ok(Some(NftStandard::Erc721)),
            (false, true) => Ok(Some(NftStandard::Erc1155)),
        }
    }

    /// The owner of an ERC-721 token, lowercase; `None` when the token does
    /// not exist (the call reverts).
    pub(crate) async fn fetch_erc721_owner(
        &self,
        contract: &str,
        token_id: &str,
    ) -> Result<Option<String>, ApiError> {
        let data = crate::api::evm_nft::owner_of_call(token_id)?;
        let Some(word) = self.call_or_revert(contract, &data).await? else {
            return Ok(None);
        };
        if word.len() != 32 || word[..12] != [0; 12] {
            return Err(ApiError::decode("ownerOf: not an address"));
        }
        Ok(Some(format!("0x{}", hex::encode(&word[12..]))))
    }

    /// How many of an ERC-1155 token id `owner` holds: a whole number, not
    /// an amount.
    pub(crate) async fn fetch_erc1155_balance(
        &self,
        contract: &str,
        owner: &str,
        token_id: &str,
    ) -> Result<num_bigint::BigUint, ApiError> {
        let data = crate::api::evm_nft::balance_of_call(owner, token_id)?;
        let word = self
            .call_or_revert(contract, &data)
            .await?
            .ok_or_else(|| ApiError::rejected("balanceOf reverted"))?;
        if word.len() != 32 {
            return Err(ApiError::decode("balanceOf: not a uint256"));
        }
        Ok(num_bigint::BigUint::from_bytes_be(&word))
    }

    /// A collection's `name()` and `symbol()`, each empty where the contract
    /// has none: both are optional in ERC-721 and absent from ERC-1155.
    pub(crate) async fn fetch_collection_label(
        &self,
        contract: &str,
    ) -> Result<(String, String), ApiError> {
        let answers = self
            .call_batch_each(
                [SEL_NAME, SEL_SYMBOL]
                    .into_iter()
                    .map(|selector| {
                        (
                            "eth_call",
                            json!([{"to": contract, "data": format!("0x{}", hex::encode(selector))}, "latest"]),
                        )
                    })
                    .collect(),
            )
            .await?;
        let text = |answer: &Result<Value, ApiError>| {
            answer
                .as_ref()
                .ok()
                .and_then(Value::as_str)
                .and_then(decode_abi_string_or_bytes32)
                .map(|text| text.trim().to_string())
                .unwrap_or_default()
        };
        Ok((text(&answers[0]), text(&answers[1])))
    }

    /// Resolve an ENS name to a checksummed Ethereum address via the ENS Ideas API.
    pub async fn resolve_ens(&self, name: &str) -> Result<Option<String>, ApiError> {
        let normalized = name.trim().to_lowercase();
        if normalized.is_empty() || !normalized.ends_with(".eth") || normalized.contains(' ') {
            return Ok(None);
        }
        let encoded = percent_encode(&normalized);
        let url = format!("https://api.ensideas.com/ens/resolve/{encoded}");
        let resp: Value = self
            .client
            .get_json(&url, RetryProfile::ChainRead)
            .await
            .map_err(|e| ApiError::Decode(format!("ENS resolve: {e}")))?;
        let address = match resp.get("address").and_then(|v| v.as_str()) {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => return Ok(None),
        };
        // Basic EVM address validation: 0x + 40 hex chars.
        let norm = address.trim().to_lowercase();
        if norm.len() == 42
            && norm.starts_with("0x")
            && norm[2..].chars().all(|c| c.is_ascii_hexdigit())
        {
            Ok(Some(norm))
        } else {
            Ok(None)
        }
    }

    /// Fetch a transaction receipt by hash. Returns `None` when the
    /// transaction is not yet mined (pending). Returns an error only on
    /// RPC failure.
    pub async fn fetch_receipt(&self, tx_hash: &str) -> Result<Option<EvmReceipt>, ApiError> {
        let result = self
            .call("eth_getTransactionReceipt", json!([tx_hash]))
            .await?;
        if result.is_null() {
            return Ok(None);
        }
        let block_number = result
            .get("blockNumber")
            .and_then(|v| v.as_str())
            .filter(|s| *s != "0x" && !s.is_empty())
            .map(parse_hex_u64)
            .transpose()?;
        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let gas_used = result
            .get("gasUsed")
            .and_then(|v| v.as_str())
            .map(parse_hex_u128)
            .transpose()?
            .map(|n| n.to_string());
        let effective_gas_price_wei = result
            .get("effectiveGasPrice")
            .and_then(|v| v.as_str())
            .map(parse_hex_u128)
            .transpose()?
            .map(|n| n.to_string());
        let is_confirmed = block_number.is_some();
        let is_failed = status.as_deref() == Some("0x0");
        let l1_fee_wei = result
            .get("l1Fee")
            .and_then(Value::as_str)
            .and_then(|value| parse_hex_u128(value).ok())
            .map(|value| value.to_string());
        let operator_fee_wei = if let (Some(block), Some(gas)) = (block_number, gas_used.as_deref())
        {
            if l1_fee_wei.is_some() {
                self.receipt_operator_fee(block, gas)
                    .await
                    .map(|fee| fee.to_string())
            } else {
                None
            }
        } else {
            None
        };
        Ok(Some(EvmReceipt {
            tx_hash: tx_hash.to_string(),
            block_number,
            status,
            gas_used,
            effective_gas_price_wei,
            l1_fee_wei,
            operator_fee_wei,
            is_confirmed,
            is_failed,
        }))
    }

    /// Query the deployed oracle at the mined block, so Isthmus, Jovian and
    /// later operator formulas are never inferred from today's deployment.
    async fn receipt_operator_fee(&self, block: u64, gas: &str) -> Option<u128> {
        use crate::registry::OpStackFeeModel;
        let chain = self.chain().ok()?;
        let model = chain.evm_rollup_fee_model()?;
        if model != OpStackFeeModel::FjordWithOperator {
            return Some(0);
        }
        let gas = gas.parse::<u64>().ok()?;
        let block_tag = format!("0x{block:x}");
        let fee = self
            .call(
                "eth_call",
                json!([{
                "to": "0x420000000000000000000000000000000000000F",
                "data": format!("0x275aedd2{gas:064x}")
            }, block_tag]),
            )
            .await
            .ok()
            .and_then(|value| {
                let bytes = decode_hex(value.as_str()?).ok()?;
                (bytes.len() == 32 && bytes[..16].iter().all(|byte| *byte == 0))
                    .then(|| u128::from_be_bytes(bytes[16..].try_into().expect("ABI word")))
            });
        if fee.is_some() {
            return fee;
        }
        // Before a verified activation timestamp there was no operator fee.
        // A failed oracle request after activation remains unknown.
        let activation = chain.evm_operator_fee_activation()?;
        let header = self
            .call("eth_getBlockByNumber", json!([block_tag, false]))
            .await
            .ok()?;
        if parse_hex_u64(header.get("number")?.as_str()?).ok()? != block {
            return None;
        }
        let timestamp = parse_hex_u64(header.get("timestamp")?.as_str()?).ok()?;
        (timestamp < activation).then_some(0)
    }

    /// Fetch the bytecode deployed at `address` (eth_getCode).
    pub async fn fetch_code(&self, address: &str) -> Result<String, ApiError> {
        let result = self.call("eth_getCode", json!([address, "latest"])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .or_decode("eth_getCode: expected string")
    }

    /// Fetch the nonce of an already-submitted transaction by hash.
    pub async fn fetch_tx_nonce(&self, tx_hash: &str) -> Result<u64, ApiError> {
        let result = self
            .call("eth_getTransactionByHash", json!([tx_hash]))
            .await?;
        let nonce_hex = result
            .get("nonce")
            .and_then(|v| v.as_str())
            .or_decode("eth_getTransactionByHash: missing nonce")?;
        parse_hex_u64(nonce_hex)
    }

    /// Fetch token metadata (symbol + decimals) in a single batch request.
    ///
    /// A contract that reports an NFT standard through ERC-165 is refused,
    /// whatever `decimals()` answers: its token ids and quantities are not a
    /// balance to scale, and every fungible read and send comes through here.
    pub async fn fetch_erc20_metadata(&self, contract: &str) -> Result<Erc20Metadata, ApiError> {
        let call = |data: Vec<u8>| {
            (
                "eth_call",
                json!([{"to": contract, "data": format!("0x{}", hex::encode(data))}, "latest"]),
            )
        };
        let mut requests = vec![call(SEL_DECIMALS.to_vec()), call(SEL_SYMBOL.to_vec())];
        requests.extend(nft_interface_calls(contract));
        let mut results = self.call_batch_each(requests).await?;
        if nft_claims(&results[2..]) != (false, false) {
            return Err(not_fungible());
        }
        results.truncate(2);
        let results = results
            .into_iter()
            .collect::<Result<Vec<Value>, ApiError>>()?;
        let decimals = crate::api::checked_token_decimals(parse_hex_u128(
            results[0]
                .as_str()
                .or_decode("eth_call decimals: expected string")?,
        )?)?;
        let symbol = results[1]
            .as_str()
            .and_then(decode_abi_string_or_bytes32)
            .unwrap_or_default();
        Ok(Erc20Metadata { symbol, decimals })
    }
}

impl EvmClient {
    /// What `owner` lets each `(token, spender)` pair move, live: one
    /// `allowance` call per pair, batched, each answering on its own.
    pub async fn fetch_erc20_allowances(
        &self,
        owner: &str,
        pairs: &[(String, String)],
    ) -> Result<Vec<Result<num_bigint::BigUint, ApiError>>, ApiError> {
        let word = |address: &str| {
            format!(
                "{:0>64}",
                address.trim_start_matches("0x").to_ascii_lowercase()
            )
        };
        let mut answers = Vec::with_capacity(pairs.len());
        for chunk in pairs.chunks(50) {
            let requests = chunk
                .iter()
                .map(|(token, spender)| {
                    (
                        "eth_call",
                        json!([{
                            "to": token,
                            "data": format!("0x{}{}{}", hex::encode(SEL_ALLOWANCE), word(owner), word(spender)),
                        }, "latest"]),
                    )
                })
                .collect();
            for answer in self.call_batch_each(requests).await? {
                answers.push(answer.and_then(|value| {
                    let bytes = decode_hex(value.as_str().or_decode("allowance: expected hex")?)?;
                    if bytes.len() != 32 {
                        return Err(ApiError::Decode("allowance: not one word".into()));
                    }
                    Ok(num_bigint::BigUint::from_bytes_be(&bytes))
                }));
            }
        }
        Ok(answers)
    }

    /// Each token's symbol and decimals, batched, `None` for a contract that
    /// does not answer as an ERC-20.
    pub async fn fetch_erc20_metadata_many(
        &self,
        contracts: &[String],
    ) -> Result<Vec<Option<Erc20Metadata>>, ApiError> {
        let mut answers = Vec::with_capacity(contracts.len());
        for chunk in contracts.chunks(25) {
            let requests = chunk
                .iter()
                .flat_map(|contract| {
                    [SEL_DECIMALS, SEL_SYMBOL].map(|selector| {
                        (
                            "eth_call",
                            json!([{"to": contract, "data": format!("0x{}", hex::encode(selector))}, "latest"]),
                        )
                    })
                })
                .collect();
            let replies = self.call_batch_each(requests).await?;
            for pair in replies.chunks(2) {
                let decimals = pair[0]
                    .as_ref()
                    .ok()
                    .and_then(|value| value.as_str())
                    .and_then(|hex| parse_hex_u128(hex).ok())
                    .and_then(|raw| crate::api::checked_token_decimals(raw).ok());
                let symbol = pair[1]
                    .as_ref()
                    .ok()
                    .and_then(|value| value.as_str())
                    .and_then(decode_abi_string_or_bytes32)
                    .unwrap_or_default();
                answers.push(decimals.map(|decimals| Erc20Metadata { symbol, decimals }));
            }
        }
        Ok(answers)
    }

    /// The ENS primary name `address` claims, shown only when that name
    /// resolves back to `address`: the reverse record alone is anyone's to
    /// set. Read from the registry on Ethereum, through this client's nodes.
    /// `None` where there is no name, it does not resolve back, or it is not
    /// already a normalized ASCII name.
    pub async fn fetch_ens_primary_name(&self, address: &str) -> Result<Option<String>, ApiError> {
        let address = address.trim_start_matches("0x").to_ascii_lowercase();
        let reverse = ens_namehash(&format!("{address}.addr.reverse"));
        let Some(resolver) = self.ens_resolver(&reverse).await? else {
            return Ok(None);
        };
        let name = self
            .call(
                "eth_call",
                json!([{"to": resolver, "data": format!("0x{}{}", hex::encode(SEL_ENS_NAME), hex::encode(reverse))}, "latest"]),
            )
            .await?;
        let Some(name) = name.as_str().and_then(decode_abi_string_or_bytes32) else {
            return Ok(None);
        };
        let normalized = name.len() <= 255
            && name.contains('.')
            && name.split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            });
        if !normalized {
            return Ok(None);
        }
        let forward = ens_namehash(&name);
        let Some(resolver) = self.ens_resolver(&forward).await? else {
            return Ok(None);
        };
        let resolved = self
            .call(
                "eth_call",
                json!([{"to": resolver, "data": format!("0x{}{}", hex::encode(SEL_ENS_ADDR), hex::encode(forward))}, "latest"]),
            )
            .await?;
        let resolved = decode_hex(resolved.as_str().or_decode("addr: expected hex")?)?;
        Ok((resolved.len() == 32 && hex::encode(&resolved[12..]) == address).then_some(name))
    }

    /// The resolver the ENS registry names for `node`, or `None` for none.
    async fn ens_resolver(&self, node: &[u8; 32]) -> Result<Option<String>, ApiError> {
        let answer = self
            .call(
                "eth_call",
                json!([{"to": ENS_REGISTRY, "data": format!("0x{}{}", hex::encode(SEL_ENS_RESOLVER), hex::encode(node))}, "latest"]),
            )
            .await?;
        let bytes = decode_hex(answer.as_str().or_decode("resolver: expected hex")?)?;
        if bytes.len() != 32 || bytes[12..].iter().all(|b| *b == 0) {
            return Ok(None);
        }
        Ok(Some(format!("0x{}", hex::encode(&bytes[12..]))))
    }
}

/// EIP-137's namehash: from the root, each label's hash folded in, last
/// label first.
pub(crate) fn ens_namehash(name: &str) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    name.rsplit('.')
        .filter(|label| !label.is_empty())
        .fold([0u8; 32], |node, label| {
            let mut hasher = Keccak256::new();
            hasher.update(node);
            hasher.update(Keccak256::digest(label.as_bytes()));
            hasher.finalize().into()
        })
}

// ── ERC-20 ABI helpers (shared with send.rs)

/// The `transfer(address,uint256)` selector as its hex spelling, for
/// [`is_erc20_transfer`] and the assembler in [`crate::send::ethereum`].
pub(crate) fn erc20_transfer_selector_hex() -> String {
    hex::encode(SEL_TRANSFER)
}

/// Whether this calldata is an ERC-20 `transfer(address,uint256)`.
///
/// The inverse of the assembler: a transfer is addressed *to the token
/// contract*, so a caller holding both the destination and the calldata can
/// name the token being moved from this alone, without being told.
pub(crate) fn is_erc20_transfer(data_hex: &str) -> bool {
    let body = data_hex.strip_prefix("0x").unwrap_or(data_hex);
    body.get(..8)
        .is_some_and(|selector| selector.eq_ignore_ascii_case(&erc20_transfer_selector_hex()))
}

/// Encode a `balanceOf(address)` call.
pub fn encode_erc20_balance_of(holder: &str) -> Result<Vec<u8>, ApiError> {
    let holder_bytes = decode_hex(holder)?;
    if holder_bytes.len() != 20 {
        return Err(ApiError::InvalidInput(format!(
            "invalid EVM holder length: {}",
            holder_bytes.len()
        )));
    }
    let mut out = Vec::with_capacity(4 + 32);
    out.extend_from_slice(&SEL_BALANCE_OF);
    out.extend_from_slice(&[0u8; 12]); // left-pad 20-byte address to 32 bytes
    out.extend_from_slice(&holder_bytes);
    Ok(out)
}

/// Decode an ABI-encoded `string` return value, or fall back to a
/// `bytes32`-style null-terminated ASCII name (MKR, DAI-era tokens).
pub fn decode_abi_string_or_bytes32(hex_str: &str) -> Option<String> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(stripped).ok()?;
    if bytes.is_empty() {
        return None;
    }

    // ABI `string` layout: offset (32) | length (32) | data
    if bytes.len() >= 64 {
        let offset = u64::from_be_bytes(bytes[24..32].try_into().ok()?) as usize;
        if offset == 32 && bytes.len() >= offset + 32 {
            let len_start = offset;
            let len_end = offset + 32;
            let len = u64::from_be_bytes(bytes[len_start + 24..len_end].try_into().ok()?) as usize;
            let data_start = len_end;
            let data_end = data_start.checked_add(len)?;
            if bytes.len() >= data_end {
                let slice = &bytes[data_start..data_end];
                if let Ok(s) = std::str::from_utf8(slice) {
                    let trimmed = s.trim_end_matches(char::from(0));
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }

    // Fallback: treat as bytes32, trim trailing null bytes.
    let trimmed: Vec<u8> = bytes
        .iter()
        .take(32)
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    let s = String::from_utf8(trimmed).ok()?;
    if s.is_empty() { None } else { Some(s) }
}

/// Percent-encode a string for use in a URL path component.
/// Only encodes characters that are not safe in a path segment.
fn percent_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                let byte = c as u8;
                format!("%{:02X}", byte).chars().collect()
            }
        })
        .collect()
}

impl EvmClient {
    /// Broadcast a pre-signed raw transaction hex (0x-prefixed).
    pub async fn broadcast_raw(&self, hex_tx: &str) -> Result<EvmSendResult, ApiError> {
        let result = self.call("eth_sendRawTransaction", json!([hex_tx])).await?;
        let txid = result
            .as_str()
            .or_decode("eth_sendRawTransaction: expected string")?
            .to_string();
        Ok(EvmSendResult {
            txid,
            nonce: 0,
            raw_tx_hex: hex_tx.to_string(),
            gas_limit: 0,
            max_fee_per_gas_wei: String::new(),
            max_priority_fee_per_gas_wei: String::new(),
        })
    }
}

#[cfg(test)]
mod fee_history_tests {
    use super::*;

    /// The reward array is per block, and each block's entry is one sample per
    /// requested percentile. The last block's 25th percentile is the one this
    /// reads.
    #[test]
    fn the_last_blocks_lowest_percentile_is_the_priority_fee() {
        let fees = parse_fee_history(&json!({
            "baseFeePerGas": ["0x3b9aca00", "0x77359400"],
            "reward": [["0x1", "0x2"], ["0x5f5e100", "0xbebc200"]],
        }))
        .unwrap();
        assert_eq!(fees.base_fee_wei, 0x7735_9400);
        assert_eq!(fees.priority_fee_wei, 0x5f5_e100);
        assert_eq!(fees.max_fee_per_gas_wei, 0x7735_9400 * 2 + 0x5f5_e100);
    }

    /// A block nobody transacted in reports no sample, and the honest priority
    /// fee for it is zero — `max_fee` still covers the base fee, so the
    /// transaction lands. Refusing this case left the send screen unable to
    /// quote a fee at all on a quiet L2 or testnet.
    #[test]
    fn an_empty_reward_sample_is_a_zero_priority_fee() {
        let fees = parse_fee_history(&json!({
            "baseFeePerGas": ["0x3b9aca00"],
            "reward": [[]],
        }))
        .unwrap();
        assert_eq!(fees.priority_fee_wei, 0);
        assert_eq!(fees.base_fee_wei, 0x3b9a_ca00);
        assert_eq!(fees.max_fee_per_gas_wei, 0x3b9a_ca00 * 2);
    }

    /// No `reward` array at all is a node that did not answer what was asked —
    /// the request always names percentiles — so there is nothing to infer.
    #[test]
    fn a_missing_reward_array_is_refused() {
        for body in [
            json!({"baseFeePerGas": ["0x3b9aca00"]}),
            json!({"baseFeePerGas": ["0x3b9aca00"], "reward": []}),
            json!({"baseFeePerGas": ["0x3b9aca00"], "reward": Value::Null}),
            json!({"baseFeePerGas": ["0x3b9aca00"], "reward": ["0x1"]}),
        ] {
            assert!(parse_fee_history(&body).is_err(), "{body}");
        }
    }

    /// A malformed response is still a refusal — the base fee is the estimate
    /// and there is nothing to fall back to.
    #[test]
    fn a_malformed_response_is_refused() {
        for body in [
            json!({}),
            json!({"baseFeePerGas": []}),
            json!({"baseFeePerGas": ["zz"]}),
            json!({"baseFeePerGas": [1]}),
            json!({"baseFeePerGas": ["0x1"], "reward": [["zz"]]}),
            json!({"baseFeePerGas": ["0x1"], "reward": [[7]]}),
            json!({"baseFeePerGas": ["0x1"], "reward": [["0x1"], "nope"]}),
        ] {
            assert!(parse_fee_history(&body).is_err(), "{body}");
        }
    }
}
