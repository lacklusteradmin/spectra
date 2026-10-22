//! Broadcast signed transactions and answer the id each node gives back.
use super::*;
use crate::send::payload::SignedPayload;

impl WalletService {
    /// Submit `payload` to every broadcast endpoint of `chain` and answer the
    /// transaction id of the first, in endpoint order, that accepted it.
    pub(crate) async fn broadcast_raw(
        &self,
        chain: crate::registry::Chain,
        payload: SignedPayload,
    ) -> Result<String, SpectraBridgeError> {
        // Every endpoint gets the payload, and every submission runs to its
        // end: one acceptance does not cut another short.
        let endpoints = self
            .chain_endpoints(chain, &[EndpointCapability::Broadcast])
            .await;
        let payload = &payload;
        let results = futures::future::join_all(endpoints.into_iter().map(|endpoint| async move {
            self.validate_broadcast_endpoint(chain, &endpoint.url)
                .await?;
            self.broadcast_at(chain, endpoint.api, Arc::new(vec![endpoint.url]), payload)
                .await
        }))
        .await;
        let mut last_err = SpectraBridgeError::failure("no endpoints configured");
        for result in results {
            match result {
                Ok(txid) => return Ok(txid),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Submit `payload` through `api` and answer the transaction id the node
    /// accepted it under: a hash, a Solana signature, a Sui digest or a TON
    /// message hash. A payload in another form than `api` takes is refused
    /// before anything is sent, and a node that names no id has not shown it
    /// accepted.
    pub(super) async fn broadcast_at(
        &self,
        chain: Chain,
        api: crate::EndpointApi,
        eps: Arc<Vec<String>>,
        payload: &SignedPayload,
    ) -> Result<String, SpectraBridgeError> {
        use crate::EndpointApi as Api;
        use SignedPayload::{Base64, Hex, Json};
        let txid = match (api, payload) {
            (api, Hex(raw)) if api.is_utxo_indexer() => {
                let endpoints = eps
                    .iter()
                    .map(|url| crate::Endpoint {
                        api,
                        url: url.clone(),
                    })
                    .collect();
                crate::api::utxo::UtxoClient::new(chain, endpoints)
                    .broadcast(raw)
                    .await?
            }
            (Api::Lightwalletd, Hex(raw)) => {
                let raw = hex::decode(raw)?;
                let network = chain.zcash_network()?;
                let txid = super::zcash_shielded::zcash_txid(&network, &raw)?;
                crate::api::lightwalletd::LightwalletdClient::new(eps)
                    .session(chain)
                    .await?
                    .submit(raw)
                    .await?;
                txid
            }
            (Api::SolanaJsonRpc, Base64(tx)) => SolanaClient::new(eps).broadcast_raw(tx).await?,
            (Api::TronHttp, Json(tx)) => TronHttpClient::new(eps).broadcast_raw(tx).await?,
            (Api::EvmJsonRpc, Hex(raw)) => {
                EvmClient::new(eps, chain.evm_chain_id()?)
                    .broadcast_raw(raw)
                    .await?
            }
            (Api::XrplJsonRpc, Hex(blob)) => XrplClient::new(eps).submit_signed_blob(blob).await?,
            (Api::Horizon, Base64(xdr)) => HorizonClient::new(eps).submit_envelope_b64(xdr).await?,
            (Api::Koios, Hex(cbor)) => KoiosClient::new(eps).submit_tx(cbor).await?,
            (Api::SubstrateJsonRpc, Hex(extrinsic)) => {
                SubstrateClient::new(eps)
                    .submit_extrinsic_hex(extrinsic)
                    .await?
            }
            (
                Api::SuiJsonRpc,
                SignedPayload::Sui {
                    transaction,
                    signature,
                },
            ) => {
                SuiClient::new(eps)
                    .execute_signed_tx(transaction, signature)
                    .await?
            }
            (Api::AptosRest, Json(body)) => AptosClient::new(eps).submit_signed_body(body).await?,
            // A multi-key transaction travels as BCS.
            (Api::AptosRest, Hex(bcs)) => {
                AptosClient::new(eps)
                    .submit_signed_bcs(&hex::decode(bcs)?)
                    .await?
            }
            (Api::ToncenterV2, Base64(boc)) => ToncenterV2Client::new(eps).send_boc(boc).await?,
            (Api::NearJsonRpc, Base64(tx)) => {
                NearClient::new(eps).broadcast_signed_tx_b64(tx).await?
            }
            (Api::IcpRosetta, Json(body)) => {
                IcpClient::new(eps).submit_signed_transaction(body).await?
            }
            (Api::MoneroDaemonRpc, Hex(raw)) => {
                use ::monero_wallet::interface::PublishTransaction;
                let bytes = hex::decode(raw)?;
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
                hex::encode(tx.hash())
            }
            (Api::KaspaRest, Json(body)) => {
                KaspaClient::new(eps)
                    .broadcast_tx_body(serde_json::from_str(body)?)
                    .await?
            }
            (Api::Insight, Hex(raw)) => InsightClient::new(eps).broadcast_raw_tx(raw).await?,
            (api, _) => {
                return Err(SpectraBridgeError::failure(format!(
                    "A {} endpoint does not take this signed transaction",
                    api.as_str()
                )));
            }
        };
        if txid.trim().is_empty() {
            return Err(SpectraBridgeError::failure(
                "Node returned no transaction identifier",
            ));
        }
        Ok(txid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    /// A node that answers a submission without naming the transaction has
    /// not shown it accepted one.
    #[tokio::test]
    async fn an_answer_naming_no_transaction_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/transactions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        let error = WalletService::new_catalog()
            .unwrap()
            .broadcast_at(
                Chain::Stellar,
                crate::EndpointApi::Horizon,
                Arc::new(vec![server.uri()]),
                &SignedPayload::Base64("AAAA".into()),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Node returned no transaction identifier"),
            "{error}"
        );
    }

    /// A payload in another form than the endpoint's API takes never
    /// reaches the node.
    #[tokio::test]
    async fn a_payload_in_another_form_is_refused_unsent() {
        let server = MockServer::start().await;
        let error = WalletService::new_catalog()
            .unwrap()
            .broadcast_at(
                Chain::Ethereum,
                crate::EndpointApi::EvmJsonRpc,
                Arc::new(vec![server.uri()]),
                &SignedPayload::Base64("AAAA".into()),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("A evm-json-rpc endpoint does not take this signed transaction"),
            "{error}"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
