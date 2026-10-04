//! Endpoint health, contract probes and transaction status reads.
use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    // `fetch_history` lives in the plain-impl block below (JSON shuttle —
    // kept internal, not exported to Swift).

    // `broadcast_at` lives in the plain `impl WalletService` block
    // in `service/send_broadcast.rs`. UniFFI exports every method of a `#[uniffi::export]`
    // impl block regardless of `pub(crate)` visibility, so chain-dispatch
    // internal protocol helpers must be outside this block.

    // `execute_send` lives in `service/send_execution.rs`.

    // `bitcoin_xpub_balance` lives in the plain-impl block below: it returns a
    // typed `HdXpubBalance` to Rust callers only, not across the FFI.

    // `fetch_evm_history_page` lives in the plain-impl block below: it is
    // called by `history_refresh`, not across the FFI.

    // `fetch_utxo_fee_preview_json` and `broadcast_raw` live in the plain-impl
    // block below (JSON shuttles — kept internal, not exported to Swift).

    // `fetch_evm_send_preview_json` / `fetch_tron_send_preview_json` /
    // `fetch_simple_chain_send_preview_json` live in the plain-impl block below
    // (JSON shuttles — kept internal, not exported to Swift). Their typed
    // wrappers below call into those internal helpers.

    /// Run read-only protocol checks for every API on `chain`. A pass never
    /// promises broadcast support.
    pub async fn probe_chain_endpoints(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<Vec<EndpointProbe>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let mut records: Vec<_> = this
                .endpoint_directory()
                .await?
                .into_iter()
                .filter(|entry| entry.record.chain_id == chain)
                .map(|entry| entry.record)
                .collect();

            if let Some(api) = chain.default_api() {
                for endpoint in this.configured_endpoint_urls(chain).await.iter() {
                    if records.iter().any(|r| &r.endpoint == endpoint) {
                        continue;
                    }
                    records.push(crate::endpoints::EndpointRecord {
                        id: format!("configured:{endpoint}"),
                        api,
                        chain_id: chain,
                        endpoint: endpoint.clone(),
                        capabilities: this
                            .endpoints
                            .read()
                            .await
                            .capabilities
                            .get(&chain)
                            .cloned()
                            .unwrap_or_default(),
                    });
                }
            }
            let mut out = Vec::with_capacity(records.len());
            for record in records {
                let (checked, reachable, detail) =
                    super::endpoint_health::probe(chain, &record).await;
                out.push(EndpointProbe {
                    api: record.api,
                    chain_id: chain,
                    endpoint: record.endpoint,
                    capabilities: record.capabilities.clone(),
                    checked,
                    reachable,
                    detail,
                });
            }
            crate::diagnostics::diagnostics_record_endpoints(chain, out.clone());
            Ok(out)
        })
        .await
    }
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
    pub(crate) async fn resolve_ens_name(
        &self,
        name: String,
    ) -> Result<Option<String>, SpectraBridgeError> {
        let eps = self
            .endpoints_for(
                crate::registry::Chain::Ethereum,
                &[EndpointCapability::Verification],
            )
            .await;
        let client = EvmClient::new(eps, 1);
        let address = client.resolve_ens(&name).await?;
        Ok(address.filter(|a| !a.is_empty()))
    }

    /// Fetch confirmation status for a UTXO chain transaction.
    ///
    /// Not exported: the pending-status poll is core's own loop, and its only
    /// caller.
    pub async fn fetch_utxo_tx_status(
        &self,
        chain: crate::registry::Chain,
        txid: String,
    ) -> Result<UtxoTxStatus, SpectraBridgeError> {
        if chain.uses_utxo_client() {
            return Ok(self
                .utxo_client(chain, &[EndpointCapability::Verification])
                .await
                .fetch_tx_status(&txid)
                .await?);
        }
        let (api, endpoints) = self
            .fetch_endpoints(chain, &[EndpointCapability::Verification])
            .await?;
        use crate::EndpointApi as Api;
        let status: UtxoTxStatus = match api {
            Api::Insight => {
                let client = InsightClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Api::KaspaRest => {
                let client = KaspaClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }

            c => {
                return Err(SpectraBridgeError::failure(format!(
                    "fetch_utxo_tx_status: unsupported API: {c:?}"
                )));
            }
        };
        Ok(status)
    }

    /// Where a broadcast EVM transaction has got to.
    ///
    /// `Ok(None)` means the node has no receipt yet, which is what pending
    /// looks like and is not an error. A receipt with `status: "0x0"` is a
    /// transaction that was mined and reverted — confirmed *and* failed — and
    /// the distinction matters, because a history summary can only say the
    /// hash appeared and would show a reverted send as successful.
    ///
    /// Not exported: the pending-status poll is core's own loop, and its only
    /// caller.
    pub async fn evm_transaction_status(
        &self,
        chain_id: crate::registry::Chain,
        tx_hash: String,
    ) -> Result<Option<crate::send::flow::EvmReceiptClassification>, SpectraBridgeError> {
        let chain = evm_network(chain_id)?;
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let client = EvmClient::new(eps, chain.evm_chain_id()?);
        let receipt = client
            .fetch_receipt(&tx_hash)
            .await
            .map_err(SpectraBridgeError::from)?;
        Ok(
            receipt.map(|receipt| crate::send::flow::EvmReceiptClassification {
                is_confirmed: receipt.is_confirmed,
                is_failed: receipt.is_failed,
                block_number: receipt.block_number.map(|n| n as i64),
                // Execution gas alone is not the complete actual fee on OP
                // Stack. Omit cost until historical L1/operator charges are
                // decoded, rather than display this subtotal as Network Fee.
                cost: if chain.evm_rollup_fee_model().is_none() {
                    crate::store::EvmReceiptCost::from_receipt(
                        receipt.gas_used.as_deref(),
                        receipt.effective_gas_price_wei.as_deref(),
                        chain.native_decimals(),
                    )
                } else {
                    None
                },
            }),
        )
    }

    // Internal JSON-returning helpers (not exported to Swift — the typed
    // wrappers above in the exported impl block call these and translate
    // the JSON into UniFFI records at the boundary).

    // ── EVM paginated history (native + ERC-20 token transfers)
}

impl WalletService {
    /// Read the live nonce for a core-owned replacement draft.
    pub async fn fetch_evm_tx_nonce(
        &self,
        chain_id: crate::registry::Chain,
        tx_hash: String,
    ) -> Result<u64, SpectraBridgeError> {
        let chain = evm_network(chain_id)?;
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let client = EvmClient::new(eps, chain.evm_chain_id()?);
        client.fetch_tx_nonce(&tx_hash).await.map_err(Into::into)
    }
}

impl WalletService {
    pub(crate) async fn fetch_evm_has_contract_code(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let chain = evm_network(chain_id)?;
        let eps = self
            .endpoints_for(chain, &[EndpointCapability::Verification])
            .await;
        let client = EvmClient::new(eps, chain.evm_chain_id()?);
        let code = client.fetch_code(&address).await?;
        Ok(crate::send::flow::evm_has_contract_code(code))
    }
}

#[cfg(test)]
mod history_page_failures {
    use super::*;
    #[tokio::test]
    async fn history_page_without_a_configured_source_is_an_error() {
        let service = WalletService::new(vec![]).unwrap();
        // BSC has no configured keyless history source. This fails offline,
        // before HTTP, and must not masquerade as an empty successful page.
        let result = service
            .fetch_evm_history_page(Chain::BnbChain, "from".into(), vec![], 2, 7)
            .await;
        assert!(result.unwrap_err().to_string().contains("no explorer"));
    }
}

#[cfg(test)]
mod receipt_cost_completeness {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::body_partial_json};

    #[tokio::test]
    async fn world_chain_receipt_omits_partial_cost_without_losing_outcome() {
        for chain in [Chain::WorldChain, Chain::Ethereum] {
            for (status, is_failed) in [("0x1", false), ("0x0", true)] {
                let server = MockServer::start().await;
                Mock::given(body_partial_json(
                    json!({"method": "eth_getTransactionReceipt"}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "jsonrpc": "2.0", "id": 1,
                    "result": {
                        "status": status, "blockNumber": "0x7",
                        "gasUsed": "0x5208", "effectiveGasPrice": "0x1"
                    }
                })))
                .expect(if chain == Chain::WorldChain { 2 } else { 1 })
                .mount(&server)
                .await;
                let service = WalletService::new(vec![ChainEndpoints {
                    chain_id: chain,
                    capabilities: vec![EndpointCapability::Verification],
                    endpoints: vec![server.uri()],
                }])
                .unwrap();
                let receipt = service
                    .evm_transaction_status(chain, format!("0x{}", "11".repeat(32)))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(receipt.is_confirmed);
                assert_eq!(receipt.is_failed, is_failed);
                assert_eq!(receipt.block_number, Some(7));
                if chain == Chain::WorldChain {
                    assert!(receipt.cost.is_none());
                    let database = std::env::temp_dir()
                        .join(format!(
                            "op-receipt-{}.sqlite",
                            crate::store::new_event_id()
                        ))
                        .to_string_lossy()
                        .into_owned();
                    service.open_state(database).await.unwrap();
                    let old: crate::store::persistence_models::CorePersistedTransactionRecord =
                        serde_json::from_value(json!({
                            "id": "pending", "walletId": "w", "walletName": "Wallet",
                            "kind": "send", "status": "pending", "chainId": chain,
                            "symbol": "ETH", "assetDisplayName": "Ethereum", "amount": "1",
                            "address": "recipient", "createdAtUnix": 0.0,
                            "transactionHash": format!("0x{}", "11".repeat(32)),
                            "receiptGasUsed": "21000", "receiptEffectiveGasPriceGwei": "0.000000001",
                            "receiptNetworkFee": "0.000000000000021",
                            "confirmedNetworkFee": "0.000000000000021"
                        }))
                        .unwrap();
                    service
                        .upsert_history_records(vec![
                            crate::wallet_db::history_record_from_payload(old),
                        ])
                        .await
                        .unwrap();
                    let changes = service.poll_pending_transactions(chain).await.unwrap();
                    assert_eq!(changes.len(), 1);
                    let polled = service.transactions().await.unwrap().remove(0);
                    service
                        .apply_resolved_pending_statuses(
                            chain,
                            vec![crate::store::ResolvedPendingStatus {
                                id: "pending".into(),
                                status: if is_failed { "failed" } else { "confirmed" }.into(),
                                confirmations: None,
                                receipt_block_number: Some(7),
                                evm_receipt_cost: crate::store::EvmReceiptCost::from_receipt(
                                    Some("21000"),
                                    Some("1"),
                                    18,
                                ),
                            }],
                        )
                        .await
                        .unwrap();
                    let supplied = service.transactions().await.unwrap().remove(0);
                    use crate::store::wallet_domain::CoreTransactionStatus;
                    for stored in [polled, supplied] {
                        assert_eq!(
                            stored.status,
                            if is_failed {
                                CoreTransactionStatus::Failed
                            } else {
                                CoreTransactionStatus::Confirmed
                            }
                        );
                        assert_eq!(stored.receipt_block_number, Some(7));
                        assert!(stored.receipt_gas_used.is_none());
                        assert!(stored.receipt_effective_gas_price_gwei.is_none());
                        assert!(stored.receipt_network_fee.is_none());
                        assert!(stored.confirmed_network_fee.is_none());
                    }
                } else {
                    let cost = receipt.cost.unwrap();
                    assert_eq!(cost.gas_used, "21000");
                    assert_eq!(cost.effective_gas_price_gwei, "0.000000001");
                    assert_eq!(cost.network_fee, "0.000000000000021");
                }
                server.verify().await;
            }
        }
    }
}
