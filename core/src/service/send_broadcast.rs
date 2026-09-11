//! Rebroadcast already-signed transactions.
use super::*;
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Typed wrapper around `broadcast_raw`: runs the broadcast then extracts
    /// the named field (typically `"txid"` or `"digest"`) from the result JSON.
    /// Returns the field value as a string, or an empty string when missing.
    pub async fn broadcast_raw_extract(
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
        let eps = self.endpoints_for(chain.str_id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let txid = client.broadcast_raw_tx(&payload).await?;
                Ok(json!({ "txid": txid }).to_string())
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Solana => {
                let client = SolanaClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Tron => {
                let client = TronClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(eps, c.evm_chain_id());
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Xrp => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let blob = val["tx_blob_hex"]
                    .as_str()
                    .ok_or("broadcast_raw xrp: missing tx_blob_hex")?
                    .to_string();
                let client = XrpClient::new(eps);
                let res = client.submit_signed_blob(&blob).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Stellar => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let xdr = val["signed_xdr_b64"]
                    .as_str()
                    .ok_or("broadcast_raw stellar: missing signed_xdr_b64")?
                    .to_string();
                let client = StellarClient::new(eps);
                let res = client.submit_envelope_b64(&xdr).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Cardano => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"]
                    .as_str()
                    .ok_or("broadcast_raw cardano: missing cbor_hex")?
                    .to_string();
                let api_key = self.api_key_for(chain.str_id()).await.unwrap_or_default();
                let client = CardanoClient::new(eps, api_key);
                let res = client.submit_tx(&cbor).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Polkadot => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let ext_hex = val["extrinsic_hex"]
                    .as_str()
                    .ok_or("broadcast_raw polkadot: missing extrinsic_hex")?
                    .to_string();
                let subscan = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = PolkadotClient::new(eps, subscan, api_key);
                let res = client.submit_extrinsic_hex(&ext_hex).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Sui => {
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
            Chain::Aptos => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let body_json = val["signed_body_json"]
                    .as_str()
                    .ok_or("broadcast_raw aptos: missing signed_body_json")?
                    .to_string();
                let client = AptosClient::new(eps);
                let res = client.submit_signed_body(&body_json).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Ton => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"]
                    .as_str()
                    .ok_or("broadcast_raw ton: missing boc_b64")?
                    .to_string();
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = TonClient::new(eps, api_key);
                let res = client.send_boc(&boc).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Near => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_b64 = val["signed_tx_b64"]
                    .as_str()
                    .ok_or("broadcast_raw near: missing signed_tx_b64")?
                    .to_string();
                let client = NearClient::new(eps);
                let res = client.broadcast_signed_tx_b64(&tx_b64).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Icp => Err(SpectraBridgeError::from(
                "ICP rebroadcast is not supported".to_string(),
            )),
            c => Err(SpectraBridgeError::from(format!(
                "broadcast_raw: chain {c:?} not supported"
            ))),
        }
    }
}
