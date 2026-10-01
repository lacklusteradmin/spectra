//! The TON Center v3 adapter: the indexer that enumerates jetton wallets and
//! reads jetton masters, which v2 cannot.

use crate::api::error::ApiError;
use serde::Deserialize;

use crate::api::http::{HttpClient, RetryProfile, race};

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
    ) -> Result<T, ApiError> {
        if self.endpoints.is_empty() {
            return Err(ApiError::NoEndpoint);
        }
        let path = path.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }

    /// Every jetton `address` holds: one row per jetton wallet with a
    /// balance, keyed by its master's raw address (`0:HEX`), with the master's
    /// decimals when the indexer has its metadata.
    ///
    /// The list is paged. A holder with more pages than this reads is refused
    /// rather than truncated, since a caller reads a jetton missing from the
    /// list as a zero balance.
    pub async fn fetch_jetton_balances(
        &self,
        address: &str,
    ) -> Result<Vec<crate::api::HeldToken>, ApiError> {
        const PAGE_SIZE: usize = 1000;
        const MAX_PAGES: usize = 10;
        let mut held = Vec::new();
        for page in 0..MAX_PAGES {
            let offset = page * PAGE_SIZE;
            let response: serde_json::Value = self
                .get(&format!(
                    "/jetton/wallets?owner_address={address}&limit={PAGE_SIZE}&offset={offset}"
                ))
                .await?;
            let (rows, count) = parse_jetton_wallets(&response)?;
            held.extend(rows);
            if count < PAGE_SIZE {
                return Ok(held);
            }
        }
        Err(ApiError::Rejected(format!(
            "holds more than {} jetton wallets; the list cannot be read whole",
            PAGE_SIZE * MAX_PAGES
        )))
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
}

/// The holdings in one `/jetton/wallets` page, and how many wallets the page
/// listed. `jetton` is the master's raw address; its decimals ride in the
/// response's `metadata` when the indexer has read the master.
fn parse_jetton_wallets(
    response: &serde_json::Value,
) -> Result<(Vec<crate::api::HeldToken>, usize), ApiError> {
    use crate::api::error::OrDecode;
    let wallets = response
        .get("jetton_wallets")
        .and_then(serde_json::Value::as_array)
        .or_decode("jetton wallets: missing list")?;
    let decimals = |master: &str| {
        response
            .pointer(&format!("/metadata/{master}/token_info"))?
            .as_array()?
            .iter()
            .find(|info| info.get("type").and_then(|t| t.as_str()) == Some("jetton_masters"))?
            .pointer("/extra/decimals")
            .and_then(|raw| {
                // TON metadata carries decimals as a string as often as a
                // number, and both mean the same count.
                raw.as_u64().or_else(|| raw.as_str()?.parse().ok())
            })
            .and_then(|d| crate::api::checked_token_decimals(u128::from(d)).ok())
    };
    let held = wallets
        .iter()
        .filter_map(|wallet| {
            let master = wallet.get("jetton")?.as_str()?;
            let balance_raw: u128 = wallet.get("balance")?.as_str()?.parse().ok()?;
            (balance_raw > 0).then(|| crate::api::HeldToken {
                contract: master.to_string(),
                balance_raw,
                decimals: decimals(master),
            })
        })
        .collect();
    Ok((held, wallets.len()))
}

#[cfg(test)]
mod jetton_wallets {
    use super::parse_jetton_wallets;
    use serde_json::json;

    /// The shape `toncenter.com/api/v3/jetton/wallets` returns: `jetton` is a
    /// raw address string, and decimals live under `metadata`.
    #[test]
    fn a_page_names_each_master_and_its_decimals() {
        let usdt = "0:B113A994B5024A16719F69139328EB759596C38A25F59028B146FECDC3621DFE";
        let other = "0:52E0FE119C45BE79C25E2E7EDA3F7C6E90167036D5B390A2290C986C774A2EE1";
        let (held, count) = parse_jetton_wallets(&json!({
            "jetton_wallets": [
                {"address": "0:2626", "balance": "879187990649145", "jetton": usdt},
                {"address": "0:07FE", "balance": "500000000000", "jetton": other},
                {"address": "0:0000", "balance": "0", "jetton": other}
            ],
            "metadata": {
                usdt: {"token_info": [{"type": "jetton_masters", "extra": {"decimals": "6"}}]}
            }
        }))
        .expect("a page");
        assert_eq!(count, 3, "the page size counts every wallet listed");
        assert_eq!(held.len(), 2, "an empty jetton wallet is not a holding");
        assert_eq!(held[0].contract, usdt);
        assert_eq!(held[0].decimals, Some(6));
        assert_eq!(held[1].decimals, None, "no metadata, no decimals");
    }
}
