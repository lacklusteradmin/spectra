//! The TON Center v3 adapter: the indexer that enumerates jetton wallets and
//! reads jetton masters, which v2 cannot.

use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, RetryProfile, race};

/// One jetton (token) balance entry returned by the v3 API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonJettonBalance {
    /// Jetton master contract address (matches the known-token `contract` field).
    pub master_address: String,
    /// Jetton wallet contract address (holder's personal wallet for this token).
    pub wallet_address: String,
    /// Raw balance in the token's smallest unit.
    pub balance_raw: u128,
}

pub struct ToncenterV3Client {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl ToncenterV3Client {
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
        if self.endpoints.is_empty() {
            return Err("ton: no v3 endpoints configured".to_string());
        }
        let path = path.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

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
        let resp: Envelope = self.get(&path).await?;
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

    /// A jetton master's own decimals, from its content. `None` when the
    /// master will not answer.
    pub async fn fetch_jetton_decimals(&self, master_address: &str) -> Option<u8> {
        #[derive(Deserialize)]
        struct MasterEnvelope {
            jetton_masters: Option<Vec<Master>>,
        }
        #[derive(Deserialize)]
        struct Master {
            jetton_content: Option<Content>,
        }
        #[derive(Deserialize)]
        struct Content {
            decimals: Option<serde_json::Value>,
        }
        let path = format!("/jetton/masters?address={master_address}&limit=1");
        let content = self
            .get::<MasterEnvelope>(&path)
            .await
            .ok()?
            .jetton_masters?
            .into_iter()
            .next()?
            .jetton_content?;
        // TON metadata carries decimals as a string as often as a number, and
        // both mean the same count.
        let raw = content.decimals?;
        raw.as_u64()
            .or_else(|| raw.as_str().and_then(|s| s.parse().ok()))
            .map(|d| d as u8)
    }

    /// Every jetton the address holds, with each jetton master's own decimals.
    ///
    /// `/jetton/wallets` enumerates the holdings but carries no content, so
    /// each master's metadata is read concurrently; a master that will not
    /// answer is reported unnamed rather than dropped.
    pub async fn fetch_all_jetton_balances(
        &self,
        address: &str,
    ) -> Result<Vec<crate::api::HeldToken>, String> {
        let wallets: Vec<TonJettonBalance> = self
            .fetch_jetton_balances(address)
            .await?
            .into_iter()
            .filter(|w| w.balance_raw > 0)
            .collect();

        let metadata = futures::future::join_all(
            wallets
                .iter()
                .map(|w| self.fetch_jetton_decimals(&w.master_address)),
        )
        .await;

        Ok(wallets
            .into_iter()
            .zip(metadata)
            .map(|(w, decimals)| crate::api::HeldToken {
                contract: w.master_address,
                balance_raw: w.balance_raw,
                decimals,
                symbol: None,
            })
            .collect())
    }
}
