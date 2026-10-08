//! The Koios REST adapter for Cardano: balances, UTXOs, history, the tip
//! slot and raw CBOR submission.

use crate::api::cardano_asset::CardanoAssetId;
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
    /// A native asset's `policy.name`; absent for ADA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// A native asset's amount at its decimals, an exact decimal; absent for
    /// ADA.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_display: Option<String>,
}

/// What a transaction's fee and outputs depend on, from the current epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CardanoProtocolParams {
    /// Lovelace per byte of the signed transaction, and the fixed part.
    pub fee_per_byte: u64,
    pub fee_fixed: u64,
    /// Lovelace an output must hold per byte it takes, plus 160.
    pub coins_per_utxo_byte: u64,
    pub max_tx_size: u64,
    /// The most bytes one output's value may take.
    pub max_value_size: u64,
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
    #[serde(default)]
    pub(crate) asset_list: Vec<CardanoAsset>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KoiosPaymentAddr {
    #[serde(default)]
    pub(crate) bech32: String,
}

/// Each transaction's net effect on `address`: what its outputs paid the
/// address less what its inputs spent from it, in ADA and in each native
/// asset, every asset its own row at its decimals.
fn cardano_history_from_transactions(
    txs: Vec<KoiosTxInfo>,
    address: &str,
    decimals: &std::collections::HashMap<CardanoAssetId, u8>,
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
        let mut assets: std::collections::BTreeMap<CardanoAssetId, i128> = Default::default();
        for (ios, sign) in [(&tx.outputs, 1i128), (&tx.inputs, -1)] {
            for io in ios.iter().filter(|io| io.payment_addr.bech32 == address) {
                for asset in &io.asset_list {
                    let quantity: i128 = asset
                        .quantity
                        .parse()
                        .map_err(|_| ApiError::decode("Koios: invalid asset quantity"))?;
                    *assets.entry(asset.id()?).or_default() += sign * quantity;
                }
            }
        }
        let block_time = crate::api::time::confirmed_history_time(tx.tx_timestamp, &tx.tx_hash)?;
        let row = |amount_lovelace: i64, is_incoming: bool| CardanoHistoryEntry {
            block_time,
            txid: tx.tx_hash.clone(),
            block: tx.block_height.to_string(),
            is_incoming,
            amount_lovelace,
            fee_lovelace: tx.fee.parse().unwrap_or(0),
            contract: None,
            amount_display: None,
        };
        if let Ok(amount_lovelace) = i64::try_from(net)
            && net != 0
        {
            entries.push(row(amount_lovelace, net > 0));
        }
        for (asset, change) in assets.into_iter().filter(|(_, change)| *change != 0) {
            entries.push(CardanoHistoryEntry {
                amount_display: Some(crate::decimal::from_units(
                    change.unsigned_abs(),
                    u32::from(decimals.get(&asset).copied().unwrap_or(0)),
                )),
                contract: Some(asset.identifier()),
                ..row(0, change > 0)
            });
        }
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.block_time));
    Ok(entries)
}

impl CardanoAsset {
    pub(crate) fn id(&self) -> Result<CardanoAssetId, ApiError> {
        CardanoAssetId::new(&self.policy_id, &self.asset_name)
            .map_err(|_| ApiError::decode("Koios: invalid native asset"))
    }

    pub(crate) fn amount(&self) -> Result<u64, ApiError> {
        self.quantity
            .parse()
            .map_err(|_| ApiError::decode("Koios: invalid asset quantity"))
    }
}

/// A native asset's decimals: its CIP-68 fungible token datum's, else the
/// token registry's, else none.
fn asset_decimals(info: &serde_json::Value) -> Result<u8, ApiError> {
    let cip68 = info
        .pointer("/cip68_metadata/333/fields/0/map")
        .and_then(serde_json::Value::as_array)
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry
                    .pointer("/k/bytes")
                    .and_then(serde_json::Value::as_str)
                    == Some("646563696d616c73")
            })
        })
        .map(|entry| {
            entry
                .pointer("/v/int")
                .and_then(serde_json::Value::as_u64)
                .or_decode("Koios: invalid CIP-68 decimals")
        })
        .transpose()?;
    let registry = match info.pointer("/token_registry_metadata/decimals") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .or_decode("Koios: invalid registry decimals")?,
        ),
    };
    let places = cip68.or(registry).unwrap_or(0);
    u8::try_from(places)
        .ok()
        .filter(|places| *places <= 38)
        .or_decode("Koios: asset decimals out of range")
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

    /// Refuse a node whose genesis is another network's.
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let genesis: Vec<serde_json::Value> = self.get("/genesis").await?;
        let magic = genesis
            .first()
            .and_then(|row| row.get("networkmagic"))
            .and_then(|magic| {
                magic
                    .as_u64()
                    .or_else(|| magic.as_str().and_then(|text| text.parse().ok()))
            })
            .or_decode("Koios genesis: missing network magic")?;
        if magic != chain.cardano_network_magic().map_err(ApiError::invalid)? {
            return Err(ApiError::invalid("Koios endpoint is on the wrong network"));
        }
        Ok(())
    }

    /// The latest epoch's fee, output and size parameters.
    pub(crate) async fn fetch_protocol_params(&self) -> Result<CardanoProtocolParams, ApiError> {
        let rows: Vec<serde_json::Value> = self
            .get("/epoch_params?order=epoch_no.desc&limit=1")
            .await?;
        let params = rows.first().or_decode("Koios: no epoch parameters")?;
        // Koios writes some of these as numbers and some as strings.
        let field = |name: &str| {
            let value = &params[name];
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                .or_decode("Koios: missing protocol parameter")
        };
        Ok(CardanoProtocolParams {
            fee_per_byte: field("min_fee_a")?,
            fee_fixed: field("min_fee_b")?,
            coins_per_utxo_byte: field("coins_per_utxo_size")?,
            max_tx_size: field("max_tx_size")?,
            max_value_size: field("max_val_size")?,
        })
    }

    /// Each asset's decimals, read in one request.
    pub(crate) async fn fetch_asset_decimals(
        &self,
        assets: &[CardanoAssetId],
    ) -> Result<std::collections::HashMap<CardanoAssetId, u8>, ApiError> {
        if assets.is_empty() {
            return Ok(Default::default());
        }
        let list: Vec<[String; 2]> = assets
            .iter()
            .map(|asset| [hex::encode(asset.policy), hex::encode(&asset.name)])
            .collect();
        let rows: Vec<serde_json::Value> = self
            .post("/asset_info", &serde_json::json!({"_asset_list": list}))
            .await?;
        let mut decimals = std::collections::HashMap::new();
        for row in rows {
            let id = CardanoAssetId::new(
                row["policy_id"]
                    .as_str()
                    .or_decode("asset_info: missing policy")?,
                row["asset_name"].as_str().unwrap_or_default(),
            )
            .map_err(|_| ApiError::decode("asset_info: invalid asset"))?;
            decimals.insert(id, asset_decimals(&row)?);
        }
        // An asset the node does not know has no metadata, so no decimals.
        for asset in assets {
            decimals.entry(asset.clone()).or_insert(0);
        }
        Ok(decimals)
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
            #[serde(rename = "_assets")]
            assets: bool,
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
                    assets: true,
                },
            )
            .await?;
        let mut assets = std::collections::BTreeSet::new();
        for io in tx_infos
            .iter()
            .flat_map(|tx| tx.inputs.iter().chain(&tx.outputs))
            .filter(|io| io.payment_addr.bech32 == address)
        {
            for asset in &io.asset_list {
                assets.insert(asset.id()?);
            }
        }
        let decimals = self
            .fetch_asset_decimals(&assets.into_iter().collect::<Vec<_>>())
            .await?;
        Ok(crate::api::HistoryPage {
            items: cardano_history_from_transactions(tx_infos, address, &decimals)?,
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
        let entries = cardano_history_from_transactions(txs, ME, &Default::default()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].txid, "send");
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_lovelace, -3_170_000, "net of the change");
        assert_eq!(entries[1].txid, "receive");
        assert!(entries[1].is_incoming);
        assert_eq!(entries[1].amount_lovelace, 2_000_000);
    }

    /// Each native asset is its own row, net of what came back as change,
    /// at the asset's decimals.
    #[test]
    fn native_assets_are_their_own_rows() {
        let asset = |name: &str, quantity: &str| serde_json::json!({"policy_id": "aa".repeat(28), "asset_name": name, "quantity": quantity});
        let txs: Vec<KoiosTxInfo> = serde_json::from_value(serde_json::json!([{
            "tx_hash": "tokens", "block_height": 3, "tx_timestamp": 300, "fee": "180000",
            "inputs": [{"payment_addr": {"bech32": ME}, "value": "5000000",
                        "asset_list": [asset("0102", "100"), asset("", "7")]}],
            "outputs": [
                {"payment_addr": {"bech32": THEM}, "value": "1200000", "asset_list": [asset("0102", "40")]},
                {"payment_addr": {"bech32": ME}, "value": "3620000",
                 "asset_list": [asset("0102", "60"), asset("", "7")]}
            ]
        }]))
        .unwrap();
        let token = CardanoAssetId::new(&"aa".repeat(28), "0102").unwrap();
        let entries =
            cardano_history_from_transactions(txs, ME, &[(token.clone(), 2)].into()).unwrap();
        // The ADA row, and one row for the asset that left: 0.40 at two places.
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].amount_lovelace, -1_380_000);
        assert_eq!(
            entries[1].contract.as_deref(),
            Some(token.identifier().as_str())
        );
        assert_eq!(entries[1].amount_display.as_deref(), Some("0.4"));
        assert!(!entries[1].is_incoming);
    }

    #[test]
    fn asset_decimals_prefer_cip_68_then_the_registry() {
        let cip68 = serde_json::json!({"cip68_metadata": {"333": {"fields": [{"map": [
            {"k": {"bytes": "646563696d616c73"}, "v": {"int": 6}}]}, {"int": 1}], "constructor": 0}},
            "token_registry_metadata": {"decimals": 2}});
        assert_eq!(asset_decimals(&cip68).unwrap(), 6);
        let registry =
            serde_json::json!({"cip68_metadata": null, "token_registry_metadata": {"decimals": 2}});
        assert_eq!(asset_decimals(&registry).unwrap(), 2);
        assert_eq!(
            asset_decimals(&serde_json::json!({"token_registry_metadata": null})).unwrap(),
            0
        );
        assert!(
            asset_decimals(&serde_json::json!({"token_registry_metadata": {"decimals": 99}}))
                .is_err()
        );
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
    async fn utxos_carry_their_assets_and_require_complete_asset_lists() {
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
        let mixed = client.fetch_utxos("mixed").await.unwrap();
        assert_eq!(mixed.len(), 2, "a spent output is not one to spend");
        assert_eq!(mixed[0].assets[0].quantity, "2");
        assert!(mixed[1].assets.is_empty());
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
        assert!(client.fetch_utxos("incomplete").await.is_err());
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
