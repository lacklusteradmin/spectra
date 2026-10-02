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
                cost: crate::store::EvmReceiptCost::from_receipt(
                    receipt.gas_used.as_deref(),
                    receipt.effective_gas_price_wei.as_deref(),
                    chain.native_decimals(),
                ),
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
