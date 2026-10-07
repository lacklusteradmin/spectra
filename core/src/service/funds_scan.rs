//! A bounded scan session. Only addresses survive candidate generation.
//!
//! A scan sends every candidate address to a provider before any wallet
//! exists, so it never starts on its own: a front end begins it, can show
//! whom it will ask (`endpoints`) first, and advances it one batch at a time.
use super::*;
use crate::derivation::funds_finder::{
    FundsFinderCandidate, FundsFinderRequest, SCANNED_ACCOUNTS, chain_candidates,
    generate_funds_finder_candidates,
};
use futures::{StreamExt, stream};

#[derive(Clone, uniffi::Record)]
pub struct FundsScanRead {
    pub candidate: FundsFinderCandidate,
    pub balance: Option<NativeBalanceSummary>,
    pub funded: bool,
    /// Whether the account has been used: it holds funds, or its history
    /// has a transaction — an emptied account was still used. Without a
    /// history endpoint on the network, the balance alone decides.
    pub used: bool,
    /// Why this address could not be read. Typed, so a front end words it as
    /// it words any other failed call rather than showing transport detail.
    pub error: Option<SpectraBridgeError>,
}

/// One endpoint a scan will send candidate addresses to.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FundsScanEndpoint {
    pub chain_id: crate::registry::Chain,
    pub endpoint: String,
    pub capabilities: Vec<EndpointCapability>,
}
#[derive(Clone, uniffi::Record)]
pub struct FundsScanProgress {
    pub total: u32,
    pub checked: u32,
    pub complete: bool,
    pub reads: Vec<FundsScanRead>,
}
#[derive(Clone, uniffi::Object)]
pub struct FundsScan {
    service: WalletService,
    candidates: Arc<Vec<FundsFinderCandidate>>,
    checked: Arc<tokio::sync::Mutex<usize>>,
}
#[uniffi::export]
impl WalletService {
    /// Begin a scan of `request`'s phrase: on one network, its derivation
    /// profiles at the first accounts — the network page's "find used
    /// accounts" — or with no network, every mainnet that restores BIP-39 —
    /// Funds Finder. Derives the candidates and contacts nothing.
    pub fn begin_funds_scan(
        &self,
        request: FundsFinderRequest,
        chain_id: Option<crate::registry::Chain>,
    ) -> Result<Arc<FundsScan>, SpectraBridgeError> {
        let candidates = match chain_id {
            Some(chain) => {
                // One account per phrase has nothing to find among.
                if chain.derivation_profiles().is_empty() && !chain.has_wallet_versions() {
                    return Err(crate::derivation::error::DerivationError::refused(
                        "%@ derives one account from a phrase; there are no others to find.",
                        [chain.chain_display_name()],
                    )
                    .into());
                }
                crate::derivation::phrase::check_phrase(
                    chain,
                    &request.seed_phrase,
                    request.passphrase.as_deref(),
                )?;
                chain_candidates(
                    chain,
                    &request.seed_phrase,
                    request.passphrase.as_deref(),
                    SCANNED_ACCOUNTS,
                )
            }
            None => generate_funds_finder_candidates(request)?,
        };
        if candidates.is_empty() {
            return Err(SpectraBridgeError::failure(
                "no scan candidates for this chain",
            ));
        }
        Ok(Arc::new(FundsScan {
            service: self.clone(),
            candidates: Arc::new(candidates),
            checked: Arc::new(tokio::sync::Mutex::new(0)),
        }))
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl FundsScan {
    pub fn candidates(&self) -> Vec<FundsFinderCandidate> {
        self.candidates.to_vec()
    }
    /// The endpoints the scan will send its candidate addresses to, by
    /// network, for a front end to name before the first batch.
    pub async fn endpoints(&self) -> Vec<FundsScanEndpoint> {
        let mut chains: Vec<crate::registry::Chain> = Vec::new();
        for candidate in self.candidates.iter() {
            if !chains.contains(&candidate.chain_id) {
                chains.push(candidate.chain_id);
            }
        }
        let mut endpoints = Vec::new();
        for chain in chains {
            let configured = self.service.configured_endpoints_on(chain).await;
            endpoints.extend(
                super::setup_summary::reading(
                    &configured,
                    &[EndpointCapability::Balance, EndpointCapability::History],
                )
                .into_iter()
                .map(|endpoint| FundsScanEndpoint {
                    chain_id: chain,
                    endpoint: endpoint.endpoint,
                    capabilities: endpoint.capabilities,
                }),
            );
        }
        endpoints
    }
    /// Cancellation leaves this batch unconsumed; a caller may retry it.
    pub async fn next_batch(&self) -> FundsScanProgress {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let mut checked = this.checked.lock().await;
            let end = (*checked + 4).min(this.candidates.len());
            // `buffered` keeps the candidates' order: the default profile
            // first, accounts ascending.
            let reads = stream::iter(this.candidates[*checked..end].iter().cloned())
                .map(|candidate| async { this.read(candidate).await })
                .buffered(4)
                .collect()
                .await;
            *checked = end;
            FundsScanProgress {
                total: this.candidates.len() as u32,
                checked: end as u32,
                complete: end == this.candidates.len(),
                reads,
            }
        })
        .await
    }
}
impl FundsScan {
    /// One candidate's balance, and its history when the balance is empty
    /// and the network has a history endpoint. A failed read is reported as
    /// unread, never as an unused account.
    async fn read(&self, candidate: FundsFinderCandidate) -> FundsScanRead {
        let chain = candidate.chain_id;
        let address = candidate.address.clone();
        let result = async {
            let balance = self
                .service
                .fetch_native_balance_summary(chain, address.clone())
                .await?;
            let is_funded = funded(&balance.smallest_unit)?;
            let has_history = !is_funded
                && self
                    .service
                    .configured_endpoints_on(chain)
                    .await
                    .iter()
                    .any(|e| e.capabilities.contains(&EndpointCapability::History));
            let used = is_funded
                || (has_history
                    && self
                        .service
                        .fetch_history_summary(chain, address)
                        .await?
                        .entry_count
                        > 0);
            Ok::<_, SpectraBridgeError>((balance, is_funded, used))
        }
        .await;
        match result {
            Ok((balance, funded, used)) => FundsScanRead {
                candidate,
                balance: Some(balance),
                funded,
                used,
                error: None,
            },
            Err(error) => FundsScanRead {
                candidate,
                balance: None,
                funded: false,
                used: false,
                error: Some(error),
            },
        }
    }
}

fn funded(raw: &str) -> Result<bool, SpectraBridgeError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SpectraBridgeError::failure("invalid smallest-unit balance"));
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
    use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::method};

    /// A TON mnemonic holds one account per wallet version, the default
    /// first, each the fixture's; a chain with one account per phrase still
    /// has nothing to find.
    #[test]
    fn a_ton_phrase_is_scanned_as_each_wallet_version() {
        use crate::derivation::ton::TonWalletVersion;
        use crate::registry::Chain;
        let mnemonics: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/ton-mnemonics.json")).unwrap();
        let w5: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/ton-w5.json")).unwrap();
        let mnemonic = mnemonics["mnemonics"][0]["mnemonic"].as_str().unwrap();
        let service = WalletService::new(Vec::new()).unwrap();
        let request = || FundsFinderRequest {
            seed_phrase: mnemonic.into(),
            passphrase: None,
        };
        let candidates = service
            .begin_funds_scan(request(), Some(Chain::Ton))
            .unwrap()
            .candidates();
        let found: Vec<_> = candidates
            .iter()
            .map(|c| (c.ton_wallet_version, c.address.as_str(), c.profile))
            .collect();
        assert_eq!(
            found,
            [
                (
                    Some(TonWalletVersion::W5),
                    w5["addresses"][0]["mainnet"].as_str().unwrap(),
                    None
                ),
                (
                    Some(TonWalletVersion::V4R2),
                    mnemonics["mnemonics"][0]["address"].as_str().unwrap(),
                    None
                ),
            ]
        );
        for chain in [Chain::Polkadot, Chain::Monero] {
            assert!(service.begin_funds_scan(request(), Some(chain)).is_err());
        }
    }
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
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: crate::registry::Chain::Ethereum,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        // No history endpoint on the network: the balance alone decides.
        service
            .wallet_state
            .write()
            .await
            .settings
            .custom_endpoints_only
            .push(crate::registry::Chain::Ethereum);
        let scan = FundsScan {
            service: (*service).clone(),
            checked: Arc::new(tokio::sync::Mutex::new(0)),
            candidates: Arc::new(
                (1..=3)
                    .map(|n| FundsFinderCandidate {
                        chain_id: crate::registry::Chain::Ethereum,
                        profile: None,
                        account: 0,
                        ton_wallet_version: None,
                        derivation_path: "fixture".into(),
                        address: format!("0x{n:040x}"),
                    })
                    .collect(),
            ),
        };
        let batch = scan.next_batch().await;
        assert_eq!(batch.checked, 3);
        assert!(batch.complete);
        assert!(!batch.reads[0].funded);
        assert!(batch.reads[0].error.is_none());
        assert!(batch.reads[1].funded);
        assert!(batch.reads[2].error.is_some());
        assert!(batch.reads[1].used && !batch.reads[0].used);
        assert!(scan.next_batch().await.reads.is_empty());
    }

    /// An emptied account with history was used; a history read that fails
    /// is reported unread, not unused; the reads keep the candidates' order;
    /// and the scan names the endpoints it will ask before it asks them.
    #[tokio::test]
    async fn an_emptied_account_with_history_is_used_and_failures_stay_unread() {
        use wiremock::matchers::query_param;
        let server = MockServer::start().await;
        let address = |n: u8| format!("0x{n:040x}");
        Mock::given(method("POST"))
            .respond_with(move |request: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                let funded = body["params"][0].as_str().unwrap().ends_with('2');
                ResponseTemplate::new(200).set_body_json(
                    json!({"jsonrpc":"2.0","id":1,"result": if funded { "0x5" } else { "0x0" }}),
                )
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("address", address(1)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "1", "message": "OK", "result": [{
                    "hash": format!("0x{}", "ab".repeat(32)), "blockNumber": "42",
                    "timeStamp": "1700000000", "from": address(9), "to": address(1),
                    "value": "1", "gasPrice": "1", "gasUsed": "21000", "isError": "0",
                    "txreceipt_status": "1"
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("address", address(3)))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let chain = crate::registry::Chain::Ethereum;
        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: EndpointCapability::ALL.to_vec(),
            chain_id: chain,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        // The indexer at its own path: one URL is one endpoint.
        let indexer = format!("{}/scout", server.uri());
        {
            let mut state = service.wallet_state.write().await;
            state.settings.custom_endpoints_only.push(chain);
            state
                .settings
                .custom_endpoints
                .push(crate::service::CustomEndpoint {
                    chain_id: chain,
                    api: crate::EndpointApi::Blockscout,
                    endpoint: indexer.clone(),
                    capabilities: vec![EndpointCapability::History],
                });
        }
        let candidates: Vec<_> = (1..=3)
            .map(|n| FundsFinderCandidate {
                chain_id: chain,
                profile: Some(crate::chains::DerivationProfile::Standard),
                account: u32::from(n) - 1,
                ton_wallet_version: None,
                derivation_path: format!("m/44'/60'/{}'/0/0", n - 1),
                address: address(n),
            })
            .collect();
        let scan = FundsScan {
            service: (*service).clone(),
            checked: Arc::new(tokio::sync::Mutex::new(0)),
            candidates: Arc::new(candidates.clone()),
        };
        assert_eq!(
            scan.endpoints().await,
            [FundsScanEndpoint {
                chain_id: chain,
                endpoint: indexer,
                capabilities: vec![EndpointCapability::History],
            }]
        );
        let reads = scan.next_batch().await.reads;
        assert_eq!(
            reads
                .iter()
                .map(|r| r.candidate.address.clone())
                .collect::<Vec<_>>(),
            candidates
                .iter()
                .map(|c| c.address.clone())
                .collect::<Vec<_>>()
        );
        assert!(!reads[0].funded && reads[0].used && reads[0].error.is_none());
        assert!(reads[1].funded && reads[1].used);
        assert!(!reads[2].used && reads[2].error.is_some());
    }
}
