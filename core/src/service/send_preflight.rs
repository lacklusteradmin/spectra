//! Send eligibility, asset routing and recipient warnings from owned state.
use crate::SpectraBridgeError;
use crate::service::WalletService;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Can this send be made?
    pub async fn send_submit_preflight(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_address: String,
        amount_input: String,
    ) -> Result<crate::send::SendPreflight, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let state = this.wallet_state.read().await;
            let wallet = state.wallets.iter().find(|w| w.id == wallet_id);
            let holding = wallet.and_then(|wallet| {
                wallet
                    .holdings
                    .iter()
                    .find(|h| h.deployment_id() == holding_key)
            });
            let asset = holding
                .and_then(|holding| crate::send::SendAsset::of(holding, &state.token_preferences));
            Ok(crate::send::validate_send_preflight(
                wallet.is_some(),
                asset.as_ref(),
                holding.map_or("0", |h| h.amount.as_str()),
                &destination_address,
                &amount_input,
            )?)
        })
        .await
    }
}

/// A send to one of the user's own wallets is not to a stranger. Its review
/// asks to confirm the self-send; "a new destination with no history" beside
/// that said the opposite about the same address, so it goes.
pub(super) fn without_new_address_for_self_send(
    warnings: Vec<crate::send::flow::HighRiskSendWarning>,
    is_self_send: bool,
) -> Vec<crate::send::flow::HighRiskSendWarning> {
    if !is_self_send {
        return warnings;
    }
    warnings
        .into_iter()
        .filter(|warning| !matches!(warning, crate::send::flow::HighRiskSendWarning::NewAddress))
        .collect()
}

impl WalletService {
    /// Reasons this send looks risky, as codes the platform localizes.
    pub async fn high_risk_send_reasons(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: String,
        destination_address: String,
        destination_input: String,
        used_ens_resolution: bool,
    ) -> Vec<crate::send::flow::HighRiskSendWarning> {
        let state = self.wallet_state.read().await;
        let Some(wallet) = state.wallets.iter().find(|w| w.id == wallet_id) else {
            return Vec::new();
        };
        let Some(holding) = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
        else {
            return Vec::new();
        };
        let chain_id = holding.chain_id;
        let symbol = holding.symbol.clone();
        let holding_amount = holding.amount.clone();
        let wallet_chain_id = wallet.chain_id;
        let address_book_entries: Vec<_> = state
            .address_book
            .iter()
            .map(|entry| crate::send::flow::HighRiskChainAddress {
                chain_id: entry.chain_id,
                address: entry.address.clone(),
            })
            .collect();
        drop(state);

        // Addresses this wallet has already sent to on this chain. A first-time
        // destination is one of the risk signals, so reading a caller's copy of
        // the history meant the signal was only as complete as that copy.
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        if let Ok(rows) = self.fetch_all_history_records().await {
            for row in rows {
                if row.payload.chain_id == chain_id {
                    seen.insert(row.payload.address.clone());
                }
            }
        }

        crate::send::flow::evaluate_high_risk_send_reasons(crate::send::flow::HighRiskSendRequest {
            chain_id,
            symbol,
            amount,
            holding_amount,
            destination_address,
            destination_input,
            used_ens_resolution,
            wallet_chain_id,
            address_book_entries,
            tx_addresses: seen
                .into_iter()
                .map(|address| crate::send::flow::HighRiskChainAddress { chain_id, address })
                .collect(),
        })
    }

    /// Warnings about an EVM recipient, as codes the platform localizes.
    ///
    /// Core makes the two contract-code probes itself. They were already its
    /// own network calls — the caller made them, caught their errors, worked
    /// out which token the holding is from core's token list, and handed the
    /// three answers back for core to turn into warnings.
    pub async fn evm_recipient_preflight(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_address: String,
    ) -> Vec<crate::store::EvmRecipientPreflightWarning> {
        let state = self.wallet_state.read().await;
        let Some(holding) = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .and_then(|wallet| {
                wallet
                    .holdings
                    .iter()
                    .find(|h| h.deployment_id() == holding_key)
            })
        else {
            return Vec::new();
        };
        let Some(chain) = Some(holding.chain_id).filter(|chain| chain.is_evm()) else {
            return Vec::new();
        };
        let holding_symbol = holding.symbol.clone();
        let token = supported_evm_token(holding, &state.token_preferences);
        drop(state);

        // A probe that fails is `None`, not `false`: "we could not check" and
        // "it is not a contract" are different answers, and the evaluator
        // treats them differently.
        let chain_id = chain;
        let recipient_has_code = self
            .fetch_evm_has_contract_code(chain_id, destination_address)
            .await
            .ok();
        let token_has_code = match &token {
            Some((_, contract)) => self
                .fetch_evm_has_contract_code(chain_id, contract.clone())
                .await
                .ok(),
            None => None,
        };
        crate::store::evm_recipient_preflight_warnings(crate::store::EvmRecipientPreflightRequest {
            chain_id,
            holding_symbol,
            token_symbol: token.map(|(symbol, _)| symbol),
            recipient_has_code,
            token_has_code,
        })
    }
}

/// The known EVM token a holding is, as `(symbol, contract)`.
///
/// `None` for a chain's own gas asset — a chain's native asset is never one of
/// its tokens — and for a token the user does not track.
fn supported_evm_token(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Option<(String, String)> {
    let chain = holding.chain_id;
    if !chain.is_evm() || holding.is_native() {
        return None;
    }
    preferences
        .iter()
        .find(|entry| entry.is_enabled && entry.token.matches_holding(holding))
        .map(|entry| (entry.token.symbol.clone(), entry.token.contract.clone()))
}

impl WalletService {
    /// Direct CLI builds get the same durable advisories even without a tracked holding.
    pub(super) async fn staged_send_review(
        &self,
        request: &crate::send::SendExecutionRequest,
    ) -> Result<crate::send::stages::SendArtifactReview, SpectraBridgeError> {
        use crate::send::flow::{HighRiskChainAddress, HighRiskSendRequest};
        let chain = request.chain_id;
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == request.wallet_id)
            .ok_or("Wallet removed")?;
        let normalize_contract =
            |value: Option<String>| crate::tokens::normalize_token_identifier(value, chain);
        let holding = wallet.holdings.iter().find(|h| {
            h.chain_id == chain
                && normalize_contract(h.contract_address.clone())
                    == normalize_contract(request.contract_address.clone())
        });
        let symbol = holding.map(|h| h.symbol.clone()).unwrap_or_else(|| {
            request
                .contract_address
                .clone()
                .unwrap_or_else(|| chain.coin_symbol().into())
        });
        let warnings = crate::send::flow::evaluate_high_risk_send_reasons(HighRiskSendRequest {
            chain_id: chain,
            symbol: symbol.clone(),
            amount: crate::decimal::canonical(&request.amount_str).ok_or("Invalid amount")?,
            holding_amount: holding.map_or_else(|| "0".into(), |h| h.amount.clone()),
            destination_address: request.to_address.clone(),
            destination_input: request.to_address.clone(),
            used_ens_resolution: false,
            wallet_chain_id: wallet.chain_id,
            address_book_entries: state
                .address_book
                .iter()
                .map(|e| HighRiskChainAddress {
                    chain_id: e.chain_id,
                    address: e.address.clone(),
                })
                .collect(),
            tx_addresses: self
                .fetch_all_history_records()
                .await?
                .into_iter()
                .map(|r| HighRiskChainAddress {
                    chain_id: r.payload.chain_id,
                    address: r.payload.address,
                })
                .collect(),
        });
        let recipient_warnings = if chain.is_evm() {
            let recipient_has_code = self
                .fetch_evm_has_contract_code(request.chain_id, request.to_address.clone())
                .await
                .ok();
            let token_has_code = if let Some(contract) = &request.contract_address {
                self.fetch_evm_has_contract_code(request.chain_id, contract.clone())
                    .await
                    .ok()
            } else {
                None
            };
            crate::store::evm_recipient_preflight_warnings(
                crate::store::EvmRecipientPreflightRequest {
                    chain_id: chain,
                    holding_symbol: symbol.clone(),
                    token_symbol: request.contract_address.as_ref().map(|_| symbol),
                    recipient_has_code,
                    token_has_code,
                },
            )
        } else {
            Vec::new()
        };
        let requires_self_send_confirmation =
            self.is_own_address(chain, &request.to_address).await?;
        Ok(crate::send::stages::SendArtifactReview {
            warnings: without_new_address_for_self_send(warnings, requires_self_send_confirmation),
            recipient_warnings,
            requires_self_send_confirmation,
        })
    }
}

#[cfg(test)]
mod self_send_warning_tests {
    use super::without_new_address_for_self_send;
    use crate::send::flow::HighRiskSendWarning;

    #[test]
    fn a_self_send_is_not_a_new_destination() {
        let warnings = vec![
            HighRiskSendWarning::NewAddress,
            HighRiskSendWarning::LargeSend {
                percent: 50,
                symbol: "ETH".into(),
            },
        ];
        assert_eq!(
            without_new_address_for_self_send(warnings.clone(), true),
            vec![HighRiskSendWarning::LargeSend {
                percent: 50,
                symbol: "ETH".into()
            }]
        );
        assert_eq!(
            without_new_address_for_self_send(warnings.clone(), false),
            warnings
        );
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use crate::registry::Chain;
    use crate::store::state::WalletState;
    use crate::store::wallet_domain::AssetHolding;
    use crate::store::wallet_domain::{CoreTokenPreferenceCategory, CoreTokenPreferenceEntry};

    fn holding(
        chain: crate::registry::Chain,
        symbol: &str,
        standard: &str,
        contract: Option<&str>,
    ) -> AssetHolding {
        AssetHolding {
            id: String::new(),
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coingecko_id: String::new(),
            chain_id: chain,
            token_standard: standard.to_string(),
            contract_address: contract.map(str::to_string),
            amount: "10".into(),
        }
    }

    fn known(chain: Chain, contract: &str) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: false,
            is_enabled: true,
            token: crate::tokens::TokenDeploymentEntry {
                deployment_id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain_id: chain,
                name: "Token".into(),
                symbol: "TOK".into(),
                token_standard: String::new(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                coinpaprika_id: String::new(),
                decimals: 6,
                tags: Vec::new(),
                color: None,
                artwork_name: String::new(),
                enabled: true,
            },
        }
    }

    /// Tracking a mint is what makes a Solana token sendable, from the token
    /// list core holds. The preview and submit paths each asked this on their
    /// own side before, and the send button a third time.
    #[tokio::test]
    async fn sendability_follows_the_token_list_core_holds() {
        let service = WalletService::new(Vec::new()).expect("service");
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let standard = Chain::Solana.token_standard().to_string();
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet = WalletState::single_address(
                "w1",
                "W",
                crate::registry::Chain::Solana,
                "SoLaddr",
                None,
                false,
            );
            wallet.holdings = vec![holding(
                crate::registry::Chain::Solana,
                "USDC",
                &standard,
                Some(mint),
            )];
            state.wallets.push(wallet);
        }
        let preflight = || {
            service.send_submit_preflight(
                "w1".into(),
                format!("solana:spl:{mint}"),
                "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin".into(),
                "1".into(),
            )
        };

        assert!(
            preflight().await.is_err(),
            "an untracked mint is not sendable"
        );
        service.wallet_state.write().await.token_preferences = vec![known(Chain::Solana, mint)];
        let known_now = preflight().await.expect("a tracked mint is sendable");
        assert_eq!(known_now.token_contract_address.as_deref(), Some(mint));
    }

    /// A wallet or holding core cannot find is refused, not guessed at.
    #[tokio::test]
    async fn a_send_for_an_unknown_wallet_is_refused() {
        let service = WalletService::new(Vec::new()).expect("service");
        let err = service
            .send_submit_preflight(
                "nope".into(),
                "bitcoin:native".into(),
                "bc1qexample".into(),
                "1".into(),
            )
            .await;
        assert!(err.is_err(), "no wallet, no send");
    }

    #[tokio::test]
    async fn a_send_resolves_its_holding_by_chain_and_symbol() {
        let service = WalletService::new(Vec::new()).expect("service");
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet = WalletState::single_address(
                "w1",
                "W",
                crate::registry::Chain::Bitcoin,
                "bc1qowner",
                None,
                false,
            );
            wallet.holdings = vec![holding(
                crate::registry::Chain::Bitcoin,
                "BTC",
                "Native",
                None,
            )];
            state.wallets.push(wallet);
        }
        let plan = service
            .send_submit_preflight(
                "w1".into(),
                "bitcoin:native".into(),
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".into(),
                "1".into(),
            )
            .await
            .expect("a known wallet and holding");
        assert_eq!(plan.chain, Chain::Bitcoin);
        assert_eq!(plan.symbol, "BTC");
        assert_eq!(plan.amount, "1");
    }
}
