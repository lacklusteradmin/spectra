    use super::{
        aggregate_owned_addresses, build_persisted_snapshot, persisted_snapshot_from_json,
        plan_receive_selection, plan_self_send_confirmation, plan_store_derived_state,
        wallet_secret_index, OwnedAddressAggregationRequest, PendingSelfSendConfirmationInput,
        PersistedAppSnapshot, PersistedAppSnapshotRequest, ReceiveSelectionHoldingInput,
        ReceiveSelectionRequest, SelfSendConfirmationRequest, StoreDerivedHoldingInput,
        StoreDerivedStateRequest, StoreDerivedWalletInput, WalletSecretObservation,
    };
    use crate::state::CoreAppState;

    #[test]
    fn builds_secret_catalog_for_persisted_snapshot() {
        let request = PersistedAppSnapshotRequest {
            app_state_json: serde_json::to_string(&CoreAppState::default()).unwrap(),
            secret_observations: vec![WalletSecretObservation {
                wallet_id: "wallet-1".to_string(),
                secret_kind: Some("seedPhrase".to_string()),
                has_seed_phrase: true,
                has_private_key: false,
                has_password: true,
            }],
        };

        let mut app_state = CoreAppState::default();
        app_state.wallets.push(crate::state::WalletSummary {
            id: "wallet-1".to_string(),
            name: "Main".to_string(),
            is_watch_only: false,
            chain_name: "Bitcoin".to_string(),
            include_in_portfolio_total: true,
            network_mode: Some("mainnet".to_string()),
            xpub: None,
            derivation_preset: "standard".to_string(),
            derivation_path: None,
            holdings: Vec::new(),
            addresses: Vec::new(),
        });

        let request = PersistedAppSnapshotRequest {
            app_state_json: serde_json::to_string(&app_state).unwrap(),
            secret_observations: request.secret_observations,
        };
        let snapshot = build_persisted_snapshot(request).unwrap();

        assert_eq!(snapshot.secrets.len(), 1);
        assert_eq!(snapshot.secrets[0].wallet_id, "wallet-1");
        assert!(snapshot.secrets[0].has_signing_material);
        assert_eq!(
            snapshot.secrets[0].password_store_key,
            "wallet.seed.password.wallet-1"
        );
    }

    #[test]
    fn computes_wallet_secret_index_from_snapshot() {
        let snapshot = PersistedAppSnapshot {
            schema_version: 1,
            app_state: CoreAppState::default(),
            secrets: vec![
                super::SecretMaterialDescriptor {
                    wallet_id: "seed-wallet".to_string(),
                    secret_kind: "seedPhrase".to_string(),
                    has_seed_phrase: true,
                    has_private_key: false,
                    has_password: true,
                    has_signing_material: true,
                    seed_phrase_store_key: "wallet.seed.seed-wallet".to_string(),
                    password_store_key: "wallet.seed.password.seed-wallet".to_string(),
                    private_key_store_key: "wallet.privatekey.seed-wallet".to_string(),
                },
                super::SecretMaterialDescriptor {
                    wallet_id: "watch-wallet".to_string(),
                    secret_kind: "watchOnly".to_string(),
                    has_seed_phrase: false,
                    has_private_key: false,
                    has_password: false,
                    has_signing_material: false,
                    seed_phrase_store_key: "wallet.seed.watch-wallet".to_string(),
                    password_store_key: "wallet.seed.password.watch-wallet".to_string(),
                    private_key_store_key: "wallet.privatekey.watch-wallet".to_string(),
                },
            ],
        };

        let index = wallet_secret_index(&snapshot);
        assert_eq!(
            index.signing_material_wallet_ids,
            vec!["seed-wallet".to_string()]
        );
        assert_eq!(
            index.password_protected_wallet_ids,
            vec!["seed-wallet".to_string()]
        );
        assert!(index.private_key_backed_wallet_ids.is_empty());
    }

    #[test]
    fn upgrades_core_state_payload_into_empty_secret_snapshot() {
        let json = serde_json::to_string(&CoreAppState::default()).unwrap();
        let snapshot = persisted_snapshot_from_json(&json).unwrap();
        assert_eq!(snapshot.schema_version, 1);
        assert!(snapshot.secrets.is_empty());
    }

    #[test]
    fn plans_store_derived_state_with_stable_grouping() {
        let plan = plan_store_derived_state(StoreDerivedStateRequest {
            wallets: vec![
                StoreDerivedWalletInput {
                    wallet_id: "wallet-1".to_string(),
                    include_in_portfolio_total: true,
                    has_signing_material: true,
                    is_private_key_backed: false,
                    holdings: vec![
                        StoreDerivedHoldingInput {
                            holding_index: 0,
                            asset_identity_key: "Bitcoin|BTC".to_string(),
                            symbol_upper: "BTC".to_string(),
                            amount: "1.25".to_string(),
                            is_priced_asset: true,
                        },
                        StoreDerivedHoldingInput {
                            holding_index: 1,
                            asset_identity_key: "Ethereum|USDC".to_string(),
                            symbol_upper: "USDC".to_string(),
                            amount: "50".to_string(),
                            is_priced_asset: true,
                        },
                    ],
                },
                StoreDerivedWalletInput {
                    wallet_id: "wallet-2".to_string(),
                    include_in_portfolio_total: true,
                    has_signing_material: false,
                    is_private_key_backed: true,
                    holdings: vec![StoreDerivedHoldingInput {
                        holding_index: 0,
                        asset_identity_key: "Bitcoin|BTC".to_string(),
                        symbol_upper: "BTC".to_string(),
                        amount: "0.75".to_string(),
                        is_priced_asset: true,
                    }],
                },
            ],
        });

        assert_eq!(plan.included_portfolio_holding_refs.len(), 3);
        assert_eq!(plan.unique_price_request_holding_refs.len(), 2);
        assert_eq!(
            plan.signing_material_wallet_ids,
            vec!["wallet-1".to_string()]
        );
        assert_eq!(
            plan.private_key_backed_wallet_ids,
            vec!["wallet-2".to_string()]
        );
        assert_eq!(plan.grouped_portfolio.len(), 2);
        assert_eq!(plan.grouped_portfolio[0].asset_identity_key, "Bitcoin|BTC");
        assert_eq!(plan.grouped_portfolio[0].total_amount, "2");
    }

    #[test]
    fn aggregates_owned_addresses_in_order_without_duplicates() {
        let addresses = aggregate_owned_addresses(OwnedAddressAggregationRequest {
            candidate_addresses: vec![
                " 0xAbc ".to_string(),
                "".to_string(),
                "0xabc".to_string(),
                "bc1example".to_string(),
            ],
        });

        assert_eq!(
            addresses,
            vec!["0xAbc".to_string(), "bc1example".to_string()]
        );
    }

    #[test]
    fn prefers_native_receive_holding_for_resolved_chain() {
        let plan = plan_receive_selection(ReceiveSelectionRequest {
            receive_chain_name: "Ethereum".to_string(),
            available_receive_chains: vec!["Ethereum".to_string()],
            available_receive_holdings: vec![
                ReceiveSelectionHoldingInput {
                    holding_index: 0,
                    chain_name: "Ethereum".to_string(),
                    has_contract_address: true,
                },
                ReceiveSelectionHoldingInput {
                    holding_index: 1,
                    chain_name: "Ethereum".to_string(),
                    has_contract_address: false,
                },
            ],
        });

        assert_eq!(plan.resolved_chain_name, "Ethereum");
        assert_eq!(plan.selected_receive_holding_index, Some(1));
    }

    #[test]
    fn consumes_matching_pending_self_send_confirmation() {
        let plan = plan_self_send_confirmation(SelfSendConfirmationRequest {
            pending_confirmation: Some(PendingSelfSendConfirmationInput {
                wallet_id: "wallet-1".to_string(),
                chain_name: "Bitcoin".to_string(),
                symbol: "BTC".to_string(),
                destination_address_lowercased: "bc1self".to_string(),
                amount: 1.5,
                created_at_unix: 100.0,
            }),
            wallet_id: "wallet-1".to_string(),
            chain_name: "Bitcoin".to_string(),
            symbol: "BTC".to_string(),
            destination_address: "BC1SELF".to_string(),
            amount: 1.5,
            now_unix: 110.0,
            window_seconds: 30.0,
            owned_addresses: vec!["bc1self".to_string()],
        });

        assert!(!plan.requires_confirmation);
        assert!(plan.consume_existing_confirmation);
        assert!(plan.clear_pending_confirmation);
    }
