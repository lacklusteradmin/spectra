//! TON fetch paths: balance, seqno, history (TonCenter v2), jetton balances (v3).

use serde::Deserialize;

use super::{TonBalance, TonClient, TonHistoryEntry, TonJettonBalance};

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
