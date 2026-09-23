//! Rebroadcast already-signed transactions.
use super::*;
impl WalletService {
    /// Typed wrapper around `broadcast_raw`: runs the broadcast then extracts
    /// the named field (typically `"txid"` or `"digest"`) from the result JSON.
    /// Returns the field value as a string, or an empty string when missing.
    pub(crate) async fn broadcast_raw_extract(
        &self,
        chain_id: String,
        payload: String,
        result_field: String,
    ) -> Result<String, SpectraBridgeError> {
        let json = self.broadcast_raw(&chain_id, payload).await?;
        Ok(crate::send::preview_decode::extract_json_string_field(
            json,
            result_field,
        ))
    }
}

impl WalletService {
    pub(crate) async fn broadcast_raw(
        &self,
        chain_id: &str,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!("broadcast_raw: chain {chain_id} not supported"))
        })?;
        let (api, eps) = self.fetch_endpoints(chain, &["broadcast"]).await?;
        crate::fetch::http::with_fallback(&eps, |endpoint| {
            let payload = payload.clone();
            async move {
                self.validate_broadcast_endpoint(chain, &endpoint)
                    .await
                    .map_err(|e| e.to_string())?;
                self.broadcast_at(chain, api, Arc::new(vec![endpoint]), payload)
                    .await
                    .map_err(|e| e.to_string())
            }
        })
        .await
        .map_err(Into::into)
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
            Api::Esplora => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let txid = client.broadcast_raw_tx(&payload).await?;
                Ok(json!({ "txid": txid }).to_string())
            }
            Api::Blockcypher => {
                let client = DogecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }

            Api::Blockbook => {
                let client = BlockbookClient::new(eps, chain);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::Whatsonchain => {
                let client = BitcoinSvClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::SolanaJsonRpc => {
                let client = SolanaClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::TronHttp => {
                let client = TronClient::new(eps);
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
                    .ok_or("broadcast_raw xrp: missing tx_blob_hex")?
                    .to_string();
                let client = XrpClient::new(eps);
                let res = client.submit_signed_blob(&blob).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::Horizon => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let xdr = val["signed_xdr_b64"]
                    .as_str()
                    .ok_or("broadcast_raw stellar: missing signed_xdr_b64")?
                    .to_string();
                let client = StellarClient::new(eps);
                let res = client.submit_envelope_b64(&xdr).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::Koios => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"]
                    .as_str()
                    .ok_or("broadcast_raw cardano: missing cbor_hex")?
                    .to_string();
                let client = CardanoClient::new(eps);
                let res = client.submit_tx(&cbor).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::SubstrateJsonRpc if chain.mainnet_counterpart() == Chain::Bittensor => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let hex = val["extrinsic_hex"]
                    .as_str()
                    .ok_or("missing extrinsic_hex")?;
                let client = BittensorClient::new(eps);
                Ok(serde_json::to_string(
                    &client.submit_extrinsic_hex(hex).await?,
                )?)
            }
            Api::SubstrateJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let ext_hex = val["extrinsic_hex"]
                    .as_str()
                    .ok_or("broadcast_raw polkadot: missing extrinsic_hex")?
                    .to_string();
                let client = PolkadotClient::new(eps);
                let res = client.submit_extrinsic_hex(&ext_hex).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::SuiJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_bytes = val["tx_bytes_b64"]
                    .as_str()
                    .ok_or("broadcast_raw sui: missing tx_bytes_b64")?
                    .to_string();
                let sig = val["sig_b64"]
                    .as_str()
                    .ok_or("broadcast_raw sui: missing sig_b64")?
                    .to_string();
                let client = SuiClient::new(eps);
                let res = client.execute_signed_tx(&tx_bytes, &sig).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::AptosRest => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let body_json = val["signed_body_json"]
                    .as_str()
                    .ok_or("broadcast_raw aptos: missing signed_body_json")?
                    .to_string();
                let client = AptosClient::new(eps);
                let res = client.submit_signed_body(&body_json).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::ToncenterV2 => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"]
                    .as_str()
                    .ok_or("broadcast_raw ton: missing boc_b64")?
                    .to_string();
                let client = TonClient::new(eps);
                let res = client.send_boc(&boc).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Api::NearJsonRpc => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_b64 = val["signed_tx_b64"]
                    .as_str()
                    .ok_or("broadcast_raw near: missing signed_tx_b64")?
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
                let bytes = hex::decode(&payload).map_err(|e| e.to_string())?;
                let mut reader = bytes.as_slice();
                let tx = ::monero_wallet::transaction::Transaction::read(&mut reader)
                    .map_err(|e| e.to_string())?;
                if !reader.is_empty() {
                    return Err("Trailing data in Monero transaction".into());
                }
                let endpoint = eps.first().ok_or("Missing Monero broadcast endpoint")?;
                let daemon = crate::send::monero_local::daemon(endpoint, chain).await?;
                daemon
                    .publish_transaction(&tx)
                    .await
                    .map_err(|e| e.to_string())?;
                Ok(json!({"txid":hex::encode(tx.hash())}).to_string())
            }
            Api::MoneroWalletRpc => {
                let client = MoneroClient::new(eps);
                Ok(serde_json::to_string(
                    &client.relay_prepared(&payload).await?,
                )?)
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
                let client = DecredClient::new(eps);
                Ok(serde_json::to_string(
                    &client.broadcast_raw_tx(&payload).await?,
                )?)
            }

            c => Err(SpectraBridgeError::from(format!(
                "broadcast_raw: chain {c:?} not supported"
            ))),
        }
    }
}
