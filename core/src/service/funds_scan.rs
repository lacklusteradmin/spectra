//! A bounded scan session. Only addresses survive candidate generation.
use super::*;
use crate::derivation::funds_finder::{
    core_generate_funds_finder_candidates, FundsFinderCandidate, FundsFinderRequest,
};
use futures::{stream, StreamExt};

#[derive(Clone, serde::Serialize, uniffi::Record)]
pub struct FundsScanRead {
    pub candidate: FundsFinderCandidate,
    pub balance: Option<NativeBalanceSummary>,
    pub funded: bool,
    pub error: Option<String>,
}
#[derive(Clone, serde::Serialize, uniffi::Record)]
pub struct FundsScanProgress {
    pub total: u32,
    pub checked: u32,
    pub complete: bool,
    pub reads: Vec<FundsScanRead>,
}
#[derive(uniffi::Object)]
pub struct FundsScan {
    service: WalletService,
    candidates: Vec<FundsFinderCandidate>,
    checked: tokio::sync::Mutex<usize>,
}
#[uniffi::export]
impl WalletService {
    pub fn begin_funds_scan(
        &self,
        request: FundsFinderRequest,
        chain_id: Option<String>,
    ) -> Result<Arc<FundsScan>, SpectraBridgeError> {
        if let Some(id) = &chain_id {
            chain_for_id(id)?;
        }
        let mut candidates = core_generate_funds_finder_candidates(request)?;
        if let Some(id) = chain_id {
            candidates.retain(|c| c.chain_id == id);
        }
        if candidates.is_empty() {
            return Err("no scan candidates for this chain".into());
        }
        Ok(Arc::new(FundsScan {
            service: self.clone(),
            candidates,
            checked: tokio::sync::Mutex::new(0),
        }))
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl FundsScan {
    pub fn candidates(&self) -> Vec<FundsFinderCandidate> {
        self.candidates.clone()
    }
    /// Cancellation leaves this batch unconsumed; a caller may retry it.
    pub async fn next_batch(&self) -> FundsScanProgress {
        let mut checked = self.checked.lock().await;
        let end = (*checked + 4).min(self.candidates.len());
        let reads = stream::iter(self.candidates[*checked..end].iter().cloned())
            .map(|candidate| async {
                let result = self
                    .service
                    .fetch_native_balance_summary(
                        candidate.chain_id.clone(),
                        candidate.address.clone(),
                    )
                    .await
                    .and_then(|balance| {
                        funded(&balance.smallest_unit).map(|is_funded| (balance, is_funded))
                    });
                match result {
                    Ok((balance, funded)) => FundsScanRead {
                        candidate,
                        balance: Some(balance),
                        funded,
                        error: None,
                    },
                    Err(error) => FundsScanRead {
                        candidate,
                        balance: None,
                        funded: false,
                        error: Some(error.to_string()),
                    },
                }
            })
            .buffered(4)
            .collect()
            .await;
        *checked = end;
        FundsScanProgress {
            total: self.candidates.len() as u32,
            checked: end as u32,
            complete: end == self.candidates.len(),
            reads,
        }
    }
}
fn funded(raw: &str) -> Result<bool, SpectraBridgeError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid smallest-unit balance".into());
    }
    Ok(raw.bytes().any(|b| b != b'0'))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_distinguishes_zero_funds_and_invalid_reads() {
        assert!(!funded("000").unwrap());
        assert!(funded("100000000000000000000000000000000000000000000000001").unwrap());
        for raw in ["", "-1", "NaN", "0.1"] {
            assert!(funded(raw).is_err());
        }
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use wiremock::{matchers::method, Mock, MockServer, Request, ResponseTemplate};
    #[tokio::test]
    async fn scan_reports_failed_reads_separately_and_finishes_batches() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(|request: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                let result = match body["params"][0].as_str().unwrap().chars().last().unwrap() {
                    '1' => json!("0x0"),
                    '2' => json!("0x1"),
                    _ => json!("invalid"),
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc":"2.0","id":1,"result":result}))
            })
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let scan = FundsScan {
            service: (*service).clone(),
            checked: tokio::sync::Mutex::new(0),
            candidates: (1..=3)
                .map(|n| FundsFinderCandidate {
                    chain_id: "ethereum".into(),
                    chain_name: "Ethereum".into(),
                    derivation_path: "fixture".into(),
                    path_label: "fixture".into(),
                    address: format!("0x{n:040x}"),
                })
                .collect(),
        };
        let batch = scan.next_batch().await;
        assert_eq!(batch.checked, 3);
        assert!(batch.complete);
        assert!(!batch.reads[0].funded);
        assert!(batch.reads[0].error.is_none());
        assert!(batch.reads[1].funded);
        assert!(batch.reads[2].error.is_some());
        assert!(scan.next_batch().await.reads.is_empty());
    }
}
