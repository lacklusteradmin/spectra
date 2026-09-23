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

    /// Run read-only protocol checks for every API on this concrete network.
    /// Explorer links remain unchecked; a pass never promises broadcast support.
    pub async fn probe_chain_endpoints(
        &self,
        chain_id: String,
    ) -> Result<Vec<EndpointProbe>, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        let name = chain.chain_display_name().to_string();
        let mut records: Vec<_> = self
            .endpoint_directory()
            .await?
            .into_iter()
            .filter(|entry| entry.record.chain_id == chain_id)
            .map(|entry| entry.record)
            .collect();

        if let Some(api) = chain.endpoint_api(EndpointSlot::Primary) {
            for endpoint in self.configured_endpoint_urls(&chain_id).await.iter() {
                if records.iter().any(|r| &r.endpoint == endpoint) {
                    continue;
                }
                records.push(crate::AppCoreEndpointRecord {
                    id: format!("configured:{endpoint}"),
                    api: Some(api),
                    chain_id: chain_id.clone(),
                    endpoint: endpoint.clone(),
                    capabilities: self
                        .endpoints
                        .read()
                        .await
                        .capabilities
                        .get(&chain_id)
                        .cloned()
                        .unwrap_or_default(),
                    probe_url: None,
                    explorer_label: None,
                    tx_suffix: String::new(),
                });
            }
        }
        let mut out = Vec::with_capacity(records.len());
        for record in records {
            let (checked, reachable, detail) = super::endpoint_health::probe(chain, &record).await;
            out.push(EndpointProbe {
                api: record.api,
                chain_id: chain_id.clone(),
                chain_name: name.clone(),
                endpoint: record.endpoint,
                capabilities: record.capabilities.clone(),
                checked,
                reachable,
                detail,
            });
        }
        Ok(out)
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
        let eps = self.endpoints_for("ethereum", &["verification"]).await;
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
    pub async fn fetch_utxo_tx_status(
        &self,
        chain_id: String,
        txid: String,
    ) -> Result<UtxoTxStatus, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_utxo_tx_status: unsupported chain_id: {chain_id}"
            ))
        })?;
        let (api, endpoints) = self.fetch_endpoints(chain, &["verification"]).await?;
        use crate::EndpointApi as Api;
        let status: UtxoTxStatus = match api {
            Api::Esplora => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Api::Blockcypher => {
                let client = DogecoinClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }

            Api::Blockbook => {
                let client = BlockbookClient::new(endpoints, chain);
                client.fetch_tx_status(&txid).await?
            }
            Api::Whatsonchain => {
                let client = BitcoinSvClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }

            Api::Insight => {
                let client = DecredClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }
            Api::KaspaRest => {
                let client = KaspaClient::new(endpoints);
                client.fetch_tx_status(&txid).await?
            }

            c => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_utxo_tx_status: unsupported API: {c:?}"
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
        let chain = evm_network_for_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id(), &["verification"]).await;
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
        chain_id: String,
        tx_hash: String,
    ) -> Result<u64, SpectraBridgeError> {
        let chain = evm_network_for_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id(), &["verification"]).await;
        let client = EvmClient::new(eps, chain.evm_chain_id()?);
        client.fetch_tx_nonce(&tx_hash).await.map_err(Into::into)
    }
}

impl WalletService {
    pub(crate) async fn fetch_evm_has_contract_code(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<bool, SpectraBridgeError> {
        let chain = evm_network_for_id(&chain_id)?;
        let eps = self.endpoints_for(chain.str_id(), &["verification"]).await;
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
            .fetch_evm_history_page(Chain::BnbChain.str_id().into(), "from".into(), vec![], 2, 7)
            .await;
        assert!(result.unwrap_err().to_string().contains("no explorer"));
    }
}
