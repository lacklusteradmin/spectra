//! The Koios REST adapter for Cardano: balances, UTXOs, history, the tip
//! slot and raw CBOR submission.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, RetryProfile, race};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoBalance {
    /// Lovelace (1 ADA = 1_000_000 lovelace).
    pub lovelace: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoUtxo {
    pub tx_hash: String,
    pub tx_index: u32,
    pub lovelace: u64,
    pub assets: Vec<CardanoAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoAsset {
    pub policy_id: String,
    pub asset_name: String,
    pub quantity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoHistoryEntry {
    pub txid: String,
    pub block: String,
    pub block_time: u64,
    pub is_incoming: bool,
    pub amount_lovelace: i64,
    pub fee_lovelace: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoSendResult {
    pub txid: String,
    /// CBOR hex of the signed transaction — stored for rebroadcast.
    pub cbor_hex: String,
}

// ── Koios response types (shared within the chain module)

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosAddressInfo {
    pub(crate) balance: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosUtxo {
    pub(crate) tx_hash: String,
    pub(crate) tx_index: u32,
    pub(crate) value: String,
    pub(crate) asset_list: Vec<CardanoAsset>,
    #[serde(default)]
    pub(crate) is_spent: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosTxRef {
    pub(crate) tx_hash: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosTxInfo {
    pub(crate) tx_hash: String,
    #[serde(default)]
    pub(crate) block_height: u64,
    #[serde(default)]
    pub(crate) tx_timestamp: Option<u64>,
    #[serde(default)]
    pub(crate) fee: String,
    #[serde(default)]
    pub(crate) inputs: Vec<KoiosTxIo>,
    #[serde(default)]
    pub(crate) outputs: Vec<KoiosTxIo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosTxIo {
    pub(crate) payment_addr: KoiosPaymentAddr,
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosPaymentAddr {
    #[serde(default)]
    pub(crate) bech32: String,
}

/// Each transaction's net effect on `address`: what its outputs paid the
/// address less what its inputs spent from it.
fn cardano_history_from_transactions(
    txs: Vec<KoiosTxInfo>,
    address: &str,
) -> Result<Vec<CardanoHistoryEntry>, ApiError> {
    let paid = |ios: &[KoiosTxIo]| -> i128 {
        ios.iter()
            .filter(|io| io.payment_addr.bech32 == address)
            .map(|io| io.value.parse::<i128>().unwrap_or(0))
            .sum()
    };
    // Koios lists only transactions already in a block.
    let mut entries = Vec::new();
    for tx in txs {
        let net = paid(&tx.outputs) - paid(&tx.inputs);
        let Ok(amount_lovelace) = i64::try_from(net) else {
            continue;
        };
        if net == 0 {
            continue;
        }
        entries.push(CardanoHistoryEntry {
            block_time: crate::api::time::confirmed_history_time(tx.tx_timestamp, &tx.tx_hash)?,
            txid: tx.tx_hash,
            block: tx.block_height.to_string(),
            is_incoming: net > 0,
            amount_lovelace,
            fee_lovelace: tx.fee.parse().unwrap_or(0),
        });
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.block_time));
    Ok(entries)
}

// ── Client

pub struct KoiosClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl KoiosClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    pub(crate) async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let path = path.to_string();
        let body_val = serde_json::to_value(body)?;
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            let body_val = body_val.clone();
            async move {
                client
                    .post_json(&url, &body_val, RetryProfile::ChainRead)
                    .await
            }
        })
        .await
    }
}

impl KoiosClient {
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        crate::api::transaction_status::validate_hex_hash(hash)?;
        let rows: Vec<serde_json::Value> = self
            .post("/tx_info", &serde_json::json!({"_tx_hashes":[hash]}))
            .await?;
        cardano_transaction_status(&rows, hash)
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<CardanoBalance, ApiError> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(rename = "_addresses")]
            addresses: &'a [&'a str],
        }
        let resp: Vec<KoiosAddressInfo> = self
            .post(
                "/address_info",
                &Req {
                    addresses: &[address],
                },
            )
            .await?;
        let lovelace: u64 = resp
            .into_iter()
            .next()
            .and_then(|r| r.balance.parse().ok())
            .unwrap_or(0);
        Ok(CardanoBalance { lovelace })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<CardanoUtxo>, ApiError> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(rename = "_addresses")]
            addresses: &'a [&'a str],
            #[serde(rename = "_extended")]
            extended: bool,
        }
        let utxos: Vec<KoiosUtxo> = self
            .post(
                "/address_utxos",
                &Req {
                    addresses: &[address],
                    extended: true,
                },
            )
            .await?;
        utxos
            .into_iter()
            .filter(|u| !u.is_spent)
            .map(|u| {
                Ok(CardanoUtxo {
                    tx_hash: u.tx_hash,
                    tx_index: u.tx_index,
                    lovelace: u.value.parse().map_err(ApiError::decode)?,
                    assets: u.asset_list,
                })
            })
            .collect::<Result<Vec<_>, ApiError>>()
    }

    /// ADA-only builders cannot consume an input carrying native assets.
    /// Keep those outputs untouched; an omitted asset list fails decoding.
    pub async fn fetch_ada_utxos(&self, address: &str) -> Result<Vec<CardanoUtxo>, ApiError> {
        Ok(self
            .fetch_utxos(address)
            .await?
            .into_iter()
            .filter(|u| u.assets.is_empty())
            .collect())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<CardanoHistoryEntry>, ApiError> {
        Ok(self.fetch_history_page(address, None).await?.items)
    }

    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<CardanoHistoryEntry>, ApiError> {
        #[derive(Serialize)]
        struct AddrReq<'a> {
            #[serde(rename = "_addresses")]
            addresses: &'a [&'a str],
        }
        #[derive(Serialize)]
        struct TxReq {
            #[serde(rename = "_tx_hashes")]
            tx_hashes: Vec<String>,
            #[serde(rename = "_inputs")]
            inputs: bool,
        }

        let number = crate::api::history_page::page_number(cursor)?;
        let offset = u64::from(number - 1) * 20;
        let tx_refs: Vec<KoiosTxRef> = self
            .post(
                &format!(
                    "/address_txs?order=block_height.desc,tx_hash.desc&limit=20&offset={offset}"
                ),
                &AddrReq {
                    addresses: &[address],
                },
            )
            .await?;

        let next_cursor = (tx_refs.len() == 20).then(|| (number + 1).to_string());
        let hashes: Vec<String> = tx_refs.iter().map(|r| r.tx_hash.clone()).collect();
        if hashes.is_empty() {
            return Ok(crate::api::HistoryPage {
                items: vec![],
                next_cursor: None,
            });
        }

        let tx_infos: Vec<KoiosTxInfo> = self
            .post(
                "/tx_info",
                &TxReq {
                    tx_hashes: hashes,
                    inputs: true,
                },
            )
            .await?;
        Ok(crate::api::HistoryPage {
            items: cardano_history_from_transactions(tx_infos, address)?,
            next_cursor,
        })
    }

    /// Fetch current slot from the latest block.
    pub async fn fetch_latest_slot(&self) -> Result<u64, ApiError> {
        #[derive(Deserialize)]
        struct Tip {
            abs_slot: u64,
        }
        let tips: Vec<Tip> = self.get("/tip").await?;
        tips.into_iter()
            .next()
            .map(|t| t.abs_slot)
            .or_decode("tip: empty response")
    }
}

fn cardano_transaction_status(
    rows: &[serde_json::Value],
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    let Some(row) = rows.first() else {
        return Ok(TransactionStatus::Pending);
    };
    if rows.len() != 1
        || !row["tx_hash"]
            .as_str()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(hash))
    {
        return Err(ApiError::decode(
            "Cardano status: transaction hash mismatch",
        ));
    }
    Ok(TransactionStatus::Confirmed {
        succeeded: row["valid_contract"]
            .as_bool()
            .or_decode("Cardano status: missing contract validation result")?,
        block: Some(
            row["block_height"]
                .as_u64()
                .or_decode("Cardano status: missing block height")?,
        ),
    })
}

impl KoiosClient {
    /// Submit a CBOR-encoded signed transaction.
    pub async fn submit_tx(&self, cbor_hex: &str) -> Result<CardanoSendResult, ApiError> {
        let cbor_hex_owned = cbor_hex.to_string();
        let cbor_bytes = hex::decode(cbor_hex)
            .map_err(|e| ApiError::InvalidInput(format!("hex decode: {e}")))?;
        race(&self.endpoints, |base| {
            let cbor_bytes = cbor_bytes.clone();
            let cbor_hex = cbor_hex_owned.clone();
            let url = format!("{}/submittx", base.trim_end_matches('/'));
            async move {
                // Koios submit-api accepts raw CBOR and returns a JSON transaction hash.
                let (status, body) = HttpClient::shared()
                    .post_bytes(
                        &url,
                        "application/cbor",
                        cbor_bytes,
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                if status != 202 {
                    return Err(ApiError::decode(format!(
                        "Koios submission: expected HTTP 202, received {status}"
                    )));
                }
                let txid: String = serde_json::from_slice(&body)
                    .map_err(|e| ApiError::Decode(format!("Koios transaction id: {e}")))?;
                if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(ApiError::Decode(
                        "Koios submission returned an invalid transaction id".into(),
                    ));
                }
                Ok(CardanoSendResult { txid, cbor_hex })
            }
        })
        .await
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "addr1qx2kd28nq8ac5prwg32hhvudlwggpgfp8utlyqxu6wqgz62f79qsdmm5dsknt9ecr5w468r9ey0fxwkdrwh08ly3tu9sy0f4qd";
    const THEM: &str = "addr1q8zup8m9ue3p98kxlxl9q8rnyan8hw3ul282tsl9s326dfj088lvedv4zckcj24arcpasr0gua4c5gq4zw2rpcpjk2lq8cmd9l";

    /// Shape of Koios `tx_info` with `_inputs`.
    #[test]
    fn amounts_are_the_addresses_net_change() {
        let txs: Vec<KoiosTxInfo> = serde_json::from_value(serde_json::json!([{
            "tx_hash": "send", "block_height": 2, "tx_timestamp": 200, "fee": "170000",
            "inputs": [{"payment_addr": {"bech32": ME}, "value": "10000000"}],
            "outputs": [
                {"payment_addr": {"bech32": THEM}, "value": "3000000"},
                {"payment_addr": {"bech32": ME}, "value": "6830000"}
            ]
        }, {
            "tx_hash": "receive", "block_height": 1, "tx_timestamp": 100, "fee": "170000",
            "inputs": [{"payment_addr": {"bech32": THEM}, "value": "9000000"}],
            "outputs": [
                {"payment_addr": {"bech32": ME}, "value": "2000000"},
                {"payment_addr": {"bech32": THEM}, "value": "6830000"}
            ]
        }]))
        .unwrap();
        let entries = cardano_history_from_transactions(txs, ME).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].txid, "send");
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_lovelace, -3_170_000, "net of the change");
        assert_eq!(entries[1].txid, "receive");
        assert!(entries[1].is_incoming);
        assert_eq!(entries[1].amount_lovelace, 2_000_000);
    }
}

#[cfg(test)]
mod keyless_submission_tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_bytes, header, method, path},
    };

    #[tokio::test]
    async fn ada_inputs_exclude_native_assets_and_require_complete_asset_lists() {
        let server = MockServer::start().await;
        let client = KoiosClient::new(Arc::new(vec![server.uri()]));
        Mock::given(method("POST"))
            .and(path("/address_utxos"))
            .and(wiremock::matchers::body_json(serde_json::json!({"_addresses":["mixed"],"_extended":true})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"tx_hash":"tokens","tx_index":0,"value":"10000000","is_spent":false,"asset_list":[{"policy_id":"ab".repeat(28),"asset_name":"cafe","quantity":"2"}]},
                {"tx_hash":"ada","tx_index":1,"value":"5000000","is_spent":false,"asset_list":[]},
                {"tx_hash":"spent","tx_index":0,"value":"1000000","is_spent":true,"asset_list":[]}
            ])))
            .mount(&server).await;
        let inputs = client.fetch_ada_utxos("mixed").await.unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].tx_hash, "ada");
        assert_eq!(inputs[0].lovelace, 5_000_000);
        assert!(inputs[0].assets.is_empty());
        let mixed = client.fetch_utxos("mixed").await.unwrap();
        assert_eq!(mixed[0].assets[0].quantity, "2");
        Mock::given(method("POST"))
            .and(path("/address_utxos"))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"_addresses":["incomplete"],"_extended":true}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"tx_hash":"unknown-assets","tx_index":0,"value":"10000000","is_spent":false}
            ])))
            .mount(&server)
            .await;
        assert!(client.fetch_ada_utxos("incomplete").await.is_err());
    }

    #[tokio::test]
    async fn koios_receives_raw_cbor_without_credentials_and_requires_a_transaction_id() {
        let server = MockServer::start().await;
        let client = KoiosClient::new(Arc::new(vec![server.uri()]));
        let txid = "ab".repeat(32);
        Mock::given(method("POST"))
            .and(path("/submittx"))
            .and(header("content-type", "application/cbor"))
            .and(body_bytes(vec![0x81, 0x00]))
            .respond_with(ResponseTemplate::new(202).set_body_json(&txid))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(client.submit_tx("8100").await.unwrap().txid, txid);
        let requests = server.received_requests().await.unwrap();
        assert!(!requests[0].headers.contains_key("authorization"));
        assert!(!requests[0].headers.contains_key("project_id"));
        assert!(requests[0].url.query().is_none());
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(202).set_body_json(""))
            .mount(&server)
            .await;
        assert!(
            client
                .submit_tx("8100")
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid transaction id")
        );
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("invalid transaction"))
            .mount(&server)
            .await;
        assert!(client.submit_tx("8100").await.is_err());
    }
}

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use serde_json::json;

    #[test]
    fn exact_transaction_contract_result_controls_confirmation() {
        assert_eq!(
            cardano_transaction_status(&[], "h").unwrap(),
            TransactionStatus::Pending
        );
        let rows = vec![json!({"tx_hash":"h","block_height":100,"valid_contract":false})];
        assert_eq!(
            cardano_transaction_status(&rows, "h").unwrap(),
            TransactionStatus::Confirmed {
                succeeded: false,
                block: Some(100)
            }
        );
        assert!(cardano_transaction_status(&rows, "other").is_err());
        assert!(
            cardano_transaction_status(&[json!({"tx_hash":"h","block_height":100})], "h").is_err()
        );
    }
}
