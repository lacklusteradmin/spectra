use super::*;
#[cfg(test)]
mod token_decimals_come_from_the_contract {
    use crate::registry::Chain;
    use crate::service::WalletService;

    /// A missing execution metadata reader does not bypass identity validation.
    #[tokio::test]
    async fn valid_tokens_without_execution_metadata_return_none() {
        let service = WalletService::new(Vec::new()).expect("service");
        for (chain, identifier) in [
            (
                Chain::Ton,
                "0:0000000000000000000000000000000000000000000000000000000000000001",
            ),
            (Chain::Sui, "0x1::coin::TEST"),
            (Chain::Aptos, "0x1"),
        ] {
            assert_eq!(
                service
                    .token_contract_decimals(chain, identifier)
                    .await
                    .unwrap(),
                None,
                "{} has no execution metadata reader",
                chain.str_id()
            );
            assert!(
                service
                    .token_contract_decimals(chain, "whatever")
                    .await
                    .is_err()
            );
        }
    }
}

pub(super) mod request_fixture {
    use crate::send::SendExecutionRequest;
    pub(in crate::service::send_execution) fn req(
        chain_id: crate::registry::Chain,
    ) -> SendExecutionRequest {
        SendExecutionRequest {
            token_standard: None,
            chain_id,
            wallet_id: "w".into(),
            password: None,
            to_address: "to".to_string(),
            amount_str: "1.5".into(),
            contract_address: None,
            token_decimals: None,
            fee_rate_svb: None,
            fee_sat: None,
            gas_budget: None,
            fee_amount: None,
            evm_overrides: None,
            monero_priority: None,
            sign_only: false,
        }
    }
}

#[cfg(test)]
mod sign_only_tests {
    use super::request_fixture::req;

    /// "Sign and stop" is one question however it was asked: through the
    /// request's own field or through the EVM overrides.
    #[test]
    fn either_route_asks_the_same_thing() {
        let plain = req(crate::registry::Chain::Ethereum);
        assert!(!plain.wants_sign_only(), "a send is not a dry run");

        let mut by_field = req(crate::registry::Chain::Ethereum);
        by_field.sign_only = true;
        assert!(by_field.wants_sign_only());

        let mut by_overrides = req(crate::registry::Chain::Ethereum);
        by_overrides.evm_overrides = Some(crate::send::ethereum::EvmSendOverridesInput {
            sign_only: Some(true),
            ..Default::default()
        });
        assert!(
            by_overrides.wants_sign_only(),
            "the older route asks for a dry run just as much"
        );

        // Overrides that say nothing about it do not unsay the field.
        let mut both = req(crate::registry::Chain::Ethereum);
        both.sign_only = true;
        both.evm_overrides = Some(crate::send::ethereum::EvmSendOverridesInput {
            sign_only: None,
            ..Default::default()
        });
        assert!(both.wants_sign_only());
    }
}

#[cfg(test)]
mod send_chain_tests {
    use super::send_chain_for;
    use crate::registry::Chain;
    use crate::store::state::{CoreAppState, WalletState};

    fn wallet(id: &str, chain: Chain, chain_id: Option<Chain>) -> WalletState {
        WalletState {
            id: id.to_string(),
            name: id.to_string(),
            signing: crate::store::state::WalletSigning::SeedPhrase {
                password_protected: false,
            },
            include_in_portfolio_total: true,
            chain_id: chain_id.unwrap_or(chain),
            xpub: None,
            derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: Vec::new(),
        }
    }

    /// A send is signed for the network the wallet is on, never the family's
    /// mainnet. `spectra send broadcast --sign-only` prints the signed chain
    /// id.
    #[test]
    fn a_send_requires_the_explicit_network_and_never_retargets() {
        let state = CoreAppState {
            wallets: vec![wallet("w1", Chain::Ethereum, Some(Chain::EthereumSepolia))],
            ..Default::default()
        };
        assert!(send_chain_for(&state, "w1", Chain::Ethereum).is_err());
        assert_eq!(
            send_chain_for(&state, "w1", Chain::EthereumSepolia).unwrap(),
            Chain::EthereumSepolia
        );
        assert!(send_chain_for(&state, "nobody", Chain::Ethereum).is_err());
    }
}

#[tokio::test]
async fn invalid_exact_amount_and_fee_refuse_before_storage_or_keys() {
    let service = WalletService::new(vec![]).unwrap();
    for amount in ["-1", "NaN", "0.0000000000000000001", "1e8"] {
        let mut request = request_fixture::req(crate::registry::Chain::Ethereum);
        request.amount_str = amount.into();
        let error = service.build_send(request).await.unwrap_err().to_string();
        assert!(
            !error.contains("wallet") && !error.contains("database"),
            "{error}"
        );
    }
    for fee in ["NaN", "-1", "inf", "0.00000000001"] {
        let mut request = request_fixture::req(crate::registry::Chain::Bitcoin);
        request.fee_rate_svb = Some(fee.into());
        assert!(
            service
                .build_send(request)
                .await
                .unwrap_err()
                .to_string()
                .contains("fee must")
        );
    }
}

#[tokio::test]
async fn xrp_amounts_outside_protocol_range_refuse_before_wallet_or_network_reads() {
    let service = WalletService::new(vec![]).unwrap();
    for chain in [
        crate::registry::Chain::Xrp,
        crate::registry::Chain::XrpTestnet,
    ] {
        for amount in ["0", "100000000000.000001", "18446744073709.551615"] {
            let mut request = request_fixture::req(chain);
            request.amount_str = amount.into();
            let error = service.build_send(request).await.unwrap_err().to_string();
            assert!(error.contains("XRP amount and fee"), "{amount}: {error}");
        }
    }
}

#[tokio::test]
async fn saved_signature_expiry_is_checked_again_before_submission() {
    use crate::send::stages::*;
    let service = WalletService::new(vec![]).unwrap();
    let mut stored = StoredSend {
        request: request_fixture::req(crate::registry::Chain::Ton),
        view: SendArtifact {
            id: "fixture".into(),
            revision: 0,
            stage: SendStage::Prepared,
            wallet_id: "w".into(),
            chain_id: crate::registry::Chain::Ton,
            sender: String::new(),
            recipient: "to".into(),
            amount: "1.5".into(),
            asset: "GRAM".into(),
            symbol: "GRAM".into(),
            staking: None,
            created_at: 0.0,
            review_digest: String::new(),
            review: SendArtifactReview::default(),
            prepared_details: String::new(),
            signing_payload_hex: String::new(),
            signed_payload: None,
            transaction_hash: None,
            attempts: vec![],
            selected_endpoints: vec![],
        },
        prepared: PreparedPayload::Ton {
            seqno: 1,
            amount: 1,
            valid_until: 0,
            jetton: None,
        },
        submission: None,
        signed_digest: None,
        substrate_verified_through: None,
        icp_staking_receipts: vec![],
    };
    assert!(
        service
            .validate_signed_expiry(Chain::Ton, &stored)
            .await
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
    stored.prepared = PreparedPayload::Ton {
        seqno: 1,
        amount: 1,
        valid_until: (crate::store::now_unix() as u32) + 60,
        jetton: None,
    };
    service
        .validate_signed_expiry(Chain::Ton, &stored)
        .await
        .unwrap();
    stored.prepared = PreparedPayload::Near {
        public_key: [1; 32],
        nonce: 1,
        block_hash: [2; 32],
        amount: 1,
        token_contract: None,
        fee_budget: "0".into(),
    };
    let server = wiremock::MockServer::start().await;
    let head = Arc::new(std::sync::atomic::AtomicU64::new(102));
    let mock_head = head.clone();
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let hash = bs58::encode([2; 32]).into_string();
            let result = match body["method"].as_str().unwrap() {
                "status" => serde_json::json!({"chain_id":"mainnet"}),
                "block" if body["params"]["finality"] == "optimistic" => {
                    serde_json::json!({"header":{"hash":hash,"height":mock_head.load(std::sync::atomic::Ordering::SeqCst)}})
                }
                "block" => serde_json::json!({"header":{"hash":hash,"height":1}}),
                "EXPERIMENTAL_protocol_config" => {
                    serde_json::json!({"transaction_validity_period":100})
                }
                other => panic!("Unexpected NEAR expiry read: {other}"),
            };
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"jsonrpc":"2.0","id":"1","result":result}))
        })
        .mount(&server)
        .await;
    let service = WalletService::new(vec![crate::service::ChainEndpoints {
        chain_id: Chain::Near,
        capabilities: vec![crate::EndpointCapability::Verification],
        endpoints: vec![server.uri()],
    }])
    .unwrap();
    // Both native and NEP-141 use the exact referenced block. A brand new
    // artifact can be expired; an old artifact can still be protocol-valid.
    for token_contract in [None, Some("token.near".to_string())] {
        if let PreparedPayload::Near {
            token_contract: contract,
            ..
        } = &mut stored.prepared
        {
            *contract = token_contract;
        }
        stored.view.created_at = crate::store::now_unix();
        head.store(102, std::sync::atomic::Ordering::SeqCst);
        assert!(
            service
                .validate_signed_expiry(Chain::Near, &stored)
                .await
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
        stored.view.created_at = 0.0;
        head.store(101, std::sync::atomic::Ordering::SeqCst);
        service
            .validate_signed_expiry(Chain::Near, &stored)
            .await
            .unwrap();
    }
}
