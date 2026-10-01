//! Rebroadcast already-signed transactions.
use super::*;
impl WalletService {
    /// Typed wrapper around `broadcast_raw`: runs the broadcast then extracts
    /// the named field (typically `"txid"` or `"digest"`) from the result JSON.
    /// Returns the field value as a string, or an empty string when missing.
    pub(crate) async fn broadcast_raw_extract(
        &self,
        chain_id: crate::registry::Chain,
        payload: String,
        result_field: String,
    ) -> Result<String, SpectraBridgeError> {
        let json = self.broadcast_raw(chain_id, payload).await?;
        Ok(crate::send::preview_decode::extract_json_string_field(
            json,
            result_field,
        ))
    }
}

impl WalletService {
    pub(crate) async fn broadcast_raw(
        &self,
        chain: crate::registry::Chain,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        // Every endpoint gets the payload, and every submission runs to its
        // end: one acceptance does not cut another short.
        let endpoints = self
            .chain_endpoints(chain, &[EndpointCapability::Broadcast])
            .await;
        let results = futures::future::join_all(endpoints.into_iter().map(|endpoint| {
            let payload = payload.clone();
            async move {
                self.validate_broadcast_endpoint(chain, &endpoint.url)
                    .await?;
                self.broadcast_at(chain, endpoint.api, Arc::new(vec![endpoint.url]), payload)
                    .await
            }
        }))
        .await;
        let mut last_err = SpectraBridgeError::failure("no endpoints configured");
        for result in results {
            match result {
                Ok(response) => return Ok(response),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub(super) async fn broadcast_at(
        &self,
        chain: Chain,
        api: crate::EndpointApi,
        eps: Arc<Vec<String>>,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        use crate::EndpointApi as Api;
        match api {
            api if api.is_utxo_indexer() => {
                let endpoints = eps
                    .iter()
                    .map(|url| crate::Endpoint {
                        api,
                        url: url.clone(),
                    })
                    .collect();
                let txid = crate::api::utxo::UtxoClient::new(chain, endpoints)
                    .broadcast(&payload)
                    .await?;
                Ok(json!({ "txid": txid }).to_string())
            }
            Api::SolanaJsonRpc => {
                let client = SolanaClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::TronHttp => {
                let client = TronHttpClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::EvmJsonRpc => {
                let client = EvmClient::new(eps, chain.evm_chain_id()?);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::XrplJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let blob = val["tx_blob_hex"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw xrp: missing tx_blob_hex")
                    })?
                    .to_string();
                let client = XrplClient::new(eps);
                let res = client.submit_signed_blob(&blob).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::Horizon => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let xdr = val["signed_xdr_b64"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw stellar: missing signed_xdr_b64")
                    })?
                    .to_string();
                let client = HorizonClient::new(eps);
                let res = client.submit_envelope_b64(&xdr).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::Koios => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw cardano: missing cbor_hex")
                    })?
                    .to_string();
                let client = KoiosClient::new(eps);
                let res = client.submit_tx(&cbor).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::SubstrateJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let hex = val["extrinsic_hex"].as_str().ok_or_else(|| {
                    SpectraBridgeError::failure("broadcast_raw substrate: missing extrinsic_hex")
                })?;
                let client = SubstrateClient::new(eps);
                Ok(serde_json::to_string(
                    &client.submit_extrinsic_hex(hex).await?,
                )?)
            }
            Api::SuiJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_bytes = val["tx_bytes_b64"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw sui: missing tx_bytes_b64")
                    })?
                    .to_string();
                let sig = val["sig_b64"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw sui: missing sig_b64")
                    })?
                    .to_string();
                let client = SuiClient::new(eps);
                let res = client.execute_signed_tx(&tx_bytes, &sig).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::AptosRest => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let body_json = val["signed_body_json"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw aptos: missing signed_body_json")
                    })?
                    .to_string();
                let client = AptosClient::new(eps);
                let res = client.submit_signed_body(&body_json).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::ToncenterV2 => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw ton: missing boc_b64")
                    })?
                    .to_string();
                let client = ToncenterV2Client::new(eps);
                let res = client.send_boc(&boc).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::NearJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_b64 = val["signed_tx_b64"]
                    .as_str()
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("broadcast_raw near: missing signed_tx_b64")
                    })?
                    .to_string();
                let client = NearClient::new(eps);
                let res = client.broadcast_signed_tx_b64(&tx_b64).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::IcpRosetta => {
                let client = IcpClient::new(eps);
                Ok(serde_json::to_string(
                    &client.submit_signed_transaction(&payload).await?,
                )?)
            }
            Api::MoneroDaemonRpc => {
                use ::monero_wallet::interface::PublishTransaction;
                let bytes = hex::decode(&payload)?;
                let mut reader = bytes.as_slice();
                let tx = ::monero_wallet::transaction::Transaction::read(&mut reader)
                    .map_err(SpectraBridgeError::failure)?;
                if !reader.is_empty() {
                    return Err(SpectraBridgeError::failure(
                        "Trailing data in Monero transaction",
                    ));
                }
                let endpoint = eps.first().ok_or_else(|| {
                    SpectraBridgeError::failure("Missing Monero broadcast endpoint")
                })?;
                let daemon = crate::api::monero_daemon_rpc::daemon(endpoint, chain).await?;
                daemon
                    .publish_transaction(&tx)
                    .await
                    .map_err(SpectraBridgeError::failure)?;
                Ok(json!({"txid":hex::encode(tx.hash())}).to_string())
            }

            Api::KaspaRest => {
                let client = KaspaClient::new(eps);
                Ok(serde_json::to_string(
                    &client
                        .broadcast_tx_body(serde_json::from_str(&payload)?)
                        .await?,
                )?)
            }
            Api::Insight => {
                let client = InsightClient::new(eps);
                Ok(serde_json::to_string(
                    &client.broadcast_raw_tx(&payload).await?,
                )?)
            }

            c => Err(SpectraBridgeError::failure(format!(
                "broadcast_raw: chain {c:?} not supported"
            ))),
        }
    }
}
