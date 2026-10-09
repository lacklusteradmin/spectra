//! The TON Center v2 adapter: balances, wallet seqno, transaction history
//! and BOC submission. Jettons are `toncenter_v3`.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::{HttpClient, RetryProfile, race};

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TonBalance {
    /// Nanotons (1 TON = 1_000_000_000 nanotons).
    pub nanotons: u64,
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

// ── Client

/// An account as `getAddressInformation` shows it.
#[derive(Debug, Clone)]
pub struct TonAccountState {
    pub state: String,
    pub nanotons: u64,
    pub code: Vec<u8>,
    pub data: Vec<u8>,
}

pub struct ToncenterV2Client {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl ToncenterV2Client {
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
        let path = path.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}{}", base.trim_end_matches('/'), path);
            async move { client.get_json(&url, RetryProfile::ChainRead).await }
        })
        .await
    }
}

impl ToncenterV2Client {
    pub async fn verify_network(&self, chain: crate::registry::Chain) -> Result<(), ApiError> {
        let (root, file) = chain
            .ton_zero_state()
            .or_decode("Unsupported TON network")?;
        let response: Value = self.get("/getMasterchainInfo").await?;
        let init = &response["result"]["init"];
        if response["ok"].as_bool() != Some(true)
            || init["workchain"].as_i64() != Some(-1)
            || init["seqno"].as_u64() != Some(0)
            || init["root_hash"].as_str() != Some(root)
            || init["file_hash"].as_str() != Some(file)
        {
            return Err(ApiError::invalid(
                "TON endpoint has the wrong masterchain zero state",
            ));
        }
        Ok(())
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<TonBalance, ApiError> {
        #[derive(Deserialize)]
        struct Resp {
            result: String,
        }
        let resp: Resp = self
            .get(&format!("/getAddressBalance?address={address}"))
            .await?;
        let nanotons: u64 = resp.result.parse().unwrap_or(0);
        Ok(TonBalance { nanotons })
    }

    /// A wallet account's state (`active`, `uninitialized`, `frozen`) and,
    /// once deployed, the wallet contract the node recognizes, from one
    /// verified node.
    pub async fn fetch_wallet_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<(String, Option<String>), ApiError> {
        #[derive(Deserialize)]
        struct Info {
            account_state: String,
            #[serde(default)]
            wallet_type: Option<String>,
        }
        #[derive(Deserialize)]
        struct Resp {
            ok: bool,
            result: Info,
        }
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let response: Resp = node
                .get(&format!("/getWalletInformation?address={address}"))
                .await?;
            if !response.ok {
                return Err(ApiError::rejected("TON wallet information refused"));
            }
            Ok((response.result.account_state, response.result.wallet_type))
        })
        .await
    }

    /// An account's state (`active`, `uninitialized`, `frozen`), balance in
    /// nanotons, and its code and data as BOCs (empty unless active), from
    /// one verified node.
    pub async fn fetch_account_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<TonAccountState, ApiError> {
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let response: Value = node
                .get(&format!("/getAddressInformation?address={address}"))
                .await?;
            let result = &response["result"];
            if response["ok"].as_bool() != Some(true) {
                return Err(ApiError::rejected("TON account information refused"));
            }
            use base64::Engine;
            let boc = |field: &str| {
                base64::engine::general_purpose::STANDARD
                    .decode(result[field].as_str().unwrap_or_default())
                    .map_err(|_| ApiError::Decode("TON: invalid account BOC".into()))
            };
            Ok(TonAccountState {
                state: result["state"]
                    .as_str()
                    .or_decode("TON: missing account state")?
                    .to_string(),
                nanotons: result["balance"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| result["balance"].as_u64().map(|n| n.to_string()))
                    .or_decode("TON: missing account balance")?
                    .parse()
                    .map_err(ApiError::decode)?,
                code: boc("code")?,
                data: boc("data")?,
            })
        })
        .await
    }

    pub async fn fetch_seqno(&self, address: &str) -> Result<u32, ApiError> {
        use serde_json::{Value, json};
        // A failed read is not an undeployed wallet. Only a positive state
        // response may select seqno zero and the deployment path.
        let info: Value = self
            .get(&format!("/getAddressInformation?address={address}"))
            .await?;
        if info["ok"].as_bool() != Some(true) {
            return Err(ApiError::Decode("TON: cannot read account state".into()));
        }
        match info["result"]["state"].as_str() {
            Some("uninitialized") => return Ok(0),
            Some("active") => {}
            _ => {
                return Err(ApiError::Decode(
                    "TON: account is frozen or state is unreadable".into(),
                ));
            }
        }
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let url = format!("{}/runGetMethod", base.trim_end_matches('/'));
            let body = json!({"address": address, "method": "seqno", "stack": []});
            async move {
                let response: Value = client
                    .post_json(&url, &body, RetryProfile::ChainRead)
                    .await?;
                if response["ok"].as_bool() != Some(true)
                    || response["result"]["exit_code"].as_i64() != Some(0)
                {
                    return Err(ApiError::Decode("TON: seqno get method failed".into()));
                }
                let stack = response["result"]["stack"]
                    .as_array()
                    .or_decode("TON: missing seqno stack")?;
                if stack.len() != 1 || stack[0][0].as_str() != Some("num") {
                    return Err(ApiError::Decode("TON: invalid seqno stack".into()));
                }
                let value = stack[0][1].as_str().or_decode("TON: missing seqno value")?;
                u32::from_str_radix(
                    value
                        .strip_prefix("0x")
                        .or_decode("TON: invalid seqno encoding")?,
                    16,
                )
                .map_err(|_| ApiError::Decode("TON: invalid seqno range".into()))
            }
        })
        .await
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<TonHistoryEntry>, ApiError> {
        Ok(self.fetch_history_page(address, None).await?.items)
    }

    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<TonHistoryEntry>, ApiError> {
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<TonTx>,
        }
        let continuation = cursor
            .map(|value| {
                value
                    .parse::<u64>()
                    .map(|lt| format!("&lt={lt}"))
                    .map_err(ApiError::invalid)
            })
            .transpose()?
            .unwrap_or_default();
        let resp: Resp = self
            .get(&format!(
                "/getTransactions?address={address}&limit=50&archival=true{continuation}"
            ))
            .await?;
        let next_cursor = if resp.result.len() == 50 {
            Some(
                resp.result
                    .last()
                    .and_then(|row| row.transaction_id.lt.parse::<u64>().ok())
                    .and_then(|lt| lt.checked_sub(1))
                    .or_decode("TON: full history page has no valid logical time")?
                    .to_string(),
            )
        } else {
            None
        };
        Ok(crate::api::HistoryPage {
            items: ton_history_from_transactions(resp.result),
            next_cursor,
        })
    }
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
    #[serde(default)]
    lt: String,
}
#[derive(Deserialize)]
struct TonMsg {
    source: String,
    destination: String,
    value: String,
    #[serde(default)]
    message: String,
}

/// The value-carrying messages of each transaction, one entry per message.
///
/// A message with no source is external: the signed request that starts a
/// wallet's own send, carrying no value. One with no destination is an
/// external log. Neither moves TON between accounts, so neither is an entry —
/// the external request was read as a receipt of 0 TON beside every send.
fn ton_history_from_transactions(txs: Vec<TonTx>) -> Vec<TonHistoryEntry> {
    let mut entries = Vec::new();
    for tx in txs {
        let txid = tx.transaction_id.hash;
        let timestamp = tx.utime;
        let fee: u64 = tx.fee.parse().unwrap_or(0);
        let internal = |msg: &TonMsg| !msg.source.is_empty() && !msg.destination.is_empty();

        if let Some(msg) = tx.in_msg.as_ref().filter(|msg| internal(msg)) {
            entries.push(TonHistoryEntry {
                txid: txid.clone(),
                timestamp,
                from: msg.source.clone(),
                to: msg.destination.clone(),
                amount_nanotons: msg.value.parse().unwrap_or(0),
                fee_nanotons: fee,
                is_incoming: true,
                comment: Some(msg.message.clone()).filter(|m| !m.is_empty()),
            });
        }
        for msg in tx.out_msgs.iter().filter(|msg| internal(msg)) {
            entries.push(TonHistoryEntry {
                txid: txid.clone(),
                timestamp,
                from: msg.source.clone(),
                to: msg.destination.clone(),
                amount_nanotons: msg.value.parse().unwrap_or(0),
                fee_nanotons: fee,
                is_incoming: false,
                comment: None,
            });
        }
    }
    entries
}

impl ToncenterV2Client {
    /// Send a pre-built BOC (for rebroadcast).
    pub async fn send_boc(&self, boc_b64: &str) -> Result<TonSendResult, ApiError> {
        let body = json!({"boc": boc_b64});
        let boc_b64 = boc_b64.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let body = body.clone();
            let boc_b64 = boc_b64.clone();
            let url = format!("{}/sendBocReturnHash", base.trim_end_matches('/'));
            async move {
                let resp: Value = client
                    .post_json(&url, &body, RetryProfile::ChainWrite)
                    .await?;
                if resp.get("ok").and_then(Value::as_bool) != Some(true) {
                    return Err(ApiError::Rejected(format!(
                        "TON broadcast rejected: {}",
                        resp.get("error").unwrap_or(&Value::Null)
                    )));
                }
                let hash = resp
                    .get("result")
                    .and_then(|r| r.get("hash"))
                    .and_then(Value::as_str)
                    .or_decode("TON broadcast: missing message hash")?
                    .to_string();
                use base64::Engine;
                if base64::engine::general_purpose::STANDARD
                    .decode(&hash)
                    .map_err(|_| ApiError::Decode("TON broadcast: invalid hash".into()))?
                    .len()
                    != 32
                {
                    return Err(ApiError::Decode(
                        "TON broadcast: invalid hash length".into(),
                    ));
                }
                Ok(TonSendResult {
                    message_hash: hash,
                    boc_b64,
                })
            }
        })
        .await
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "EQCD39VS5jcptHL8vMjEXrzGaRcCVYto7HUn4bpAOg8xqB2N";
    const THEM: &str = "EQBvW8Z5huBkMJYdnfAEM5JqTNkuWX3diqYENkWsIL0XggGG";

    /// A toncenter v2 wallet send: the external request comes in with no
    /// source and no value, and the transfer goes out.
    #[test]
    fn a_send_is_one_outgoing_entry_without_a_zero_receipt() {
        let txs: Vec<TonTx> = serde_json::from_value(serde_json::json!([{
            "transaction_id": {"hash": "send"},
            "utime": 1_790_000_000u64,
            "fee": "2780000",
            "in_msg": {"source": "", "destination": ME, "value": "0", "message": ""},
            "out_msgs": [{"source": ME, "destination": THEM, "value": "1500000000", "message": ""}]
        }, {
            "transaction_id": {"hash": "receive"},
            "utime": 1_790_000_100u64,
            "fee": "0",
            "in_msg": {"source": THEM, "destination": ME, "value": "250000000", "message": "hi"},
            "out_msgs": []
        }]))
        .unwrap();
        let entries = ton_history_from_transactions(txs);
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_nanotons, 1_500_000_000);
        assert!(entries[1].is_incoming);
        assert_eq!(entries[1].amount_nanotons, 250_000_000);
        assert_eq!(entries[1].comment.as_deref(), Some("hi"));
    }
}

#[cfg(test)]
mod submission_tests {
    use super::*;

    #[tokio::test]
    async fn ton_reads_real_seqno_and_refuses_failed_reads_and_submissions() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        let client = ToncenterV2Client::new(std::sync::Arc::new(vec![server.uri()]));
        Mock::given(method("GET"))
            .and(path("/getAddressInformation"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"state":"active"}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/runGetMethod"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"ok":true,"result":{"exit_code":0,"stack":[["num","0x2a"]]}}),
            ))
            .mount(&server)
            .await;
        assert_eq!(client.fetch_seqno("address").await.unwrap(), 42);
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":false})))
            .mount(&server)
            .await;
        assert!(client.fetch_seqno("address").await.is_err());
        Mock::given(method("POST"))
            .and(path("/sendBocReturnHash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"invalid boc"})),
            )
            .mount(&server)
            .await;
        assert!(client.send_boc("payload").await.is_err());
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"state":"uninitialized"}})),
            )
            .mount(&server)
            .await;
        assert_eq!(client.fetch_seqno("address").await.unwrap(), 0);
    }
}

#[cfg(test)]
mod network_identity_tests {
    use super::*;
    use crate::registry::Chain;
    use std::sync::Arc;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

    #[tokio::test]
    async fn custom_nodes_require_both_official_zero_state_hashes() {
        for chain in [Chain::Ton, Chain::TonTestnet] {
            let (root, file) = chain.ton_zero_state().unwrap();
            let valid = json!({"ok":true,"result":{"init":{"workchain":-1,"seqno":0,"root_hash":root,"file_hash":file}}});
            let other = if chain == Chain::Ton {
                Chain::TonTestnet
            } else {
                Chain::Ton
            };
            let other_root = other.ton_zero_state().unwrap().0;
            let mut wrong_root = valid.clone();
            wrong_root["result"]["init"]["root_hash"] = json!(other_root);
            let mut wrong_file = valid.clone();
            wrong_file["result"]["init"]["file_hash"] = json!("missing");
            let mut failed = valid.clone();
            failed["ok"] = json!(false);
            let mut nonzero = valid.clone();
            nonzero["result"]["init"]["seqno"] = json!(1);
            for (response, accepted) in [
                (valid, true),
                (wrong_root, false),
                (wrong_file, false),
                (failed, false),
                (nonzero, false),
                (json!({}), false),
            ] {
                let server = MockServer::start().await;
                Mock::given(path("/getMasterchainInfo"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(response))
                    .expect(1)
                    .mount(&server)
                    .await;
                let client = ToncenterV2Client::new(Arc::new(vec![server.uri()]));
                assert_eq!(client.verify_network(chain).await.is_ok(), accepted);
            }
        }
    }
}
