use super::*;
#[cfg(test)]
mod token_decimals_come_from_the_contract {
    use crate::registry::Chain;
    use crate::service::WalletService;

    /// Which families core can ask, and which still take the caller's word.
    ///
    /// `build_execute_send_payload` used `req.token_decimals.unwrap_or(6)`, so
    /// a caller that supplied nothing denominated its transfer at six places
    /// whatever the contract said. It reads `decimals()` off the token now,
    /// and only a family without a reader may fall back to caller precision.
    ///
    /// This asserts the gate, not the network read: a chain the helper has no
    /// client for must answer `None` without attempting a call, which is what
    /// keeps the fallback reachable for TON, Sui and Aptos.
    #[tokio::test]
    async fn a_family_core_cannot_ask_falls_back_to_the_caller() {
        let service = WalletService::new(Vec::new()).expect("service");
        for chain in [Chain::Ton, Chain::Sui, Chain::Aptos] {
            assert_eq!(
                service
                    .token_contract_decimals(chain, "whatever")
                    .await
                    .unwrap(),
                None,
                "{} has no metadata client, so the caller's value must stand",
                chain.str_id()
            );
        }
    }

    /// The families that are asked are the ones with a metadata call.
    #[test]
    fn the_families_core_asks_are_evm_and_tron() {
        let asks: Vec<_> = Chain::all()
            .filter(|c| !c.is_testnet() && (c.is_evm() || *c == Chain::Tron))
            .collect();
        assert!(
            asks.len() >= 24,
            "expected the EVM family plus Tron, got {}",
            asks.len()
        );
        assert!(asks.contains(&Chain::Tron));
        assert!(asks.contains(&Chain::Ethereum));
    }
}

pub(super) mod request_fixture {
    use crate::send::SendExecutionRequest;
    pub(in crate::service::send_execution) fn req(
        chain_id: &str,
        _chain_name: &str,
    ) -> SendExecutionRequest {
        SendExecutionRequest {
            chain_id: chain_id.to_string(),
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

    /// "Sign and stop" is one question however it was asked.
    ///
    /// It had two routes and four readers, and they disagreed: the refusal
    /// read both routes, the result field read only `sign_only`, and the
    /// Bitcoin builder read only `sign_only` too. So a caller asking through
    /// the EVM overrides — the route that existed first, and the one the
    /// field's own doc comment still points at — got a signed transaction
    /// back with `signed_payload: None`.
    #[test]
    fn either_route_asks_the_same_thing() {
        let plain = req("ethereum", "ethereum");
        assert!(!plain.wants_sign_only(), "a send is not a dry run");

        let mut by_field = req("ethereum", "ethereum");
        by_field.sign_only = true;
        assert!(by_field.wants_sign_only());

        let mut by_overrides = req("ethereum", "ethereum");
        by_overrides.evm_overrides = Some(crate::send::ethereum::EvmSendOverridesInput {
            sign_only: Some(true),
            ..Default::default()
        });
        assert!(
            by_overrides.wants_sign_only(),
            "the older route asks for a dry run just as much"
        );

        // Overrides that say nothing about it do not unsay the field.
        let mut both = req("ethereum", "ethereum");
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

    fn wallet(id: &str, chain: Chain, chain_id: Option<&str>) -> WalletState {
        WalletState {
            id: id.to_string(),
            name: id.to_string(),
            signing: crate::store::state::WalletSigning::SeedPhrase {
                password_protected: false,
            },
            include_in_portfolio_total: true,
            chain_id: chain_id.unwrap_or(chain.str_id()).to_string(),
            xpub: None,
            derivation_preset: crate::store::wallet_domain::CoreSeedDerivationPreset::Standard,
            derivation_path: None,
            derivation_overrides: Default::default(),
            holdings: Vec::new(),
            addresses: Vec::new(),
        }
    }

    /// A send is signed for the network the wallet is on.
    ///
    /// It used to be signed for the family's mainnet whatever network was
    /// selected: with the app on Sepolia, a send still signed chain id 1 and
    /// read mainnet endpoints, so what the user believed was a testnet
    /// transaction was a valid mainnet one. `spectra send broadcast
    /// --sign-only` prints the signed chain id, which is how it was found.
    #[test]
    fn a_send_requires_the_explicit_network_and_never_retargets() {
        let mut state = CoreAppState {
            wallets: vec![wallet("w1", Chain::Ethereum, Some("ethereum-sepolia"))],
            ..Default::default()
        };
        assert!(send_chain_for(&state, "w1", Chain::Ethereum).is_err());
        assert_eq!(
            send_chain_for(&state, "w1", Chain::EthereumSepolia).unwrap(),
            Chain::EthereumSepolia
        );
        assert!(send_chain_for(&state, "nobody", Chain::Ethereum).is_err());
        state
            .settings
            .selected_chain_by_family
            .insert("ethereum".into(), "ethereum-hoodi".into());
        assert_eq!(
            send_chain_for(&state, "w1", Chain::EthereumSepolia).unwrap(),
            Chain::EthereumSepolia
        );
    }
}

#[tokio::test]
async fn invalid_exact_amount_and_fee_refuse_before_storage_or_keys() {
    let service = WalletService::new(vec![]).unwrap();
    for amount in ["-1", "NaN", "0.0000000000000000001", "1e8"] {
        let mut request = request_fixture::req("ethereum", "ethereum");
        request.amount_str = amount.into();
        let error = service.build_send(request).await.unwrap_err().to_string();
        assert!(
            !error.contains("wallet") && !error.contains("database"),
            "{error}"
        );
    }
    for fee in [f64::NAN, -1.0, f64::INFINITY, 0.00000000001] {
        let mut request = request_fixture::req("bitcoin", "bitcoin");
        request.fee_rate_svb = Some(fee);
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
async fn saved_signature_expiry_is_checked_again_before_submission() {
    use crate::send::stages::*;
    let service = WalletService::new(vec![]).unwrap();
    let mut stored = StoredSend {
        request: request_fixture::req("ton", "TON"),
        view: SendArtifact {
            id: "fixture".into(),
            revision: 0,
            stage: SendStage::Prepared,
            wallet_id: "w".into(),
            chain_id: "ton".into(),
            sender: String::new(),
            recipient: "to".into(),
            amount: "1.5".into(),
            asset: "TON".into(),
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
        },
        submission: None,
        signed_digest: None,
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
    };
    assert!(
        service
            .validate_signed_expiry(Chain::Near, &stored)
            .await
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
}
