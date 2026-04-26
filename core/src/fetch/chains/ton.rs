//! TON (The Open Network) chain client.
//!
//! Uses the TON Center REST API (toncenter.com/api/v2).
//! Signing uses Ed25519 (ed25519-dalek).
//! TON cells are complex; for transfers we use the tonlib-compatible
//! approach of sending via the `walletv4r2` contract message format.

use serde::{Deserialize, Serialize};

use crate::http::{with_fallback, HttpClient, RetryProfile};



// ----------------------------------------------------------------
// Public result types
// ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonBalance {
    /// Nanotons (1 TON = 1_000_000_000 nanotons).
    pub nanotons: u64,
    pub ton_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonHistoryEntry {
    pub txid: String,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_nanotons: u64,
    pub fee_nanotons: u64,
    pub is_incoming: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonSendResult {
    pub message_hash: String,
    /// Base64-encoded BOC — stored for rebroadcast.
    pub boc_b64: String,
}

/// One jetton (token) balance entry returned by the v3 API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonJettonBalance {
    /// Jetton master contract address (matches the tracked-token `contract` field).
    pub master_address: String,
    /// Jetton wallet contract address (holder's personal wallet for this token).
    pub wallet_address: String,
    /// Raw balance in the token's smallest unit.
    pub balance_raw: u128,
}

// ----------------------------------------------------------------
// Client
// ----------------------------------------------------------------

pub struct TonClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) v3_endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) api_key: Option<String>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl TonClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>, api_key: Option<String>) -> Self {
        Self {
            endpoints,
            v3_endpoints: std::sync::Arc::new(Vec::new()),
            api_key,
            client: HttpClient::shared(),
        }
    }

    pub fn with_v3_endpoints(mut self, v3_endpoints: std::sync::Arc<Vec<String>>) -> Self {
        self.v3_endpoints = v3_endpoints;
        self
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let mut url = format!("{}{}", base.trim_end_matches('/'), path);
            if let Some(key) = &api_key {
                url.push_str(&format!("&api_key={key}"));
            }
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// GET from the TonCenter v3 base URL (if configured).
    pub(crate) async fn get_v3<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        if self.v3_endpoints.is_empty() {
            return Err("ton: no v3 endpoints configured".to_string());
        }
        let path = path.to_string();
        let api_key = self.api_key.clone();
        with_fallback(&self.v3_endpoints, |base| {
            let client = self.client.clone();
            let api_key = api_key.clone();
            let mut url = format!("{}{}", base.trim_end_matches('/'), path);
            if let Some(key) = &api_key {
                // v3 uses query param `api_key` as well
                if url.contains('?') {
                    url.push_str(&format!("&api_key={key}"));
                } else {
                    url.push_str(&format!("?api_key={key}"));
                }
            }
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}
// TON fetch paths: balance, seqno, history (TonCenter v2), jetton balances (v3).



impl TonClient {
    /// Fetch all jetton (token) balances for `address` via the TonCenter v3 API.
    /// Returns a list of `TonJettonBalance` entries — one per jetton wallet found.
    pub async fn fetch_jetton_balances(
        &self,
        address: &str,
    ) -> Result<Vec<TonJettonBalance>, String> {
        #[derive(Deserialize)]
        struct Envelope {
            jetton_wallets: Option<Vec<JettonEntry>>,
        }
        #[derive(Deserialize)]
        struct JettonEntry {
            balance: Option<String>,
            address: Option<String>,
            jetton: Option<AddressWrapper>,
        }
        #[derive(Deserialize)]
        struct AddressWrapper {
            address: Option<String>,
        }

        let path = format!("/jetton/wallets?owner_address={address}&limit=100");
        let resp: Envelope = self.get_v3(&path).await?;
        let wallets = resp.jetton_wallets.unwrap_or_default();
        Ok(wallets
            .into_iter()
            .filter_map(|entry| {
                let master_address = entry.jetton?.address?;
                let wallet_address = entry.address?;
                let balance_raw: u128 = entry.balance?.parse().ok()?;
                Some(TonJettonBalance {
                    master_address,
                    wallet_address,
                    balance_raw,
                })
            })
            .collect())
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<TonBalance, String> {
        #[derive(Deserialize)]
        struct Resp {
            result: String,
        }
        let resp: Resp = self
            .get(&format!("/getAddressBalance?address={address}"))
            .await?;
        let nanotons: u64 = resp.result.parse().unwrap_or(0);
        Ok(TonBalance {
            nanotons,
            ton_display: format_ton(nanotons),
        })
    }

    pub async fn fetch_seqno(&self, address: &str) -> Result<u32, String> {
        #[derive(Deserialize)]
        struct Resp {
            result: u32,
        }
        let resp: Resp = self
            .get(&format!(
                "/runGetMethod?address={address}&method=seqno&stack=[]"
            ))
            .await
            .unwrap_or(Resp { result: 0 });
        Ok(resp.result)
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<TonHistoryEntry>, String> {
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<TonTx>,
        }
        #[derive(Deserialize)]
        struct TonTx {
            transaction_id: TonTxId,
            utime: u64,
            in_msg: Option<TonMsg>,
            out_msgs: Vec<TonMsg>,
            fee: String,
        }
        #[derive(Deserialize)]
        struct TonTxId {
            hash: String,
        }
        #[derive(Deserialize)]
        struct TonMsg {
            source: String,
            destination: String,
            value: String,
            #[serde(default)]
            message: String,
        }

        let resp: Resp = self
            .get(&format!(
                "/getTransactions?address={address}&limit=50&archival=false"
            ))
            .await?;

        let mut entries = Vec::new();
        for tx in resp.result {
            let txid = tx.transaction_id.hash;
            let timestamp = tx.utime;
            let fee: u64 = tx.fee.parse().unwrap_or(0);

            // Incoming: in_msg.destination == address
            if let Some(msg) = &tx.in_msg {
                if !msg.destination.is_empty() {
                    let amount: u64 = msg.value.parse().unwrap_or(0);
                    let comment = if msg.message.is_empty() {
                        None
                    } else {
                        Some(msg.message.clone())
                    };
                    entries.push(TonHistoryEntry {
                        txid: txid.clone(),
                        timestamp,
                        from: msg.source.clone(),
                        to: msg.destination.clone(),
                        amount_nanotons: amount,
                        fee_nanotons: fee,
                        is_incoming: true,
                        comment,
                    });
                }
            }
            // Outgoing.
            for msg in &tx.out_msgs {
                let amount: u64 = msg.value.parse().unwrap_or(0);
                entries.push(TonHistoryEntry {
                    txid: txid.clone(),
                    timestamp,
                    from: msg.source.clone(),
                    to: msg.destination.clone(),
                    amount_nanotons: amount,
                    fee_nanotons: fee,
                    is_incoming: false,
                    comment: None,
                });
            }
        }
        Ok(entries)
    }
}

fn format_ton(nanotons: u64) -> String {
    let whole = nanotons / 1_000_000_000;
    let frac = nanotons % 1_000_000_000;
    if frac == 0 {
        return whole.to_string();
    }
    let frac_str = format!("{:09}", frac);
    let trimmed = frac_str.trim_end_matches('0');
    let capped = if trimmed.len() > 6 { &trimmed[..6] } else { trimmed };
    format!("{}.{}", whole, capped)
}
