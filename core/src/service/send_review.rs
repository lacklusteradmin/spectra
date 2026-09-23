//! Short-lived, single-use confirmation of owned send inputs and their quote.
//! Restart invalidates a review; it never resumes a send without a new user action.
use super::*;

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct SendReviewInput {
    pub wallet_id: String,
    pub holding_key: String,
    pub amount: String,
    pub destination: String,
    pub overrides: Option<crate::send::ethereum::EvmSendOverridesInput>,
}
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct OwnedSendReview {
    pub id: String,
    pub request: crate::send::SendExecutionRequest,
    pub preview: Option<crate::send::flow::SendPreview>,
    pub warnings: Vec<crate::send::flow::HighRiskSendWarning>,
    pub recipient_warnings: Vec<crate::store::EvmRecipientPreflightWarning>,
    pub requires_self_send_confirmation: bool,
    pub requires_wallet_password: bool,
}
pub(crate) struct ReviewedSend {
    input: String,
    request: crate::send::SendExecutionRequest,
    sender: String,
    created: std::time::Instant,
}
impl ReviewedSend {
    fn validate_input(&self, input: &SendReviewInput) -> Result<(), SpectraBridgeError> {
        if self.created.elapsed().as_secs() >= 120 {
            return Err("Send review expired; review the transaction again".into());
        }
        if self.input != serde_json::to_string(input).map_err(|e| e.to_string())? {
            return Err("Send inputs changed; review the transaction again".into());
        }
        Ok(())
    }
}
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn review_owned_send(
        &self,
        input: SendReviewInput,
    ) -> Result<OwnedSendReview, SpectraBridgeError> {
        if input.overrides.as_ref().is_some_and(|o| {
            o.gas_limit.is_some()
                || o.calldata_hex.is_some()
                || o.access_list_json.is_some()
                || o.sign_only.is_some()
        }) {
            return Err("Owned send review supports fee and nonce edits only".into());
        }
        let mut quote = self
            .quote_owned_send(
                input.wallet_id.clone(),
                input.holding_key.clone(),
                input.amount.clone(),
                input.destination.clone(),
                input.overrides.clone(),
            )
            .await?;
        if let Some(crate::send::flow::SendPreview::Ethereum { preview }) = &mut quote.preview {
            if input.overrides.as_ref().and_then(|o| o.nonce).is_none() {
                let state = self.app_state().await;
                let chain = chain_for_id(&quote.request.chain_id)?;
                let wallet = state
                    .wallets
                    .iter()
                    .find(|w| w.id == input.wallet_id)
                    .ok_or("Wallet removed")?;
                let sender = wallet
                    .address_on(chain)
                    .ok_or("Wallet has no sending address")?;
                preview.nonce = i64::try_from(self.next_send_nonce(chain, sender).await?)
                    .map_err(|_| "Nonce exceeds supported range")?;
            }
            let fees = quote
                .request
                .evm_overrides
                .get_or_insert_with(Default::default);
            fees.nonce = Some(preview.nonce);
            fees.gas_limit = Some(preview.gasLimit);
            fees.custom_fees = Some(crate::send::ethereum::EvmCustomFeeConfiguration {
                max_fee_per_gas_gwei: preview.maxFeePerGasGwei,
                max_priority_fee_per_gas_gwei: preview.maxPriorityFeePerGasGwei,
            });
        }
        let chain = chain_for_id(&quote.request.chain_id)?;
        let resolved = self
            .verify_send_destination(
                quote.request.chain_id.clone(),
                input.destination.clone(),
                quote.request.to_address.clone(),
            )
            .await?;
        let amount = input
            .amount
            .trim()
            .parse::<f64>()
            .map_err(|_| "Invalid amount")?;
        let warnings = self
            .high_risk_send_reasons(
                input.wallet_id.clone(),
                input.holding_key.clone(),
                amount,
                resolved.address.clone(),
                input.destination.clone(),
                resolved.used_ens,
            )
            .await;
        let recipient_warnings = self
            .evm_recipient_preflight(
                input.wallet_id.clone(),
                input.holding_key.clone(),
                resolved.address.clone(),
            )
            .await;
        let self_send = self
            .self_send_confirmation(
                input.wallet_id.clone(),
                input.holding_key.clone(),
                resolved.address,
                amount,
                None,
            )
            .await?;
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == input.wallet_id)
            .ok_or("Wallet removed")?;
        super::send_execution::send_chain_for(&state, &input.wallet_id, chain)?;
        let sender = wallet
            .address_on(chain)
            .ok_or("Wallet has no sending address")?
            .to_owned();
        let requires_wallet_password = self.wallet_secret_state(input.wallet_id.clone())?.is_sealed;
        let id = hex::encode(rand::random::<[u8; 32]>());
        let mut reviews = self.send_reviews.lock().await;
        reviews.retain(|_, r| r.created.elapsed().as_secs() < 120);
        if reviews.len() >= 32 {
            return Err("Too many pending send reviews".into());
        }
        reviews.insert(
            id.clone(),
            ReviewedSend {
                input: serde_json::to_string(&input).map_err(|e| e.to_string())?,
                request: quote.request.clone(),
                sender,
                created: std::time::Instant::now(),
            },
        );
        Ok(OwnedSendReview {
            id,
            request: quote.request,
            preview: quote.preview,
            warnings,
            recipient_warnings,
            requires_self_send_confirmation: self_send.requires_confirmation,
            requires_wallet_password,
        })
    }

    /// The explicit confirmation action consumes the review before any signing.
    /// A failed/uncertain send cannot reuse that confirmation.
    pub async fn execute_owned_send(
        &self,
        review_id: String,
        input: SendReviewInput,
        password: Option<String>,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let mut reviewed = self
            .send_reviews
            .lock()
            .await
            .remove(&review_id)
            .ok_or("Send review missing or already consumed; review again")?;
        reviewed.validate_input(&input)?;
        let state = self.app_state().await;
        let chain = chain_for_id(&reviewed.request.chain_id)?;
        super::send_execution::send_chain_for(&state, &input.wallet_id, chain)?;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == input.wallet_id)
            .ok_or("Wallet removed")?;
        if wallet.address_on(chain) != Some(reviewed.sender.as_str())
            || !wallet
                .holdings
                .iter()
                .any(|h| h.deployment_id() == input.holding_key && h.chain() == Some(chain))
        {
            return Err("Sending identity changed; review again".into());
        }
        let automatic_nonce = input.overrides.as_ref().and_then(|o| o.nonce).is_none();
        let preflight = self
            .send_submit_preflight(
                input.wallet_id,
                input.holding_key,
                reviewed.request.to_address.clone(),
                input.amount,
            )
            .await?;
        if preflight.token_contract_address != reviewed.request.contract_address
            || preflight.token_decimals != reviewed.request.token_decimals
        {
            return Err("Token identity changed; review again".into());
        }
        self.verify_send_destination(
            reviewed.request.chain_id.clone(),
            input.destination,
            reviewed.request.to_address.clone(),
        )
        .await?;
        reviewed.request.password = password;
        self.execute_confirmed_send(reviewed.request, reviewed.sender, automatic_nonce)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> SendReviewInput {
        SendReviewInput {
            wallet_id: "w".into(),
            holding_key: "ethereum:native".into(),
            amount: "1.000000000000000001".into(),
            destination: format!("0x{}", "11".repeat(20)),
            overrides: None,
        }
    }
    fn reviewed(input: &SendReviewInput) -> ReviewedSend {
        ReviewedSend {
            input: serde_json::to_string(input).unwrap(),
            sender: format!("0x{}", "22".repeat(20)),
            created: std::time::Instant::now(),
            request: crate::send::SendExecutionRequest {
                chain_id: "ethereum".into(),
                wallet_id: input.wallet_id.clone(),
                password: None,
                to_address: input.destination.clone(),
                amount_str: input.amount.clone(),
                contract_address: None,
                token_decimals: None,
                fee_rate_svb: None,
                fee_sat: None,
                gas_budget: None,
                fee_amount: None,
                evm_overrides: None,
                monero_priority: None,
                sign_only: false,
            },
        }
    }
    #[test]
    fn confirmation_binds_every_edit_including_sub_float_precision() {
        let original = input();
        let r = reviewed(&original);
        assert!(r.validate_input(&original).is_ok());
        for modified in [
            SendReviewInput {
                wallet_id: "other".into(),
                ..original.clone()
            },
            SendReviewInput {
                holding_key: "ethereum-sepolia:native".into(),
                ..original.clone()
            },
            SendReviewInput {
                amount: "1.000000000000000002".into(),
                ..original.clone()
            },
            SendReviewInput {
                destination: format!("0x{}", "33".repeat(20)),
                ..original.clone()
            },
            SendReviewInput {
                overrides: Some(crate::send::ethereum::EvmSendOverridesInput {
                    nonce: Some(7),
                    ..Default::default()
                }),
                ..original.clone()
            },
        ] {
            assert!(r.validate_input(&modified).is_err());
        }
        let mut expired = reviewed(&original);
        expired.created -= std::time::Duration::from_secs(121);
        assert!(expired
            .validate_input(&original)
            .unwrap_err()
            .to_string()
            .contains("expired"));
    }
    #[tokio::test]
    async fn tampered_confirmations_are_consumed_and_restart_requires_review() {
        let service = WalletService::new(vec![]).unwrap();
        let original = input();
        service
            .send_reviews
            .lock()
            .await
            .insert("review".into(), reviewed(&original));
        let modified = SendReviewInput {
            amount: "2".into(),
            ..original.clone()
        };
        let error = service
            .execute_owned_send("review".into(), modified, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("inputs changed"));
        let error = service
            .execute_owned_send("review".into(), original.clone(), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already consumed"));
        let reopened = WalletService::new(vec![]).unwrap();
        assert!(reopened
            .execute_owned_send("review".into(), original, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("review missing"));
    }
    #[tokio::test]
    async fn reviewed_quote_signs_once_and_stale_nonce_never_broadcasts() {
        for password in [None, Some("sealed-test-password")] {
            use crate::store::{
                secret_backends::InMemorySecretStore, state::WalletState,
                wallet_secrets::store_seed_phrase,
            };
            use serde_json::{json, Value};
            use sha3::Digest;
            use std::sync::atomic::{AtomicU64, Ordering};
            use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
            let server = MockServer::start().await;
            let nonce = Arc::new(AtomicU64::new(7));
            let observed_nonce = nonce.clone();
            Mock::given(any())
                .respond_with(move |r: &Request| {
                    let body: Value = r.body_json().unwrap();
                    let result = match body["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x1"),
                        "eth_getTransactionCount" => {
                            json!(format!("0x{:x}", observed_nonce.load(Ordering::SeqCst)))
                        }
                        "eth_getBalance" => json!("0x8ac7230489e80000"),
                        "eth_estimateGas" => json!("0x5208"),
                        "eth_getCode" => json!("0x"),
                        "eth_feeHistory" => {
                            json!({"baseFeePerGas":["0x3b9aca00"], "reward":[["0x77359400"]]})
                        }
                        "eth_sendRawTransaction" => {
                            let raw = body["params"][0].as_str().unwrap();
                            json!(format!(
                                "0x{}",
                                hex::encode(sha3::Keccak256::digest(
                                    hex::decode(&raw[2..]).unwrap()
                                ))
                            ))
                        }
                        other => panic!("Unexpected RPC {other}"),
                    };
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
                })
                .mount(&server)
                .await;
            let service = WalletService::new(vec![ChainEndpoints {
                chain_id: "ethereum".into(),
                endpoints: vec![server.uri()],
            }])
            .unwrap();
            let database = std::env::temp_dir().join(format!(
                "send-review-{}.sqlite",
                crate::store::new_event_id()
            ));
            service
                .open_state(database.to_string_lossy().into())
                .await
                .unwrap();
            let secrets = Arc::new(InMemorySecretStore::new());
            service.set_secret_store(secrets.clone());
            let mut wallet = WalletState::single_address(
                "w",
                "W",
                "Ethereum",
                "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
                Some("m/44'/60'/0'/0/0".into()),
                false,
            );
            let mut holding = native_coin_template("ethereum").unwrap();
            holding.amount = 10.0;
            wallet.holdings.push(holding);
            service
                .apply_state_command(StateCommand::UpsertWallet { wallet })
                .await
                .unwrap();
            store_seed_phrase(&*secrets, "w", "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about", password).unwrap();
            let input = input();
            if password.is_some() {
                for supplied in [None, Some("wrong-password".into())] {
                    let review = service.review_owned_send(input.clone()).await.unwrap();
                    assert!(review.requires_wallet_password);
                    let error = service
                        .execute_owned_send(review.id, input.clone(), supplied)
                        .await
                        .unwrap_err();
                    assert!(error.to_string().contains("password"), "{error}");
                }
                assert!(service.transactions().await.unwrap().is_empty());
                assert!(
                    !server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|r| r.body_json::<Value>().unwrap()["method"]
                            == "eth_sendRawTransaction")
                );
            }
            let review = service.review_owned_send(input.clone()).await.unwrap();
            assert_eq!(review.requires_wallet_password, password.is_some());
            let pinned = review.request.evm_overrides.clone().unwrap();
            assert_eq!(pinned.nonce, Some(7));
            let sent = service
                .execute_owned_send(
                    review.id.clone(),
                    input.clone(),
                    password.map(str::to_owned),
                )
                .await
                .unwrap();
            assert_eq!(sent.evm.unwrap().nonce, 7);
            assert!(service
                .execute_owned_send(review.id, input.clone(), None)
                .await
                .unwrap_err()
                .to_string()
                .contains("already consumed"));
            let stale = service.review_owned_send(input.clone()).await.unwrap();
            assert_eq!(
                stale.request.evm_overrides.unwrap().nonce,
                Some(8),
                "the journal advances a stale provider's nonce"
            );
            nonce.store(9, Ordering::SeqCst);
            assert!(service
                .execute_owned_send(stale.id, input, password.map(str::to_owned))
                .await
                .unwrap_err()
                .to_string()
                .contains("nonce changed"));
            let broadcasts = server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| r.body_json::<Value>().unwrap()["method"] == "eth_sendRawTransaction")
                .count();
            assert_eq!(broadcasts, 1);
            assert_eq!(service.transactions().await.unwrap().len(), 1);
        }
    }
}
