//! Send decisions over owned wallet identity. UI inputs contain only user edits.
use super::*;
use crate::send::flow::SendPreview;

/// A quote bound to the stored holding that produced it. Clients render these
/// derived values; they never supply asset metadata to reinterpret a preview.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct OwnedSendPreview {
    pub wallet_id: String,
    pub holding_key: String,
    pub chain_id: String,
    /// The amount quoted, exactly as it was asked for. A value or fee shown
    /// beside a different amount field belongs to another quote.
    pub amount: String,
    pub preview: SendPreview,
    /// The estimated fee in the chain's gas asset, as an exact decimal cut
    /// to the gas asset's precision.
    pub network_fee: Option<String>,
    /// That fee in the display currency, when the gas asset has a quote.
    pub network_fee_value: Option<f64>,
    /// The amount being quoted, in the display currency.
    pub amount_value: Option<f64>,
    pub details: Option<SendPreviewDetails>,
    pub shortcuts: HashMap<u32, String>,
    /// What the destination's own history says, checked beside the quote.
    /// `None` when no destination was given.
    pub recipient: Option<RecipientCheck>,
}

/// Whether the destination has been used, from raw smallest-unit balances.
#[derive(Debug, Clone, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RecipientCheck {
    Checked {
        activity: super::types::SendDestinationActivity,
    },
    /// The destination's balance or history could not be read.
    Unavailable,
}

/// What a preview says about the funds, beyond the fee. Amounts are exact
/// decimals in the sent asset, cut to its precision and never rounded up.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendPreviewDetails {
    pub spendable_balance: Option<String>,
    pub fee_rate_description: Option<String>,
    pub estimated_transaction_bytes: Option<i64>,
    pub selected_input_count: Option<i64>,
    pub uses_change_output: Option<bool>,
    pub max_sendable: Option<String>,
}

impl SendPreviewDetails {
    fn from_core(core: crate::send::flow::SendPreviewDetailsCore, decimals: u32) -> Self {
        let exact = |v: Option<f64>| {
            v.and_then(crate::decimal::from_f64)
                .and_then(|d| crate::decimal::truncate(&d, decimals))
        };
        Self {
            spendable_balance: exact(core.spendableBalance),
            fee_rate_description: core.feeRateDescription,
            estimated_transaction_bytes: core.estimatedTransactionBytes,
            selected_input_count: core.selectedInputCount,
            uses_change_output: core.usesChangeOutput,
            max_sendable: exact(core.maxSendable),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn owned_preview(
    state: &CoreAppState,
    wallet_id: String,
    holding: &crate::store::wallet_domain::AssetHolding,
    amount: &str,
    chain: Chain,
    decimals: Option<u32>,
    preview: SendPreview,
) -> OwnedSendPreview {
    let holding_key = holding.deployment_id();
    let is_native = holding.is_native();
    let balance = crate::decimal::to_f64(&holding.amount);
    let gas_decimals = u32::from(chain.native_decimals());
    let network_fee = crate::decimal::from_f64(preview.network_fee())
        .and_then(|fee| crate::decimal::truncate(&fee, gas_decimals));
    let network_fee_value = network_fee.as_deref().and_then(|fee| {
        super::valuation::display_value_of(state, &chain.native_holding_template(), fee)
    });
    let amount_value = super::valuation::display_value_of(state, holding, amount);
    let shortcuts = [25, 50, 75, 100]
        .into_iter()
        .filter_map(|percent| {
            crate::send::flow::quoted_send_amount(
                Some(preview.clone()),
                chain.str_id().into(),
                is_native,
                decimals,
                percent,
            )
            .map(|amount| (percent, amount))
        })
        .collect();
    let mut details =
        crate::send::flow::compute_send_preview_details(Some(preview.clone()), balance);
    if !is_native
        && !matches!(
            preview,
            SendPreview::Ethereum { .. } | SendPreview::Tron { .. }
        )
        && let Some(details) = &mut details
    {
        details.spendableBalance = None;
        details.maxSendable = None;
    }
    let asset_decimals = decimals.unwrap_or(gas_decimals);
    OwnedSendPreview {
        wallet_id,
        holding_key,
        chain_id: chain.str_id().into(),
        amount: amount.to_string(),
        preview,
        network_fee,
        network_fee_value,
        amount_value,
        details: details.map(|d| SendPreviewDetails::from_core(d, asset_decimals)),
        shortcuts,
        recipient: None,
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Quote a send and, alongside it, check whether the destination has been
    /// used. A recipient read that fails leaves the quote standing.
    pub async fn preview_owned_send(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: String,
        destination: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::send::ethereum::EvmCustomFeeConfiguration>,
    ) -> Result<Option<OwnedSendPreview>, SpectraBridgeError> {
        let recipient = async {
            if destination.trim().is_empty() {
                return None;
            }
            Some(
                match self
                    .send_destination_risk(
                        wallet_id.clone(),
                        holding_key.clone(),
                        destination.clone(),
                    )
                    .await
                {
                    Ok(risk) => RecipientCheck::Checked {
                        activity: risk.activity,
                    },
                    Err(_) => RecipientCheck::Unavailable,
                },
            )
        };
        let quote = self.preview_quote_only(
            wallet_id.clone(),
            holding_key.clone(),
            amount,
            destination.clone(),
            explicit_nonce,
            custom_fees,
        );
        let (quote, recipient) = tokio::join!(quote, recipient);
        Ok(quote?.map(|preview| OwnedSendPreview {
            recipient,
            ..preview
        }))
    }
}

impl WalletService {
    async fn preview_quote_only(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: String,
        destination: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::send::ethereum::EvmCustomFeeConfiguration>,
    ) -> Result<Option<OwnedSendPreview>, SpectraBridgeError> {
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("wallet does not exist")?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or("holding does not exist")?;
        let (network, token) =
            super::send_destination::destination_probe_asset(holding, &state.token_preferences)?;
        let chain = super::send_execution::send_chain_for(&state, &wallet_id, network)?;
        let route = self
            .send_asset_routing(wallet_id.clone(), holding_key.clone())
            .await
            .ok_or("asset has no route")?;
        if route.preview_kind.is_none() {
            return Ok(None);
        }
        let token_decimals = token.as_ref().map(|t| u32::from(t.decimals));
        let wrap = |preview| {
            owned_preview(
                &state,
                wallet_id.clone(),
                holding,
                &amount,
                chain,
                token_decimals,
                preview,
            )
        };
        if chain.is_evm() {
            return Ok(self
                .preview_owned_evm_send(
                    wallet_id.clone(),
                    holding_key.clone(),
                    amount.clone(),
                    destination,
                    explicit_nonce,
                    custom_fees,
                )
                .await?
                .map(|preview| wrap(SendPreview::Ethereum { preview })));
        }
        if explicit_nonce.is_some() || custom_fees.is_some() {
            return Err("EVM fee inputs require an EVM asset".into());
        }
        let decimals = token
            .as_ref()
            .map(|t| u32::from(t.decimals))
            .unwrap_or(u32::from(chain.native_decimals()));
        if crate::send::amount_input::parse_raw_amount(&amount, decimals)? == 0 {
            return Err("amount must be positive".into());
        }
        let address = wallet
            .address_on(chain)
            .ok_or("wallet has no address on selected network")?
            .to_string();
        let destination = if destination.trim().is_empty() {
            String::new()
        } else {
            self.resolve_send_destination(chain.str_id().into(), destination)
                .await?
                .address
        };
        let preview = match chain.mainnet_counterpart() {
            Chain::Bitcoin => {
                if let Some(xpub) = wallet.xpub.as_ref().filter(|x| !x.trim().is_empty()) {
                    self.fetch_bitcoin_hd_send_preview(chain.str_id().into(), xpub.clone(), 20, 20)
                        .await?
                } else {
                    self.fetch_utxo_fee_preview(chain.str_id().into(), address, 0, destination)
                        .await?
                }
                .map(|preview| SendPreview::Utxo { preview })
            }
            Chain::BitcoinCash | Chain::BitcoinSV | Chain::Litecoin => self
                .fetch_utxo_fee_preview(chain.str_id().into(), address, 0, destination)
                .await?
                .map(|preview| SendPreview::Utxo { preview }),
            Chain::Dogecoin => {
                let priority = state
                    .settings
                    .fee_priority_by_chain
                    .get(chain.str_id())
                    .copied()
                    .unwrap_or(crate::store::state::FeePriority::Normal);
                // This legacy provider preview accepts a display amount; signing parses the exact input separately.
                self.fetch_dogecoin_send_preview(
                    address,
                    amount.parse().map_err(|_| "invalid amount")?,
                    priority.as_raw().to_string(),
                )
                .await?
                .map(|preview| SendPreview::Dogecoin { preview })
            }
            Chain::Tron => self
                .fetch_tron_send_preview(
                    address,
                    holding.symbol.clone(),
                    token.map(|t| t.contract).unwrap_or_default(),
                )
                .await?
                .map(|preview| SendPreview::Tron { preview }),
            _ => Some(
                self.fetch_simple_chain_send_preview(chain.str_id().into(), address)
                    .await?
                    .into(),
            ),
        };
        Ok(preview.map(wrap))
    }
}

impl From<crate::send::preview_decode::SimpleChainPreview> for SendPreview {
    fn from(value: crate::send::preview_decode::SimpleChainPreview) -> Self {
        use crate::send::preview_decode::SimpleChainPreview as P;
        match value {
            P::Solana { preview } => Self::Solana { preview },
            P::Xrp { preview } => Self::Xrp { preview },
            P::Stellar { preview } => Self::Stellar { preview },
            P::Monero { preview } => Self::Monero { preview },
            P::Cardano { preview } => Self::Cardano { preview },
            P::Sui { preview } => Self::Sui { preview },
            P::Aptos { preview } => Self::Aptos { preview },
            P::Ton { preview } => Self::Ton { preview },
            P::Icp { preview } => Self::Icp { preview },
            P::Near { preview } => Self::Near { preview },
            P::Polkadot { preview } => Self::Polkadot { preview },
            P::Bittensor { preview } => Self::Bittensor { preview },
        }
    }
}

impl SendPreview {
    fn network_fee(&self) -> f64 {
        match self {
            Self::Utxo { preview } => preview.estimatedNetworkFee,
            Self::Dogecoin { preview } => preview.estimatedNetworkFee,
            Self::Ethereum { preview } => preview.estimatedNetworkFee,
            Self::Tron { preview } => preview.estimatedNetworkFee,
            Self::Solana { preview } => preview.estimatedNetworkFee,
            Self::Xrp { preview } => preview.estimatedNetworkFee,
            Self::Stellar { preview } => preview.estimatedNetworkFee,
            Self::Monero { preview } => preview.estimatedNetworkFee,
            Self::Cardano { preview } => preview.estimatedNetworkFee,
            Self::Sui { preview } => preview.estimatedNetworkFee,
            Self::Aptos { preview } => preview.estimatedNetworkFee,
            Self::Ton { preview } => preview.estimatedNetworkFee,
            Self::Icp { preview } => preview.estimatedNetworkFee,
            Self::Near { preview } => preview.estimatedNetworkFee,
            Self::Polkadot { preview } => preview.estimatedNetworkFee,
            Self::Bittensor { preview } => preview.estimatedNetworkFee,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct OwnedSendQuote {
    pub request: crate::send::SendExecutionRequest,
    pub preview: Option<SendPreview>,
}

impl WalletService {
    /// Resolve fees, token identity and affordability using owned state. No signing.
    pub async fn quote_owned_send(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: String,
        destination: String,
        overrides: Option<crate::send::ethereum::EvmSendOverridesInput>,
    ) -> Result<OwnedSendQuote, SpectraBridgeError> {
        let preflight = self
            .send_submit_preflight(
                wallet_id.clone(),
                holding_key.clone(),
                destination.clone(),
                amount.clone(),
            )
            .await?;
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("wallet does not exist")?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or("holding does not exist")?;
        let chain = holding.chain().ok_or("invalid network")?;
        super::send_execution::send_chain_for(&state, &wallet_id, chain)?;
        if let Some(input) = &overrides {
            input.resolve(chain)?;
        }
        let destination = self
            .resolve_send_destination(chain.str_id().into(), destination)
            .await?
            .address;
        let preview = self
            .preview_owned_send(
                wallet_id.clone(),
                holding_key.clone(),
                amount.clone(),
                destination.clone(),
                overrides.as_ref().and_then(|o| o.nonce),
                overrides.as_ref().and_then(|o| o.custom_fees.clone()),
            )
            .await?
            .map(|quote| quote.preview);
        let shape = chain.send_execution_shape();
        let fee = preview
            .as_ref()
            .map(SendPreview::network_fee)
            .or(preflight.token_send_gas_reserve)
            .or_else(|| (shape.fee_fallback > 0.0).then_some(shape.fee_fallback))
            .or_else(|| (shape.fee_field == crate::registry::SendFeeField::None).then_some(0.0))
            .ok_or("Unable to estimate network fee")?;
        if !fee.is_finite() || fee < 0.0 {
            return Err("invalid network fee".into());
        }
        let latest = self.app_state().await;
        let wallet = latest
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("wallet was removed")?;
        super::send_execution::send_chain_for(&latest, &wallet_id, chain)?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or("holding was removed")?;
        let verdict = crate::send::send_affordability(crate::send::SendAffordabilityInput {
            is_native: holding.is_native(),
            chain_id: chain.str_id().into(),
            symbol: holding.symbol.clone(),
            amount: preflight.amount_str.clone(),
            network_fee: crate::decimal::from_f64(fee).ok_or("invalid network fee")?,
            holding_balance: holding.amount.clone(),
            gas_balance: wallet
                .holdings
                .iter()
                .find(|h| h.is_native() && h.chain() == Some(chain))
                .map(|h| h.amount.clone()),
        });
        use crate::send::SendAffordability;
        match verdict {
            SendAffordability::Affordable => {}
            SendAffordability::Unavailable => return Err("Unable to determine the available gas balance".into()),
            SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } =>
                return Err(format!("Insufficient {symbol} for amount plus network fee (requires {required} {symbol})").into()),
            SendAffordability::AmountExceedsBalance { symbol } =>
                return Err(format!("Insufficient {symbol} balance").into()),
            SendAffordability::FeeExceedsGasBalance { gas_symbol, fee, chain_id } => {
                let network = crate::registry::Chain::display_name_for_id(&chain_id);
                return Err(format!("Insufficient {gas_symbol} for the {network} network fee ({fee} {gas_symbol})").into());
            }
        }
        let fee_rate_svb = match &preview {
            Some(SendPreview::Utxo { preview })
                if chain.mainnet_counterpart() == Chain::Bitcoin =>
            {
                Some(preview.estimatedFeeRateSatVb as f64)
            }
            Some(SendPreview::Dogecoin { preview }) => Some(preview.estimatedFeeRateDogePerKb),
            _ => None,
        };
        use crate::registry::SendFeeField;
        Ok(OwnedSendQuote {
            request: crate::send::SendExecutionRequest {
                chain_id: chain.str_id().into(),
                wallet_id,
                password: None,
                to_address: destination,
                amount_str: amount,
                contract_address: preflight.token_contract_address,
                token_decimals: preflight.token_decimals,
                fee_rate_svb,
                fee_sat: if shape.fee_field == SendFeeField::FeeSats {
                    Some(crate::send::payload::fee_units(
                        fee,
                        chain.native_decimals().into(),
                    )?)
                } else {
                    None
                },
                gas_budget: (shape.fee_field == SendFeeField::GasBudget).then_some(fee),
                fee_amount: (shape.fee_field == SendFeeField::FeeAmount).then_some(fee),
                evm_overrides: overrides,
                monero_priority: None,
                sign_only: false,
            },
            preview,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct OwnedReplacementDraft {
    pub wallet_id: String,
    pub holding_key: String,
    pub destination: String,
    pub amount: String,
    pub nonce: i64,
    pub max_fee_gwei: String,
    pub priority_fee_gwei: String,
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Reconstruct a replacement from the stored pending transaction, never UI metadata.
    pub async fn replacement_draft(
        &self,
        transaction_id: String,
        cancel: bool,
    ) -> Result<OwnedReplacementDraft, SpectraBridgeError> {
        let pending = self
            .replaceable_sends()
            .await?
            .into_iter()
            .find(|p| p.transaction_id.eq_ignore_ascii_case(&transaction_id))
            .ok_or("transaction is no longer replaceable")?;
        if !cancel && !pending.can_speed_up {
            return Err("This token transfer cannot be reconstructed; cancel it instead".into());
        }
        let state = self.app_state().await;
        let chain = Chain::from_str_id(&pending.chain_id).ok_or("invalid transaction network")?;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id.eq_ignore_ascii_case(&pending.wallet_id))
            .ok_or("wallet does not exist")?;
        super::send_execution::send_chain_for(&state, &wallet.id, chain)?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.is_native() && h.chain() == Some(chain))
            .ok_or("wallet has no native holding on transaction network")?;
        let destination = if cancel {
            wallet
                .address_on(chain)
                .ok_or("wallet has no address on transaction network")?
                .into()
        } else {
            pending.to_address
        };
        // Stored amount precision is retained; eight-decimal formatting lost value.
        let amount = if cancel {
            "0".into()
        } else {
            pending.amount.to_string()
        };
        let nonce = i64::try_from(
            self.fetch_evm_tx_nonce(pending.chain_id, pending.transaction_hash)
                .await?,
        )
        .map_err(|_| "nonce exceeds supported range")?;
        let preview = self
            .preview_owned_evm_send(
                wallet.id.clone(),
                holding.deployment_id(),
                amount.clone(),
                destination.clone(),
                Some(nonce),
                None,
            )
            .await?
            .ok_or("Unable to estimate replacement fees")?;
        let bump = crate::send::flow::evm_replacement_fee_bump(
            Some(preview.maxFeePerGasGwei.to_string()),
            Some(preview.maxPriorityFeePerGasGwei.to_string()),
            preview.maxFeePerGasGwei,
            preview.maxPriorityFeePerGasGwei,
        );
        Ok(OwnedReplacementDraft {
            wallet_id: wallet.id.clone(),
            holding_key: holding.deployment_id(),
            destination,
            amount,
            nonce,
            max_fee_gwei: bump.max_fee_gwei,
            priority_fee_gwei: bump.priority_fee_gwei,
        })
    }
}

impl WalletService {
    /// Whether a send to `destination` goes to one of the user's own
    /// addresses on the holding's network. The CLI's check; review asks
    /// [`Self::is_own_address`] directly with the address it resolved.
    pub async fn is_own_send_destination(
        &self,
        wallet_id: String,
        holding_key: String,
        destination: String,
    ) -> Result<bool, SpectraBridgeError> {
        let state = self.app_state().await;
        let holding = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("wallet does not exist")?
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or("holding does not exist")?;
        let chain = holding.chain().ok_or("invalid asset network")?;
        super::send_execution::send_chain_for(&state, &wallet_id, chain)?;
        let destination = self
            .resolve_send_destination(chain.str_id().into(), destination)
            .await?
            .address;
        self.is_own_address(chain, &destination).await
    }

    /// Whether `destination` is an address of the user's on `chain`, compared
    /// in the chain's normal form — which lowercases an all-caps bech32
    /// address, so one typed in caps is still recognised.
    pub(super) async fn is_own_address(
        &self,
        chain: Chain,
        destination: &str,
    ) -> Result<bool, SpectraBridgeError> {
        let normalize = |address: &str| {
            crate::send::flow::normalized_send_address(chain.str_id().into(), address.into())
        };
        let destination = normalize(destination);
        Ok(self
            .send_owned_addresses(chain)
            .await?
            .iter()
            .any(|address| normalize(address) == destination))
    }

    pub(super) async fn send_owned_addresses(
        &self,
        chain: Chain,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let mut owned = Vec::new();
        for wallet in &self.app_state().await.wallets {
            if let Some(address) = wallet.address_on(chain) {
                owned.push(address.to_string());
            }
            owned.extend(
                self.owned_addresses_for_wallet(wallet.id.clone(), Some(chain.str_id().into()))
                    .await,
            );
            if chain.supports_deep_utxo_discovery() && wallet.chain() == Some(chain) {
                owned.extend(
                    self.known_utxo_addresses(wallet.id.clone(), chain.str_id().into())
                        .await?,
                );
            }
        }
        Ok(owned)
    }
}

#[cfg(test)]
mod quote_projection_tests {
    use super::*;
    #[test]
    fn a_native_fee_quote_does_not_claim_a_token_balance_or_maximum() {
        let preview = SendPreview::Solana {
            preview: crate::send::preview_types::SolanaSendPreview {
                spendableBalance: 10.0,
                maxSendable: 9.0,
                ..Default::default()
            },
        };
        let token = crate::store::wallet_domain::AssetHolding {
            token_standard: Chain::Solana.token_standard().into(),
            contract_address: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into()),
            amount: "100".into(),
            ..Chain::Solana.native_holding_template()
        }
        .identified();
        let quote = owned_preview(
            &CoreAppState::default(),
            "w".into(),
            &token,
            "1",
            Chain::Solana,
            Some(6),
            preview,
        );
        assert!(quote.shortcuts.is_empty());
        let details = quote.details.unwrap();
        assert_eq!(details.spendable_balance, None);
        assert_eq!(details.max_sendable, None);
    }
}
