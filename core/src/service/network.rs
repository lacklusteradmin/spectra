//! Endpoint health, contract probes and transaction status reads.
use super::*;
use crate::api::endpoint::EndpointApi;

/// One endpoint and whether it answered.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct EndpointProbe {
    pub api: EndpointApi,
    pub chain_id: crate::registry::Chain,
    pub endpoint: String,
    /// Operations the endpoint declares.
    pub capabilities: Vec<EndpointCapability>,
    /// False when nothing knows how to probe this endpoint. Not a pass.
    pub checked: bool,
    pub reachable: bool,
    pub detail: String,
}

pub(super) fn evm_network(chain: Chain) -> Result<Chain, SpectraBridgeError> {
    if chain.is_evm() {
        Ok(chain)
    } else {
        Err(SpectraBridgeError::failure(format!(
            "unsupported EVM chain_id: {chain}"
        )))
    }
}

#[cfg(test)]
#[path = "tests/network.rs"]
mod tests;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    // UniFFI exports every method of a `#[uniffi::export]` impl block,
    // whatever its visibility, so internal helpers live in plain
    // `impl WalletService` blocks of their own modules.

    /// Run read-only protocol checks for every API on `chain`. A pass never
    /// promises broadcast support.
    pub async fn probe_chain_endpoints(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<Vec<EndpointProbe>, SpectraBridgeError> {
        let this = self.clone();
        crate::service::worker::run(async move {
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
                    records.push(crate::catalog::endpoints::EndpointRecord {
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
                let endpoint = crate::api::endpoint::Endpoint {
                    api: record.api,
                    url: record.endpoint.clone(),
                };
                let (checked, reachable, detail) =
                    crate::api::health::probe(chain, &endpoint, &record.capabilities).await;
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
            this.diagnostic_runs.record_endpoints(chain, out.clone());
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
        use EndpointApi as Api;
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
                cost: if chain.evm_rollup_fee_model().is_none() {
                    crate::store::EvmReceiptCost::from_receipt(
                        receipt.gas_used.as_deref(),
                        receipt.effective_gas_price_wei.as_deref(),
                        chain.native_decimals(),
                    )
                } else {
                    crate::store::EvmReceiptCost::from_rollup_receipt(
                        receipt.gas_used.as_deref(),
                        receipt.effective_gas_price_wei.as_deref(),
                        receipt.l1_fee_wei.as_deref(),
                        receipt.operator_fee_wei.as_deref(),
                        chain.native_decimals(),
                    )
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
