//! Send decisions over owned wallet identity. UI inputs contain only user edits.
use super::*;
use crate::send::flow::SendPreview;

/// A quote bound to the stored holding that produced it. Clients render these
/// derived values; they never supply asset metadata to reinterpret a preview.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct OwnedSendPreview {
    pub wallet_id: String,
    pub holding_key: String,
    pub chain_id: crate::registry::Chain,
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
    /// What the destination is, checked beside the quote. `None` when no
    /// destination was given.
    pub recipient: Option<RecipientCheck>,
}

/// The share of the fee-adjusted maximum each amount shortcut fills in; 100
/// is the maximum itself. Every quote carries an amount for each of these
/// that it can afford.
const SEND_AMOUNT_SHORTCUT_PERCENTAGES: [u32; 4] = [25, 50, 75, 100];

/// What a shortcut fills in. The maximum is exact to the last unit, since
/// anything less leaves dust behind. A share is cut to the asset's display
/// precision instead: "0.012490233065563998" is not an amount anyone meant,
/// and cutting only ever lowers it, so it stays affordable.
fn shortcut_amount(exact: String, percent: u32, asset_decimals: u32) -> String {
    if percent >= 100 {
        return exact;
    }
    match crate::formatting::format_asset_amount(exact.clone(), asset_decimals) {
        Some(text) if !text.below_threshold => text.value,
        _ => exact,
    }
}

/// The amount shortcuts a composer offers, in order, whether or not a quote
/// has priced them yet — so the controls exist before the first quote does.
#[uniffi::export]
pub fn send_amount_shortcut_percentages() -> Vec<u32> {
    SEND_AMOUNT_SHORTCUT_PERCENTAGES.to_vec()
}

/// What a quote knows about its destination before anything is built.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct RecipientCheck {
    /// Whether the destination has been used, from raw smallest-unit
    /// balances. `None` when its balance or history could not be read.
    pub activity: Option<super::types::SendDestinationActivity>,
    /// The destination is an address of one of the user's own wallets on
    /// this network. Read from local state, so a failed activity read does
    /// not hide it; the build review asks for confirmation on the same fact.
    pub is_own_address: bool,
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
        let exact = |v: Option<String>| v.and_then(|d| crate::decimal::truncate(&d, decimals));
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
    let gas_decimals = u32::from(chain.native_decimals());
    let network_fee = crate::decimal::truncate(preview.network_fee(), gas_decimals);
    let network_fee_value = network_fee.as_deref().and_then(|fee| {
        super::valuation::display_value_of(state, &chain.native_holding_template(), fee)
    });
    let amount_value = super::valuation::display_value_of(state, holding, amount);
    let shortcuts = SEND_AMOUNT_SHORTCUT_PERCENTAGES
        .into_iter()
        .filter_map(|percent| {
            let exact = crate::send::flow::quoted_send_amount(
                Some(preview.clone()),
                chain,
                is_native,
                decimals,
                percent,
            )?;
            Some((
                percent,
                shortcut_amount(exact, percent, decimals.unwrap_or(gas_decimals)),
            ))
        })
        .collect();
    let mut details = crate::send::flow::compute_send_preview_details(
        Some(preview.clone()),
        Some(&holding.amount),
    );
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
        chain_id: chain,
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
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let recipient = async {
                if destination.trim().is_empty() {
                    return None;
                }
                let risk = this.send_destination_risk(
                    wallet_id.clone(),
                    holding_key.clone(),
                    destination.clone(),
                );
                let own = async {
                    let (chain, _) = this
                        .destination_probe_target(&wallet_id, &holding_key)
                        .await?;
                    let address = this
                        .resolve_send_destination(chain, destination.clone())
                        .await?
                        .address;
                    this.is_own_address(chain, &address).await
                };
                let (risk, own) = tokio::join!(risk, own);
                Some(RecipientCheck {
                    activity: risk.ok().map(|risk| risk.activity),
                    is_own_address: own.unwrap_or(false),
                })
            };
            let quote = this.preview_quote_only(
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
        })
        .await
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
            .ok_or_else(|| SpectraBridgeError::failure("wallet does not exist"))?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or_else(|| SpectraBridgeError::failure("holding does not exist"))?;
        let (network, token) =
            super::send_destination::destination_probe_asset(holding, &state.token_preferences)?;
        let chain = super::send_execution::send_chain_for(&state, &wallet_id, network)?;
        if !crate::send::SendAsset::of(holding, &state.token_preferences)
            .is_some_and(|asset| asset.is_sendable())
        {
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
            return Err(SpectraBridgeError::failure(
                "EVM fee inputs require an EVM asset",
            ));
        }
        let decimals = token
            .as_ref()
            .map(|t| u32::from(t.decimals))
            .unwrap_or(u32::from(chain.native_decimals()));
        if crate::send::amount_input::parse_raw_amount(&amount, decimals)? == 0 {
            return Err(SpectraBridgeError::failure("amount must be positive"));
        }
        let address = wallet
            .address_on(chain)
            .ok_or_else(|| {
                SpectraBridgeError::failure("wallet has no address on selected network")
            })?
            .to_string();
        let destination = if destination.trim().is_empty() {
            String::new()
        } else {
            self.resolve_send_destination(chain, destination)
                .await?
                .address
        };
        let preview = match chain.mainnet_counterpart() {
            Chain::Bitcoin => {
                if let Some(xpub) = wallet.xpub.as_ref().filter(|x| !x.trim().is_empty()) {
                    self.fetch_bitcoin_hd_send_preview(chain, xpub.clone(), 20, 20)
                        .await?
                } else {
                    self.fetch_utxo_fee_preview(chain, address, 0, destination)
                        .await?
                }
                .map(|preview| SendPreview::Utxo { preview })
            }
            Chain::BitcoinCash | Chain::BitcoinSV | Chain::Litecoin => self
                .fetch_utxo_fee_preview(chain, address, 0, destination)
                .await?
                .map(|preview| SendPreview::Utxo { preview }),
            Chain::Dogecoin => {
                // This legacy provider preview accepts a display amount; signing parses the exact input separately.
                self.fetch_dogecoin_send_preview(address, amount.clone())
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
            _ => self
                .fetch_simple_chain_send_preview(chain, address)
                .await?
                .map(Into::into),
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
    fn network_fee(&self) -> &str {
        match self {
            Self::Utxo { preview } => &preview.estimatedNetworkFee,
            Self::Dogecoin { preview } => &preview.estimatedNetworkFee,
            Self::Ethereum { preview } => &preview.estimatedNetworkFee,
            Self::Tron { preview } => &preview.estimatedNetworkFee,
            Self::Solana { preview } => &preview.estimatedNetworkFee,
            Self::Xrp { preview } => &preview.estimatedNetworkFee,
            Self::Stellar { preview } => &preview.estimatedNetworkFee,
            Self::Monero { preview } => &preview.estimatedNetworkFee,
            Self::Cardano { preview } => &preview.estimatedNetworkFee,
            Self::Sui { preview } => &preview.estimatedNetworkFee,
            Self::Aptos { preview } => &preview.estimatedNetworkFee,
            Self::Ton { preview } => &preview.estimatedNetworkFee,
            Self::Icp { preview } => &preview.estimatedNetworkFee,
            Self::Near { preview } => &preview.estimatedNetworkFee,
            Self::Polkadot { preview } => &preview.estimatedNetworkFee,
            Self::Bittensor { preview } => &preview.estimatedNetworkFee,
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
            .ok_or_else(|| SpectraBridgeError::failure("wallet does not exist"))?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or_else(|| SpectraBridgeError::failure("holding does not exist"))?;
        let chain = holding.chain_id;
        super::send_execution::send_chain_for(&state, &wallet_id, chain)?;
        if let Some(input) = &overrides {
            input.resolve(chain)?;
        }
        let destination = self
            .resolve_send_destination(chain, destination)
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
            .map(|preview| preview.network_fee().to_string())
            .or(preflight.token_send_gas_reserve)
            .or_else(|| shape.fee_fallback.map(str::to_string))
            .or_else(|| {
                (shape.fee_field == crate::registry::SendFeeField::None).then(|| "0".into())
            })
            .ok_or_else(|| SpectraBridgeError::failure("Unable to estimate network fee"))?;
        let fee = crate::decimal::canonical(&fee)
            .ok_or_else(|| SpectraBridgeError::failure("invalid network fee"))?;
        let latest = self.app_state().await;
        let wallet = latest
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or_else(|| SpectraBridgeError::failure("wallet was removed"))?;
        super::send_execution::send_chain_for(&latest, &wallet_id, chain)?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or_else(|| SpectraBridgeError::failure("holding was removed"))?;
        let verdict = crate::send::send_affordability(crate::send::SendAffordabilityInput {
            is_native: holding.is_native(),
            chain_id: chain,
            symbol: holding.symbol.clone(),
            amount: preflight.amount.clone(),
            network_fee: fee.clone(),
            holding_balance: holding.amount.clone(),
            gas_balance: wallet
                .holdings
                .iter()
                .find(|h| h.is_native() && h.chain_id == chain)
                .map(|h| h.amount.clone()),
        });
        use crate::send::SendAffordability;
        match verdict {
            SendAffordability::Affordable => {}
            SendAffordability::Unavailable => {
                return Err(SpectraBridgeError::failure(
                    "Unable to determine the available gas balance",
                ));
            }
            SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } => {
                return Err(SpectraBridgeError::failed(
                    "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                    [symbol.as_str(), required.as_str(), symbol.as_str()],
                ));
            }
            SendAffordability::AmountExceedsBalance { symbol } => {
                return Err(SpectraBridgeError::failed(
                    "Insufficient %@ balance.",
                    [symbol],
                ));
            }
            SendAffordability::FeeExceedsGasBalance {
                gas_symbol,
                fee,
                chain_id,
            } => {
                let network = chain_id.chain_display_name();
                return Err(SpectraBridgeError::failed(
                    "Insufficient %@ for the %@ network fee (%@ %@).",
                    [
                        gas_symbol.as_str(),
                        network,
                        fee.as_str(),
                        gas_symbol.as_str(),
                    ],
                ));
            }
        }
        let fee_rate_svb = match &preview {
            Some(SendPreview::Utxo { preview })
                if chain.mainnet_counterpart() == Chain::Bitcoin =>
            {
                Some(preview.estimatedFeeRateSatVb.to_string())
            }
            Some(SendPreview::Dogecoin { preview }) => {
                Some(preview.estimatedFeeRateDogePerKb.clone())
            }
            _ => None,
        };
        use crate::registry::SendFeeField;
        Ok(OwnedSendQuote {
            request: crate::send::SendExecutionRequest {
                chain_id: chain,
                wallet_id,
                password: None,
                to_address: destination,
                amount_str: amount,
                contract_address: preflight.token_contract_address,
                token_decimals: preflight.token_decimals,
                fee_rate_svb,
                fee_sat: if shape.fee_field == SendFeeField::FeeSats {
                    Some(crate::send::payload::fee_units(
                        &fee,
                        chain.native_decimals().into(),
                    )?)
                } else {
                    None
                },
                gas_budget: (shape.fee_field == SendFeeField::GasBudget).then(|| fee.clone()),
                fee_amount: (shape.fee_field == SendFeeField::FeeAmount).then(|| fee.clone()),
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
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let pending = this
                .replaceable_sends()
                .await?
                .into_iter()
                .find(|p| p.transaction_id.eq_ignore_ascii_case(&transaction_id))
                .ok_or_else(|| {
                    SpectraBridgeError::failure("transaction is no longer replaceable")
                })?;
            if !cancel && !pending.can_speed_up {
                return Err(crate::SpectraBridgeError::failure(
                    "This token transfer cannot be reconstructed; cancel it instead",
                ));
            }
            let state = this.app_state().await;
            let chain = pending.chain_id;
            let wallet = state
                .wallets
                .iter()
                .find(|w| w.id.eq_ignore_ascii_case(&pending.wallet_id))
                .ok_or_else(|| SpectraBridgeError::failure("wallet does not exist"))?;
            super::send_execution::send_chain_for(&state, &wallet.id, chain)?;
            let holding = wallet
                .holdings
                .iter()
                .find(|h| h.is_native() && h.chain_id == chain)
                .ok_or_else(|| {
                    SpectraBridgeError::failure(
                        "wallet has no native holding on transaction network",
                    )
                })?;
            let destination = if cancel {
                wallet
                    .address_on(chain)
                    .ok_or_else(|| {
                        SpectraBridgeError::failure("wallet has no address on transaction network")
                    })?
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
                this.fetch_evm_tx_nonce(pending.chain_id, pending.transaction_hash)
                    .await?,
            )
            .map_err(|_| SpectraBridgeError::failure("nonce exceeds supported range"))?;
            let preview = this
                .preview_owned_evm_send(
                    wallet.id.clone(),
                    holding.deployment_id(),
                    amount.clone(),
                    destination.clone(),
                    Some(nonce),
                    None,
                )
                .await?
                .ok_or_else(|| {
                    SpectraBridgeError::failure("Unable to estimate replacement fees")
                })?;
            let bump = crate::send::flow::evm_replacement_fee_bump(
                &preview.maxFeePerGasGwei,
                &preview.maxPriorityFeePerGasGwei,
            )
            .ok_or_else(|| SpectraBridgeError::failure("Unable to estimate replacement fees"))?;
            Ok(OwnedReplacementDraft {
                wallet_id: wallet.id.clone(),
                holding_key: holding.deployment_id(),
                destination,
                amount,
                nonce,
                max_fee_gwei: bump.max_fee_gwei,
                priority_fee_gwei: bump.priority_fee_gwei,
            })
        })
        .await
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
            .ok_or_else(|| SpectraBridgeError::failure("wallet does not exist"))?
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or_else(|| SpectraBridgeError::failure("holding does not exist"))?;
        let chain = holding.chain_id;
        super::send_execution::send_chain_for(&state, &wallet_id, chain)?;
        let destination = self
            .resolve_send_destination(chain, destination)
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
        let normalize =
            |address: &str| crate::send::flow::normalized_send_address(chain, address.into());
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
                self.owned_addresses_for_wallet(wallet.id.clone(), Some(chain))
                    .await,
            );
            if chain.supports_deep_utxo_discovery() && wallet.chain_id == chain {
                owned.extend(self.known_utxo_addresses(wallet.id.clone(), chain).await?);
            }
        }
        Ok(owned)
    }
}

#[cfg(test)]
mod shortcut_amount_tests {
    use super::shortcut_amount;

    #[test]
    fn shares_are_cut_to_display_precision_and_the_maximum_is_exact() {
        let exact = "0.012490233065563998".to_string();
        assert_eq!(shortcut_amount(exact.clone(), 25, 18), "0.0124902");
        assert_eq!(shortcut_amount(exact.clone(), 100, 18), exact);
        // Dust has no display form; it stays exact rather than become zero.
        let dust = "0.000000000000000001".to_string();
        assert_eq!(shortcut_amount(dust.clone(), 25, 18), dust);
    }
}

#[cfg(test)]
mod quote_projection_tests {
    use super::*;
    #[test]
    fn a_native_fee_quote_does_not_claim_a_token_balance_or_maximum() {
        let preview = SendPreview::Solana {
            preview: crate::send::preview_types::SolanaSendPreview {
                spendableBalance: "10".into(),
                maxSendable: "9".into(),
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
