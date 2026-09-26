//! Send eligibility, asset routing and recipient warnings from owned state.
use crate::SpectraBridgeError;
use crate::service::WalletService;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Can this send be made, and how should it be routed?
    pub async fn send_submit_preflight(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_address: String,
        amount_input: String,
    ) -> Result<crate::send::SendPreflight, SpectraBridgeError> {
        let state = self.wallet_state.read().await;
        let wallet = state.wallets.iter().find(|w| w.id == wallet_id);
        let holding = wallet.and_then(|wallet| {
            wallet
                .holdings
                .iter()
                .find(|h| h.deployment_id() == holding_key)
        });
        let request = crate::send::SendSubmitPreflightRequest {
            wallet_found: wallet.is_some(),
            asset_found: holding.is_some(),
            destination_address,
            amount_input,
            available_balance: holding
                .map(|h| h.amount.clone())
                .unwrap_or_else(|| "0".into()),
            asset: holding.map(|holding| routing_input(holding, &state.token_preferences)),
            token: holding
                .and_then(|holding| send_token_identity(holding, &state.token_preferences)),
        };
        Ok(crate::send::validate_send_preflight(request)?)
    }

    /// How a holding's send and preview are routed.
    ///
    /// The same derivation the preflight does, for the callers that only want
    /// the routing: which preview to refresh, and which submit branch to take.
    /// Both used to be decided again on the caller's side — the preview from a
    /// `SendAssetRoutingInput` it assembled, the Solana submit branch from its
    /// own copy of the send-support rule — so three places could disagree
    /// about whether an asset is sendable.
    pub async fn send_asset_routing(
        &self,
        wallet_id: String,
        holding_key: String,
    ) -> Option<crate::send::SendAssetRoute> {
        let state = self.wallet_state.read().await;
        let holding = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)?
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)?;
        Some(crate::send::route_send_asset(&routing_input(
            holding,
            &state.token_preferences,
        )))
    }
}

impl WalletService {
    /// Reasons this send looks risky, as codes the platform localizes.
    pub async fn high_risk_send_reasons(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: f64,
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
        let chain_id = holding.chain_id.clone();
        let symbol = holding.symbol.clone();
        let holding_amount = crate::decimal::to_f64(&holding.amount);
        let wallet_chain_id = wallet.chain_id.clone();
        let address_book_entries: Vec<_> = state
            .address_book
            .iter()
            .map(|entry| crate::send::flow::HighRiskChainAddress {
                chain_id: entry.chain_id.clone(),
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
            chain_id: chain_id.clone(),
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
                .map(|address| crate::send::flow::HighRiskChainAddress {
                    chain_id: chain_id.clone(),
                    address,
                })
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
        let Some(chain) =
            crate::registry::Chain::from_str_id(&holding.chain_id).filter(|chain| chain.is_evm())
        else {
            return Vec::new();
        };
        let holding_symbol = holding.symbol.clone();
        let token = supported_evm_token(holding, &state.token_preferences);
        drop(state);

        // A probe that fails is `None`, not `false`: "we could not check" and
        // "it is not a contract" are different answers, and the evaluator
        // treats them differently.
        let chain_id = chain.str_id().to_string();
        let recipient_has_code = self
            .fetch_evm_has_contract_code(chain_id.clone(), destination_address)
            .await
            .ok();
        let token_has_code = match &token {
            Some((_, contract)) => self
                .fetch_evm_has_contract_code(chain_id.clone(), contract.clone())
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
    let chain = crate::registry::Chain::from_str_id(&holding.chain_id)?;
    if !chain.is_evm() || holding.is_native() {
        return None;
    }
    preferences
        .iter()
        .find(|entry| entry.is_enabled && entry.token.matches_holding(holding))
        .map(|entry| (entry.token.symbol.clone(), entry.token.contract.clone()))
}

/// What routing needs to know about a holding, derived from core's own state.
/// The token a holding is, with the decimals the catalog gives it.
///
/// A holding whose symbol is the chain's gas asset is native and has none. A
/// token the user does not track has none either, which is what makes the
/// send refuse rather than guess a scale — every submit branch used to look
/// this up itself, and Tron's branch hard-coded six decimals for every token
/// on it.
pub(super) fn send_token_identity(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Option<crate::send::SendTokenIdentity> {
    crate::registry::Chain::from_str_id(&holding.chain_id)?;
    if holding.is_native() {
        return None;
    }
    preferences
        .iter()
        .find(|entry| entry.is_enabled && entry.token.matches_holding(holding))
        .map(|entry| crate::send::SendTokenIdentity {
            contract: entry.token.contract.clone(),
            decimals: entry.token.decimals,
        })
}

fn routing_input(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> crate::send::SendAssetRoutingInput {
    crate::send::SendAssetRoutingInput {
        is_native: holding.is_native(),
        chain_id: holding.chain_id.clone(),
        symbol: holding.symbol.clone(),
        is_evm_chain: crate::registry::Chain::from_str_id(&holding.chain_id)
            .is_some_and(|chain| chain.is_evm()),
        supports_solana_send_coin: supports_solana_send(holding, preferences),
        supports_near_token_send: supports_near_token_send(holding, preferences),
    }
}

/// Whether a Solana holding is one this build can send.
///
/// SOL always; a token only if the user tracks its mint. The mint comes from
/// the holding's contract, or from the catalog by symbol when the holding does
/// not carry one.
fn supports_solana_send(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    let chain = crate::registry::Chain::Solana;
    if holding.chain_id != chain.str_id() {
        return false;
    }
    if holding.is_native() {
        return true;
    }
    if holding.token_standard != chain.token_standard() {
        return false;
    }
    let Some(mint) = holding.contract_address.clone().filter(|c| !c.is_empty()) else {
        return false;
    };
    preferences
        .iter()
        .any(|entry| entry.hosting_chain() == Some(chain) && entry.token.contract == mint)
}

/// Whether a NEAR holding is a token this build can send. NEAR itself is not:
/// the native path handles it.
fn supports_near_token_send(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    let chain = crate::registry::Chain::Near;
    if holding.chain_id != chain.str_id() || holding.is_native() {
        return false;
    }
    if holding.token_standard != chain.token_standard() {
        return false;
    }
    let Some(contract) = holding
        .contract_address
        .as_deref()
        .filter(|c| !c.is_empty())
    else {
        return false;
    };
    preferences.iter().any(|entry| {
        entry.hosting_chain() == Some(chain) && entry.token.contract.eq_ignore_ascii_case(contract)
    })
}

impl WalletService {
    /// Direct CLI builds get the same durable advisories even without a tracked holding.
    pub(super) async fn staged_send_review(
        &self,
        request: &crate::send::SendExecutionRequest,
    ) -> Result<crate::send::stages::SendArtifactReview, SpectraBridgeError> {
        use crate::send::flow::{HighRiskChainAddress, HighRiskSendRequest};
        let chain = super::chain_for_id(&request.chain_id)?;
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == request.wallet_id)
            .ok_or("Wallet removed")?;
        let normalize_contract = |value: Option<String>| {
            crate::tokens::normalize_token_identifier(value, chain.str_id().into())
        };
        let holding = wallet.holdings.iter().find(|h| {
            h.chain() == Some(chain)
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
            chain_id: chain.str_id().into(),
            symbol: symbol.clone(),
            amount: request.amount_str.parse().map_err(|_| "Invalid amount")?,
            holding_amount: holding
                .map(|h| crate::decimal::to_f64(&h.amount))
                .unwrap_or(0.0),
            destination_address: request.to_address.clone(),
            destination_input: request.to_address.clone(),
            used_ens_resolution: false,
            wallet_chain_id: wallet.chain_id.clone(),
            address_book_entries: state
                .address_book
                .iter()
                .map(|e| HighRiskChainAddress {
                    chain_id: e.chain_id.clone(),
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
                .fetch_evm_has_contract_code(request.chain_id.clone(), request.to_address.clone())
                .await
                .ok();
            let token_has_code = if let Some(contract) = &request.contract_address {
                self.fetch_evm_has_contract_code(request.chain_id.clone(), contract.clone())
                    .await
                    .ok()
            } else {
                None
            };
            crate::store::evm_recipient_preflight_warnings(
                crate::store::EvmRecipientPreflightRequest {
                    chain_id: chain.str_id().into(),
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
            warnings,
            recipient_warnings,
            requires_self_send_confirmation,
        })
    }
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use crate::registry::Chain;
    use crate::store::state::WalletState;
    use crate::store::wallet_domain::AssetHolding;
    use crate::store::wallet_domain::{CoreTokenPreferenceCategory, CoreTokenPreferenceEntry};

    fn holding(chain: &str, symbol: &str, standard: &str, contract: Option<&str>) -> AssetHolding {
        AssetHolding {
            id: String::new(),
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coingecko_id: String::new(),
            chain_id: chain.to_string(),
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
                chain_id: chain.str_id().to_string(),
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

    /// The two send-support rules, against core's own token list.
    ///
    /// They were iOS predicates whose answers were passed *in* to the
    /// preflight — so on the funds path core took a caller's word about which
    /// assets core itself knows how to send.
    #[test]
    fn solana_sends_sol_always_and_a_token_only_when_known() {
        let sol = holding("solana", "SOL", "Native", None);
        assert!(supports_solana_send(&sol, &[]));

        let standard = Chain::Solana.token_standard().to_string();
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let usdc = holding("solana", "USDC", &standard, Some(mint));
        assert!(!supports_solana_send(&usdc, &[]), "unknown mint");
        assert!(supports_solana_send(&usdc, &[known(Chain::Solana, mint)]));

        let wrong_standard = holding("solana", "USDC", "ERC-20", Some(mint));
        assert!(!supports_solana_send(
            &wrong_standard,
            &[known(Chain::Solana, mint)]
        ));
    }

    #[test]
    fn near_sends_known_tokens_but_not_near_itself() {
        let standard = Chain::Near.token_standard().to_string();
        let native = holding("near", "NEAR", &standard, Some("wrap.near"));
        assert!(
            !supports_near_token_send(&native, &[]),
            "native is not a token send"
        );

        let token = holding("near", "USDC", &standard, Some("usdc.near"));
        assert!(!supports_near_token_send(&token, &[]));
        assert!(
            supports_near_token_send(&token, &[known(Chain::Near, "USDC.NEAR")]),
            "contract matching is case-insensitive"
        );
    }

    /// Knowing a mint is what makes a Solana token routable — through the
    /// service, from core's own token list.
    ///
    /// The preview path and the submit path both asked this on their own side
    /// before, from a Swift copy of the rule. Three answers to one question is
    /// three chances to disagree about whether a send can be made.
    #[tokio::test]
    async fn routing_follows_the_token_list_core_holds() {
        let service = WalletService::new(Vec::new()).expect("service");
        let mint = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let standard = Chain::Solana.token_standard().to_string();
        {
            let mut state = service.wallet_state.write().await;
            let mut wallet =
                WalletState::single_address("w1", "W", "solana", "SoLaddr", None, false);
            wallet.holdings = vec![holding("solana", "USDC", &standard, Some(mint))];
            state.wallets.push(wallet);
        }

        let untracked = service
            .send_asset_routing(
                "w1".into(),
                "solana:spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            )
            .await
            .expect("the holding is there");
        assert_eq!(
            untracked.submit_kind, None,
            "an untracked mint is not sendable"
        );

        service.wallet_state.write().await.token_preferences = vec![known(Chain::Solana, mint)];
        let known_now = service
            .send_asset_routing(
                "w1".into(),
                "solana:spl:EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            )
            .await
            .expect("the holding is there");
        assert_eq!(known_now.submit_kind.as_deref(), Some("solana"));
        assert_eq!(
            known_now.preview_kind, known_now.submit_kind,
            "the preview and the submit are one decision"
        );
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
            let mut wallet =
                WalletState::single_address("w1", "W", "bitcoin", "bc1qowner", None, false);
            wallet.holdings = vec![holding("bitcoin", "BTC", "Native", None)];
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
        assert_eq!(plan.chain_id, "bitcoin");
        assert_eq!(plan.symbol, "BTC");
        assert_eq!(plan.amount, 1.0);
    }
}

#[cfg(test)]
mod send_token_identity_tests {
    use super::send_token_identity;
    use crate::store::wallet_domain::{
        AssetHolding, CoreTokenPreferenceCategory, CoreTokenPreferenceEntry,
    };

    fn entry(chain: &str, symbol: &str, contract: &str, decimals: u32) -> CoreTokenPreferenceEntry {
        CoreTokenPreferenceEntry {
            token: crate::tokens::TokenDeploymentEntry {
                deployment_id: "fixture:token".into(),
                token_id: "fixture:token".into(),
                kind: crate::tokens::TokenKind::Protocol {
                    standard: "fixture".into(),
                    identifier: "fixture".into(),
                },
                chain_id: chain.to_string(),
                name: symbol.to_string(),
                symbol: symbol.to_string(),
                token_standard: "trc20".to_string(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                coinpaprika_id: String::new(),
                decimals,
                tags: Vec::new(),
                color: None,
                artwork_name: String::new(),
                enabled: true,
            },
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: true,
            is_enabled: true,
        }
    }

    fn holding(chain: &str, symbol: &str, contract: Option<&str>) -> AssetHolding {
        AssetHolding {
            id: String::new(),
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            coingecko_id: String::new(),
            chain_id: chain.to_string(),
            token_standard: "trc20".to_string(),
            contract_address: contract.map(str::to_string),
            amount: "1".into(),
        }
    }

    /// A token carries its own decimals, and a native asset carries none.
    ///
    /// Every submit branch resolved this itself. Tron's hard-coded six for
    /// every token on it — right for USDT, wrong for the four
    /// eighteen-decimal ones — and NEAR's fell back to six for a token it
    /// could not find, which is a scale guessed on the funds path.
    #[test]
    fn a_token_carries_its_own_decimals_and_a_native_asset_none() {
        let preferences = vec![
            entry("tron", "USDT", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", 6),
            entry("tron", "USDD", "TPYmHEhy5n8TCEfYGqW2rPxsghSfzghPDn", 18),
        ];

        // The native asset is not a token.
        assert!(send_token_identity(&holding("tron", "TRX", None), &preferences).is_none());

        // Each token's own scale, matched by contract.
        let usdd = send_token_identity(
            &holding("tron", "USDD", Some("TPYmHEhy5n8TCEfYGqW2rPxsghSfzghPDn")),
            &preferences,
        )
        .expect("USDD is tracked");
        assert_eq!(usdd.decimals, 18);
        let usdt = send_token_identity(
            &holding("tron", "USDT", Some("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t")),
            &preferences,
        )
        .expect("USDT is tracked");
        assert_eq!(usdt.decimals, 6);

        // A ticker is not sufficient identity for a protocol token.
        assert!(send_token_identity(&holding("tron", "USDD", None), &preferences).is_none());

        // A token nothing tracks has no identity, so the send refuses rather
        // than guessing a scale.
        assert!(send_token_identity(&holding("tron", "NOPE", None), &preferences).is_none());
        assert!(
            send_token_identity(
                &holding("tron", "USDT", Some("TSomeOtherContractAddressEntirely")),
                &preferences
            )
            .is_none()
        );
        // And a chain that hosts no known tokens has none either.
        assert!(send_token_identity(&holding("monero", "XMR", None), &preferences).is_none());
    }
}
