//! Cardano chain client.
//!
//! Uses the Koios REST API (api.koios.rest/api/v1) for balance,
//! history, UTXOs, and protocol params.
//! Cardano transactions are encoded in CBOR (cardano-multiplatform-lib
//! is too heavy; we use a minimal handwritten CBOR encoder for simple
//! ADA-only transfers).
//! Signing uses Ed25519 (ed25519-dalek).

use serde::{Deserialize, Serialize};

use crate::fetch::http::{HttpClient, RetryProfile, with_fallback};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoBalance {
    /// Lovelace (1 ADA = 1_000_000 lovelace).
    pub lovelace: u64,
    pub ada_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardanoUtxo {
    pub tx_hash: String,
    pub tx_index: u32,
    pub lovelace: u64,
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
) -> Result<Vec<CardanoHistoryEntry>, String> {
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
            block_time: super::confirmed_history_time(tx.tx_timestamp, &tx.tx_hash)?,
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

pub struct CardanoClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl CardanoClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, String> {
        self.client.get_path(&self.endpoints, path).await
    }

    pub(crate) async fn post<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, String> {
        let path = path.to_string();
        let body_val = serde_json::to_value(body).map_err(|e| e.to_string())?;
        with_fallback(&self.endpoints, |base| {
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

impl CardanoClient {
    pub async fn fetch_balance(&self, address: &str) -> Result<CardanoBalance, String> {
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
        Ok(CardanoBalance {
            lovelace,
            ada_display: format_ada(lovelace),
        })
    }

    pub async fn fetch_utxos(&self, address: &str) -> Result<Vec<CardanoUtxo>, String> {
        #[derive(Serialize)]
        struct Req<'a> {
            #[serde(rename = "_addresses")]
            addresses: &'a [&'a str],
        }
        let utxos: Vec<KoiosUtxo> = self
            .post(
                "/address_utxos",
                &Req {
                    addresses: &[address],
                },
            )
            .await?;
        Ok(utxos
            .into_iter()
            .filter(|u| !u.is_spent)
            .map(|u| CardanoUtxo {
                tx_hash: u.tx_hash,
                tx_index: u.tx_index,
                lovelace: u.value.parse().unwrap_or(0),
            })
            .collect())
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<CardanoHistoryEntry>, String> {
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

        let tx_refs: Vec<KoiosTxRef> = self
            .post(
                "/address_txs",
                &AddrReq {
                    addresses: &[address],
                },
            )
            .await?;

        let hashes: Vec<String> = tx_refs.iter().take(20).map(|r| r.tx_hash.clone()).collect();
        if hashes.is_empty() {
            return Ok(vec![]);
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
        cardano_history_from_transactions(tx_infos, address)
    }

    /// Fetch current slot from the latest block.
    pub async fn fetch_latest_slot(&self) -> Result<u64, String> {
        #[derive(Deserialize)]
        struct Tip {
            abs_slot: u64,
        }
        let tips: Vec<Tip> = self.get("/tip").await?;
        tips.into_iter()
            .next()
            .map(|t| t.abs_slot)
            .ok_or_else(|| "tip: empty response".to_string())
    }
}

fn format_ada(lovelace: u64) -> String {
    let whole = lovelace / 1_000_000;
    let frac = lovelace % 1_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:06}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    format!("{}.{}", whole, trimmed)
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
