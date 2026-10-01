//! The BCH REST v2 adapter (`bch-api`, as `rest.bch.actorforth.org/v2`
//! serves it): Bitcoin Cash balances, UTXOs, history, transaction status and
//! broadcast. It quotes no fee rate. `api::utxo` decides which adapter
//! serves a request.
//!
//! Amounts in transaction bodies are in BCH, not satoshis — an input's
//! `valueSat` included, whatever its name says.

use crate::api::error::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::http::{HttpClient, RetryProfile, race};
use crate::api::utxo::{Utxo, UtxoBalance, UtxoHistoryEntry, UtxoStatus, UtxoTxStatus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddressDetails {
    balance_sat: u64,
    unconfirmed_balance_sat: i64,
    tx_appearances: u64,
    unconfirmed_tx_appearances: u64,
}

#[derive(Debug, Deserialize)]
struct AddressUtxos {
    utxos: Vec<RestUtxo>,
}

#[derive(Debug, Deserialize)]
struct RestUtxo {
    txid: String,
    vout: u32,
    satoshis: u64,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    confirmations: u64,
}

/// `/address/transactions`: `txs` pairs each transaction with an HTTP status,
/// and names the queried address in both of its forms.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddressTransactions {
    txs: Vec<(RestTx, Value)>,
    legacy_address: String,
    cash_address: String,
}

#[derive(Debug, Deserialize)]
struct RestTx {
    txid: String,
    /// Absent while the transaction is unconfirmed.
    blockheight: Option<i64>,
    blocktime: Option<u64>,
    confirmations: Option<u64>,
    fees: Option<f64>,
    #[serde(default)]
    vin: Vec<RestInput>,
    #[serde(default)]
    vout: Vec<RestOutput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestInput {
    cash_address: Option<String>,
    /// BCH, despite the name. `/transaction/details` calls it `value`.
    #[serde(alias = "value")]
    value_sat: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestOutput {
    value: f64,
    script_pub_key: Option<RestScript>,
}

#[derive(Debug, Deserialize)]
struct RestScript {
    #[serde(default)]
    addresses: Vec<String>,
}

/// A BCH amount as satoshis. Every BCH amount fits an `f64` exactly to the
/// satoshi, so rounding is exact; a negative or non-finite one is refused.
fn satoshis(bch: f64) -> Result<i64, ApiError> {
    let sats = (bch * 100_000_000.0).round();
    if !sats.is_finite() || sats < 0.0 || sats > 21e14 {
        return Err(ApiError::Decode(format!("BCH REST: invalid amount {bch}")));
    }
    Ok(sats as i64)
}

pub struct BchRestClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl BchRestClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }

    pub(crate) async fn has_activity(&self, address: &str) -> Result<bool, ApiError> {
        let details: AddressDetails = self.get(&format!("/address/details/{address}")).await?;
        Ok(details.tx_appearances > 0 || details.unconfirmed_tx_appearances > 0)
    }

    /// The confirmed balance and the mempool's net change to it.
    pub async fn fetch_balance(&self, address: &str) -> Result<UtxoBalance, ApiError> {
        let details: AddressDetails = self.get(&format!("/address/details/{address}")).await?;
        Ok(UtxoBalance {
            confirmed_sats: details.balance_sat,
            unconfirmed_sats: details.unconfirmed_balance_sat,
        })
    }

    /// Unspent outputs, the mempool's included.
    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<Utxo>, ApiError> {
        let list: AddressUtxos = self.get(&format!("/address/utxo/{address}")).await?;
        Ok(list
            .utxos
            .into_iter()
            .map(|u| {
                let confirmed = u.confirmations > 0 && u.height > 0;
                Utxo {
                    txid: u.txid,
                    vout: u.vout,
                    value: u.satoshis,
                    status: UtxoStatus {
                        confirmed,
                        block_height: confirmed.then_some(u.height),
                    },
                }
            })
            .collect())
    }

    /// The first page of the address's transactions. `net_sats` is what the
    /// address received less what its own inputs spent; the fee is the whole
    /// transaction's.
    pub async fn fetch_history(&self, address: &str) -> Result<Vec<UtxoHistoryEntry>, ApiError> {
        let page: AddressTransactions = self
            .get(&format!("/address/transactions/{address}"))
            .await?;
        let ours = |candidate: &str| {
            candidate == address
                || candidate == page.cash_address
                || candidate == page.legacy_address
        };
        let mut entries = Vec::new();
        for (tx, _) in &page.txs {
            let mut net = 0i64;
            for output in &tx.vout {
                if output
                    .script_pub_key
                    .as_ref()
                    .is_some_and(|s| s.addresses.iter().any(|a| ours(a)))
                {
                    net += satoshis(output.value)?;
                }
            }
            for input in &tx.vin {
                if input.cash_address.as_deref().is_some_and(ours) {
                    net -= satoshis(input.value_sat.ok_or_else(|| {
                        ApiError::decode(format!("BCH REST: input of {} has no value", tx.txid))
                    })?)?;
                }
            }
            if net == 0 {
                continue;
            }
            let block_height = tx
                .blockheight
                .filter(|height| *height > 0)
                .map(|height| height as u64);
            entries.push(UtxoHistoryEntry {
                txid: tx.txid.clone(),
                confirmed: block_height.is_some(),
                block_height,
                block_time: crate::api::time::history_time(
                    block_height.is_some(),
                    tx.blocktime,
                    &tx.txid,
                )?,
                net_sats: net,
                fee_sats: tx.fees.map(satoshis).transpose()?.map(|fee| fee as u64),
            });
        }
        Ok(entries)
    }

    pub async fn fetch_tx_status(&self, txid: &str) -> Result<UtxoTxStatus, ApiError> {
        let tx: RestTx = self.get(&format!("/transaction/details/{txid}")).await?;
        let block_height = tx
            .blockheight
            .filter(|height| *height > 0)
            .map(|height| height as u64);
        Ok(UtxoTxStatus {
            txid: tx.txid,
            confirmed: block_height.is_some(),
            block_height,
            block_time: tx.blocktime,
            confirmations: tx.confirmations,
        })
    }

    /// Submit a signed transaction; the node answers with its txid.
    pub async fn broadcast_raw_tx(&self, hex_tx: &str) -> Result<String, ApiError> {
        let body = json!({ "hexes": [hex_tx] });
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let url = format!(
                "{}/rawtransactions/sendRawTransaction",
                base.trim_end_matches('/')
            );
            async move {
                let response: Value = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                let txid = match &response {
                    Value::Array(ids) => ids.first().and_then(Value::as_str),
                    Value::String(id) => Some(id.as_str()),
                    _ => None,
                }
                .ok_or_else(|| {
                    ApiError::Rejected(format!("BCH REST broadcast refused: {response}"))
                })?;
                if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(ApiError::Decode(
                        "BCH REST returned an invalid transaction hash".into(),
                    ));
                }
                Ok(txid.to_lowercase())
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ME: &str = "1DXJfXMGEiUfT3MaY4fs1B2e1Pn2miCjfz";
    const ME_CASH: &str = "bitcoincash:qzy4ew7vguqwd0yj2gy0l4wgyfzjhmtl0ygswmmwej";
    const THEM: &str = "bitcoincash:pwdzcre3z37a59cwtx420xpwfl3lcfvj30cf7907reuh5txtqhrwqe4mzs4zm";

    async fn client(server: &MockServer) -> BchRestClient {
        BchRestClient::new(Arc::new(vec![server.uri()]))
    }

    /// Shapes as `rest.bch.actorforth.org/v2` returned them on 2026-09-29.
    #[tokio::test]
    async fn balances_and_utxos_are_in_satoshis() {
        let server = MockServer::start().await;
        Mock::given(path(format!("/address/details/{ME}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "balanceSat": 2800, "unconfirmedBalanceSat": -700, "balance": 0.000028,
                "txAppearances": 8, "unconfirmedTxAppearances": 4,
                "legacyAddress": ME, "cashAddress": ME_CASH
            })))
            .mount(&server)
            .await;
        Mock::given(path(format!("/address/utxo/{ME}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"utxos": [
                {"height": 0, "txid": "b6", "vout": 1, "satoshis": 45208289, "amount": 0.45208289, "confirmations": 0},
                {"height": 970705, "txid": "b7", "vout": 0, "satoshis": 1000, "amount": 0.00001, "confirmations": 3}
            ]})))
            .mount(&server)
            .await;
        let client = client(&server).await;
        let balance = client.fetch_balance(ME).await.unwrap();
        assert_eq!(
            (balance.confirmed_sats, balance.unconfirmed_sats),
            (2800, -700)
        );
        assert!(client.has_activity(ME).await.unwrap());
        let utxos = client.fetch_utxos(ME).await.unwrap();
        assert_eq!(utxos[0].value, 45_208_289);
        assert!(!utxos[0].status.confirmed);
        assert_eq!(utxos[1].status.block_height, Some(970_705));
    }

    /// An input's `valueSat` is in BCH, and the queried address appears in
    /// transactions under its cash address.
    #[tokio::test]
    async fn history_nets_the_addresses_own_inputs_and_outputs() {
        let server = MockServer::start().await;
        let tx = |txid: &str, height: Option<i64>, vin: Value, vout: Value| {
            let mut tx = json!({"txid": txid, "fees": 0.000008, "vin": vin, "vout": vout});
            if let Some(height) = height {
                tx["blockheight"] = json!(height);
                tx["blocktime"] = json!(1790685802);
                tx["confirmations"] = json!(1);
            }
            json!([tx, 200])
        };
        let output =
            |value: f64, to: &str| json!({"value": value, "scriptPubKey": {"addresses": [to]}});
        Mock::given(path(format!("/address/transactions/{ME}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "legacyAddress": ME, "cashAddress": ME_CASH, "currentPage": 0, "pagesTotal": 0,
                "txs": [
                    tx("receive", Some(970705),
                       json!([{"cashAddress": THEM, "valueSat": 0.11075701}]),
                       json!([output(0.11074201, THEM), output(0.00000700, ME_CASH)])),
                    tx("send", None,
                       json!([{"cashAddress": ME_CASH, "valueSat": 0.00000700}]),
                       json!([output(0.00000500, THEM)])),
                    tx("unrelated", Some(970704),
                       json!([{"cashAddress": THEM, "valueSat": 0.5}]),
                       json!([output(0.49, THEM)])),
                ]
            })))
            .mount(&server)
            .await;
        let history = client(&server).await.fetch_history(ME).await.unwrap();
        let rows: Vec<_> = history
            .iter()
            .map(|e| (e.txid.as_str(), e.net_sats, e.confirmed, e.fee_sats))
            .collect();
        assert_eq!(
            rows,
            [
                ("receive", 700, true, Some(800)),
                ("send", -700, false, Some(800))
            ]
        );
    }

    #[tokio::test]
    async fn a_broadcast_answers_with_the_nodes_txid_or_is_refused() {
        let server = MockServer::start().await;
        let txid = "ab".repeat(32);
        Mock::given(method("POST"))
            .and(path("/rawtransactions/sendRawTransaction"))
            .and(body_json(json!({"hexes": ["0200"]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([txid])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/rawtransactions/sendRawTransaction"))
            .and(body_json(json!({"hexes": ["0100"]})))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"error": "TX decode failed"})),
            )
            .mount(&server)
            .await;
        let client = client(&server).await;
        assert_eq!(client.broadcast_raw_tx("0200").await.unwrap(), txid);
        assert!(client.broadcast_raw_tx("0100").await.is_err());
    }
}
