//! Endpoint health, contract probes and transaction status reads.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    // `fetch_history` lives in the plain-impl block below (JSON shuttle —
    // kept internal, not exported to Swift).

    // `sign_and_broadcast_send` lives in the plain `impl WalletService` block
    // in `service/send.rs`. UniFFI exports every method of a `#[uniffi::export]`
    // impl block regardless of `pub(crate)` visibility, so chain-dispatch
    // helpers consumed only by `execute_send` must be outside this block.

    // ── Token balance (ERC-20 / SPL / NEP-141 / TRC-20 / Stellar assets)

    // ── Unified execute_send — collapses derive → payload → sign trampoline

    // `execute_send` lives in `service/send_execution.rs`.

    // ── Bitcoin HD multi-address (xpub / ypub / zpub)

    // `bitcoin_xpub_balance` lives in the plain-impl block below: it returns a
    // typed `HdXpubBalance` to Rust callers only, not across the FFI.

    // ── EVM paginated history (native + ERC-20 token transfers)

    // `fetch_evm_history_page` lives in the plain-impl block below: it is
    // called by `history_refresh` and by `fetch_evm_history_diagnostics`,
    // not across the FFI.

    // ── Typed token-array wrappers (no JSON serialization on caller side)

    // ── EVM utilities (contract detection, nonce lookup)

    /// Returns true iff `address` has deployed bytecode on the given EVM chain.
    pub(crate) async fn fetch_evm_has_contract_code(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let code = client.fetch_code(&address).await?;
        Ok(crate::send::flow::core_evm_has_contract_code(code))
    }

    /// Fetch the nonce of a submitted transaction by hash on an EVM chain.
    /// Used to pre-fill the replacement-tx nonce field.
    pub async fn fetch_evm_tx_nonce_typed(
        &self,
        chain_id: String,
        tx_hash: String,
    ) -> Result<u64, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        client.fetch_tx_nonce(&tx_hash).await.map_err(Into::into)
    }

    // `fetch_utxo_fee_preview` and `broadcast_raw` live in the plain-impl
    // block below (JSON shuttles — kept internal, not exported to Swift).

    // `fetch_evm_send_preview` / `fetch_tron_send_preview` /
    // `fetch_simple_chain_send_preview` live in the plain-impl block below
    // (JSON shuttles — kept internal, not exported to Swift). Their typed
    // wrappers below call into those internal helpers.

    /// Call every registered endpoint and report which ones answer.
    ///
    /// The catalog is static JSON, so an endpoint that dies stays in the list
    /// and costs a full timeout plus the 180 ms `with_fallback` pause on every
    /// call that reaches it. Eleven were dead when this was written, and the
    /// only way anyone would have found out was opening the diagnostics screen
    /// for each chain in turn.
    ///
    /// A chain with no `rpc_health_method` and no `probe_url` reports
    /// `checked: false` rather than a pass — Aptos and Tron are REST rather
    /// than JSON-RPC, and calling them a success because nothing asked is how
    /// a dead endpoint hides.
    pub async fn probe_chain_endpoints(
        &self,
        chain_id: String,
    ) -> Result<Vec<EndpointProbe>, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        let name = chain.chain_display_name().to_string();
        let method = chain.rpc_health_method();
        let mut records = crate::endpoint_records_for_chain_masked(name.clone(), 0, false)
            .map_err(|e| SpectraBridgeError::from(format!("endpoints for {name}: {e}")))?;

        // Custom endpoints use the same protocol probe as the catalog's node.
        if let Some(template) = records.iter().find(|r| r.kind != "web-link").cloned() {
            for endpoint in self.endpoints_for(&chain_id).await.iter() {
                if records.iter().any(|r| &r.endpoint == endpoint) {
                    continue;
                }
                let mut custom = template.clone();
                custom.id = format!("configured:{endpoint}");
                custom.probe_url = template
                    .probe_url
                    .as_ref()
                    .map(|url| url.replacen(&template.endpoint, endpoint, 1));
                custom.endpoint = endpoint.clone();
                records.push(custom);
            }
        }
        let mut out = Vec::with_capacity(records.len());
        for record in records {
            // The `rpc` role means "JSON-RPC node", and only that: Bitcoin's
            // Esplora, Cardano's Koios and Stellar's Horizon are REST and
            // correctly lack it. Ten EVM chains were missing it while being
            // exactly that, so a role-gated probe GET a JSON-RPC endpoint and
            // called it dead — on this command and on the app's diagnostics
            // screen, which gates the same way through `diagnostics_checks`.
            // The role is on those records now.
            //
            // A record with only the `explorer` role is a `/tx/` link for a
            // person to tap, not an API. Nothing knows how to probe it, which
            // is `checked: false` rather than a failure.
            let is_rpc = record.kind == "rpc-node";
            let is_link_only = record.kind == "web-link";
            let explicit_probe = record.probe_url.as_deref();
            let rpc_method = is_rpc.then_some(method).flatten();
            if is_link_only && explicit_probe.is_none() {
                out.push(EndpointProbe {
                    chain_name: name.clone(),
                    endpoint: record.endpoint,
                    kind: record.kind.clone(),
                    capabilities: record.capabilities.clone(),
                    checked: false,
                    reachable: false,
                    detail: "an explorer link, not an API".to_string(),
                });
                continue;
            }
            let (checked, reachable, detail) = match (rpc_method, &explicit_probe) {
                (Some(method), _) => {
                    // Confirmed before it accuses. Sweeping every chain fires
                    // well over a hundred requests, and a burst produces
                    // transport errors that have nothing to do with the
                    // endpoint — the first version of this command reported
                    // four BNB seeds, Polygon, Hyperliquid and Ethereum
                    // Classic as dead, and all of them answered when asked
                    // again on their own. A probe that cries wolf is worse
                    // than no probe, because it is the one people learn to
                    // scroll past.
                    let mut verdict = probe_json_rpc(&record.endpoint, method).await;
                    if verdict.is_err() {
                        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                        verdict = probe_json_rpc(&record.endpoint, method).await;
                    }
                    match verdict {
                        Ok(()) => (true, true, method.to_string()),
                        Err(e) => (true, false, e),
                    }
                }
                (None, Some(url)) => {
                    let (ok, detail) = probe_http_endpoint(chain, url).await;
                    (true, ok, detail)
                }
                (None, None) => (false, false, "no probe for this endpoint".to_string()),
            };
            out.push(EndpointProbe {
                chain_name: name.clone(),
                endpoint: record.endpoint,
                kind: record.kind.clone(),
                capabilities: record.capabilities.clone(),
                checked,
                reachable,
                detail,
            });
        }
        Ok(out)
    }

    // ── EVM receipt polling

    // ── Typed send-preview wrappers (fuse fetch + decode in Rust)

    // `send_destination_risk` is its only caller.

    // ── UTXO tx status
}

impl WalletService {
    // ── ENS resolution

    /// Resolve an ENS name to an Ethereum address via the ENS Ideas public API.
    /// Returns the resolved address, or `None` if the name has no registered
    /// address.
    ///
    /// Not exported: `WalletService::resolve_send_destination` is the entry
    /// point, because *when* a typed name is a name to look up is
    /// `Chain::resolves_ens_names` and every lookup reads the provider again. A front end
    /// calling this directly is a front end deciding both.
    pub(crate) async fn resolve_ens_name_typed(
        &self,
        name: String,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let eps = self.endpoints_for("ethereum").await;
        let client = EvmClient::new(eps, 1);
        let address = client.resolve_ens(&name).await?;
        Ok(address.filter(|a| !a.is_empty()))
    }

    // Not exported: the pending-status poll is core's own loop now, and it
    // is the only caller. It was an export because a front end drove the
    // loop and asked for each piece.
    /// Fetch confirmation status for a UTXO chain transaction.
    /// Returns a typed record so Swift can read `confirmed`/`block_height`/
    /// `confirmations` fields without bouncing through JSON.
    /// Supported chain_ids: 0 (BTC), 3 (DOGE), 5 (LTC), 6 (BCH), 22 (BSV).
    pub async fn fetch_utxo_tx_status_typed(
        &self,
        chain_id: String,
        txid: String,
    ) -> Result<UtxoTxStatus, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_utxo_tx_status: unsupported chain_id: {chain_id}"
            ))
        })?;
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let status: UtxoTxStatus = match chain.mainnet_counterpart() {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Zcash => {
                let client = ZcashClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::BitcoinGold => {
                let client = BitcoinGoldClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Decred => {
                let client = DecredClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Kaspa => {
                let client = KaspaClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Chain::Dash => {
                let client = DashClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_utxo_tx_status: unsupported chain: {c:?}"
                )))
            }
        };
        Ok(status)
    }

    // Not exported: the pending-status poll is core's own loop now, and it
    // is the only caller. It was an export because a front end drove the
    // loop and asked for each piece.
    /// Where a broadcast EVM transaction has got to.
    ///
    /// `Ok(None)` means the node has no receipt yet, which is what pending
    /// looks like and is not an error. A receipt with `status: "0x0"` is a
    /// transaction that was mined and reverted — confirmed *and* failed — and
    /// the distinction matters, because a history summary can only say the
    /// hash appeared and would show a reverted send as successful.
    ///
    /// There was a `classify_evm_receipt_json` beside this that took the
    /// receipt as a JSON string and re-parsed it for three fields core had
    /// already decoded. It had no callers; the projection is direct now.
    pub async fn evm_transaction_status(
        &self,
        chain_id: String,
        tx_hash: String,
    ) -> Result<Option<crate::send::flow::EvmReceiptClassification>, SpectraBridgeError> {
        let chain = chain_for_evm_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());
        let receipt = client
            .fetch_receipt(&tx_hash)
            .await
            .map_err(SpectraBridgeError::from)?;
        Ok(
            receipt.map(|receipt| crate::send::flow::EvmReceiptClassification {
                is_confirmed: receipt.is_confirmed,
                is_failed: receipt.is_failed,
                block_number: receipt.block_number.map(|n| n as i64),
            }),
        )
    }

    // Internal JSON-returning helpers (not exported to Swift — the typed
    // wrappers above in the exported impl block call these and translate
    // the JSON into UniFFI records at the boundary).

    // ── EVM paginated history (native + ERC-20 token transfers)
}

/// One JSON-RPC health call. `Err` carries what went wrong, transport or
/// protocol; a JSON-RPC `error` object counts as a failure because an endpoint
/// that answers "you need an API key" is not one this app can use.
async fn probe_json_rpc(endpoint: &str, method: &str) -> Result<(), String> {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": [] });
    let value = crate::fetch::http::HttpClient::shared()
        .post_json::<serde_json::Value, serde_json::Value>(
            endpoint,
            &body,
            crate::fetch::http::RetryProfile::Diagnostics,
        )
        .await?;
    match value.get("error") {
        Some(err) => Err(err.to_string()),
        None => Ok(()),
    }
}

// ── Fetch dispatch ────────────────────────────────────────────────────────
// Three free functions replace the old ChainClient enum. Each builds the
// right client inline and runs the fetch — no enum intermediary.
// Adding a chain means one new arm per function.

#[cfg(test)]
mod history_page_failures {
    use super::*;
    #[tokio::test]
    async fn history_page_without_required_provider_credentials_is_an_error() {
        let service = WalletService::new_typed(vec![]).unwrap();
        // BSC's registry source requires an explorer key. This fails offline,
        // before HTTP, and must not masquerade as an empty successful page.
        let result = service
            .fetch_evm_history_page(Chain::BnbChain.str_id().into(), "from".into(), vec![], 2, 7)
            .await;
        assert!(result.unwrap_err().to_string().contains("key"));
    }
}

async fn probe_http_endpoint(chain: Chain, url: &str) -> (bool, String) {
    use crate::fetch::http::{http_request, HttpHeader, HttpRetryProfile};
    let body = chain.http_health_post_body();
    let method = if body.is_some() { "POST" } else { "GET" };
    let response = http_request(
        method.into(),
        url.into(),
        if body.is_some() {
            vec![HttpHeader {
                name: "Content-Type".into(),
                value: "application/json".into(),
            }]
        } else {
            vec![]
        },
        body.map(|s| s.as_bytes().to_vec()),
        HttpRetryProfile::Diagnostics,
    )
    .await;
    match response {
        Ok(response) => {
            let success = (200..300).contains(&response.status_code);
            let valid = body.is_none()
                || serde_json::from_slice::<serde_json::Value>(&response.body)
                    .ok()
                    .and_then(|v| {
                        v.get("network_identifiers")
                            .and_then(|v| v.as_array())
                            .map(|a| !a.is_empty())
                    })
                    .unwrap_or(false);
            (
                success && valid,
                format!(
                    "{method} HTTP {}{}",
                    response.status_code,
                    if success && !valid {
                        " (missing Rosetta networks)"
                    } else {
                        ""
                    }
                ),
            )
        }
        Err(error) => (false, format!("{method}: {error}")),
    }
}

#[cfg(test)]
mod http_probe_regressions {
    use super::*;
    use wiremock::matchers::{body_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    #[tokio::test]
    async fn rosetta_posts_metadata_and_requires_a_network() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(body_json(serde_json::json!({"metadata":{}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"network_identifiers":[{"blockchain":"Internet Computer","network":"test"}]})))
            .expect(1).mount(&server).await;
        assert!(probe_http_endpoint(Chain::Icp, &server.uri()).await.0);
        server.reset().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"network_identifiers":[]})),
            )
            .mount(&server)
            .await;
        assert!(!probe_http_endpoint(Chain::Icp, &server.uri()).await.0);
    }
    #[tokio::test]
    async fn http_denials_are_reported_with_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let (ok, detail) = probe_http_endpoint(Chain::Zcash, &server.uri()).await;
        assert!(!ok);
        assert!(detail.contains("403"), "{detail}");
    }
}
