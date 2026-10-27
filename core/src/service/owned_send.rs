//! Send decisions over owned wallet identity. UI inputs contain only user edits.
use super::*;
use crate::send::preview_types::{SendFeeTerms, SendPreview};

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
    /// The fee is cut to the gas asset's precision, the balance and the
    /// maximum to the sent asset's, never rounded up.
    pub preview: SendPreview,
    /// What leaves the wallet in the sent asset: the amount and the fee, when
    /// the fee is paid in that asset too. `None` for a token, whose fee is
    /// the gas asset's, and while the fee is unknown.
    #[uniffi(default = None)]
    pub total: Option<String>,
    /// That fee in the display currency, when the gas asset has a quote.
    pub network_fee_value: Option<f64>,
    /// The amount being quoted, in the display currency.
    pub amount_value: Option<f64>,
    pub shortcuts: HashMap<u32, String>,
    /// What the destination is, checked beside the quote. `None` when no
    /// destination was given.
    pub recipient: Option<RecipientCheck>,
    /// Why this amount cannot be sent as quoted — more than the balance, or
    /// too little left for the fee — as the build would refuse it. `None`
    /// when the amount and the fee fit, or when the gas balance is unknown.
    #[uniffi(default = None)]
    pub amount_refusal: Option<crate::error::LocalizableMessage>,
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
    match crate::amount::presentation::format_asset_amount(exact.clone(), asset_decimals) {
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
    pub activity: Option<super::SendDestinationActivity>,
    /// The destination is an address of one of the user's own wallets on
    /// this network. Read from local state, so a failed activity read does
    /// not hide it; the build review asks for confirmation on the same fact.
    pub is_own_address: bool,
    /// Neither the address book nor this wallet's history has sent here:
    /// the build's `NewAddress` warning, known before anything is built.
    /// Never set for an own address.
    #[uniffi(default = false)]
    pub is_new_to_wallet: bool,
}

fn owned_preview(
    state: &ResidentState,
    wallet_id: String,
    holding: &crate::store::wallet_domain::AssetHolding,
    amount: &str,
    chain: Chain,
    decimals: Option<u32>,
    mut preview: SendPreview,
) -> Result<OwnedSendPreview, SpectraBridgeError> {
    let holding_key = holding.deployment_id();
    let is_native = holding.is_native();
    let gas_decimals = u32::from(chain.native_decimals());
    let asset_decimals = decimals.unwrap_or(gas_decimals);
    let cut = |value: &str, places: u32| {
        crate::amount::decimal::truncate(value, places)
            .ok_or_else(|| SpectraBridgeError::failure("Send quote is not an exact amount"))
    };
    preview.estimated_network_fee = cut(&preview.estimated_network_fee, gas_decimals)?;
    preview.spendable_balance = cut(&preview.spendable_balance, asset_decimals)?;
    preview.max_sendable = cut(&preview.max_sendable, asset_decimals)?;
    let network_fee = preview.estimated_network_fee.clone();
    let network_fee_value =
        super::valuation::display_value_of(state, &chain.native_holding_template(), &network_fee);
    let amount_value = super::valuation::display_value_of(state, holding, amount);
    // No fallback to the portfolio's balance: a quote without a maximum
    // offers no shortcut rather than the whole holding.
    let shortcuts = SEND_AMOUNT_SHORTCUT_PERCENTAGES
        .into_iter()
        .filter_map(|percent| {
            let exact = crate::amount::input::send_amount_shortcut(
                preview.max_sendable.clone(),
                asset_decimals,
                percent,
            )?;
            Some((percent, shortcut_amount(exact, percent, asset_decimals)))
        })
        .collect();
    let total = send_total(is_native, amount, Some(&network_fee));
    let amount_refusal = state
        .wallets
        .iter()
        .find(|w| w.id == wallet_id)
        .and_then(|wallet| {
            amount_refusal(&crate::send::send_affordability(
                crate::send::SendAffordabilityInput {
                    is_native,
                    chain_id: chain,
                    symbol: holding.symbol.clone(),
                    amount: amount.to_string(),
                    network_fee,
                    holding_balance: holding.amount.clone(),
                    gas_balance: wallet
                        .holdings
                        .iter()
                        .find(|h| h.is_native() && h.chain_id == chain)
                        .map(|h| h.amount.clone()),
                },
            ))
        });
    Ok(OwnedSendPreview {
        wallet_id,
        holding_key,
        chain_id: chain,
        amount: amount.to_string(),
        preview,
        total,
        network_fee_value,
        amount_value,
        shortcuts,
        recipient: None,
        amount_refusal,
    })
}

/// The amount and the fee together, when both are the sent asset: a coin
/// pays its own fee, a token pays the gas asset's.
pub(super) fn send_total(
    is_native: bool,
    amount: &str,
    network_fee: Option<&str>,
) -> Option<String> {
    if !is_native {
        return None;
    }
    crate::amount::decimal::add(amount, network_fee?)
}

/// Why an amount does not fit, as a person reads it. `Unavailable` is not a
/// refusal of the amount: the gas balance is unknown, which the build checks
/// for itself.
fn amount_refusal(
    verdict: &crate::send::SendAffordability,
) -> Option<crate::error::LocalizableMessage> {
    use crate::send::SendAffordability;
    match verdict {
        SendAffordability::Affordable | SendAffordability::Unavailable => None,
        SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } => {
            Some(crate::error::LocalizableMessage::new(
                "Insufficient %@ for the amount plus the network fee (requires %@ %@).",
                [symbol.as_str(), required.as_str(), symbol.as_str()],
            ))
        }
        SendAffordability::AmountExceedsBalance { symbol } => Some(
            crate::error::LocalizableMessage::new("Insufficient %@ balance.", [symbol]),
        ),
        SendAffordability::FeeExceedsGasBalance {
            gas_symbol,
            fee,
            chain_id,
        } => Some(crate::error::LocalizableMessage::new(
            "Insufficient %@ for the %@ network fee (%@ %@).",
            [
                gas_symbol.as_str(),
                chain_id.chain_display_name(),
                fee.as_str(),
                gas_symbol.as_str(),
            ],
        )),
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
        crate::service::worker::run(async move {
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
                // Whose address it is, and whether this wallet has sent
                // here, by the rule the build's review applies.
                let known = async {
                    let (chain, _) = this
                        .destination_probe_target(&wallet_id, &holding_key)
                        .await?;
                    let resolved = this
                        .resolve_send_destination(chain, destination.clone())
                        .await?;
                    if this.is_own_address(chain, &resolved.address).await? {
                        return Ok::<_, SpectraBridgeError>((true, false));
                    }
                    let warnings = this
                        .high_risk_send_reasons(
                            wallet_id.clone(),
                            holding_key.clone(),
                            "0".into(),
                            resolved.address,
                            destination.clone(),
                            resolved.used_ens,
                        )
                        .await;
                    let is_new =
                        warnings.contains(&crate::send::flow::HighRiskSendWarning::NewAddress);
                    Ok((false, is_new))
                };
                let (risk, known) = tokio::join!(risk, known);
                let (is_own_address, is_new_to_wallet) = known.unwrap_or((false, false));
                Some(RecipientCheck {
                    activity: risk.ok().map(|risk| risk.activity),
                    is_own_address,
                    is_new_to_wallet,
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
            let preview = self
                .preview_owned_evm_send(
                    wallet_id.clone(),
                    holding_key.clone(),
                    amount.clone(),
                    destination,
                    explicit_nonce,
                    custom_fees,
                )
                .await?;
            return wrap(preview).map(Some);
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
        if crate::amount::input::parse_raw_amount(&amount, decimals)? == 0 {
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
        // Tron reads its tokens in its own preview; every other network that
        // sends tokens quotes its coin's fee here and the token's balance.
        if let Some(token) = token.as_ref()
            && chain.mainnet_counterpart() != Chain::Tron
        {
            let real_decimals = self
                .token_decimals(chain, &token.contract, MetadataRead::Fresh)
                .await?;
            if real_decimals != token.decimals {
                return Err(SpectraBridgeError::invalid(
                    "Token precision differs from the stored deployment",
                ));
            }
            let reads = self
                .fetch_token_balances(
                    chain,
                    address.clone(),
                    vec![TokenDescriptor {
                        standard: chain.token_standard_for_identifier(&token.contract),
                        contract: token.contract.clone(),
                        symbol: holding.symbol.clone(),
                        decimals: token.decimals,
                        name: None,
                    }],
                )
                .await?;
            let balance = reads
                .into_iter()
                .next()
                .ok_or_else(|| SpectraBridgeError::failure("Token balance unavailable"))?;
            if balance.decimals != real_decimals {
                return Err(SpectraBridgeError::invalid(
                    "Token balance precision changed",
                ));
            }
            let mut preview = if chain.mainnet_counterpart() == Chain::Near {
                self.preview_near_send(
                    chain,
                    &address,
                    &destination,
                    crate::amount::input::parse_raw_amount(&amount, u32::from(real_decimals))?,
                    Some(&token.contract),
                )
                .await?
            } else {
                self.fetch_shared_send_preview(chain, address.clone())
                    .await?
            };
            // An XRP Ledger issuer's transfer rate takes its share on top of
            // what arrives, so the most that can arrive is the balance over
            // the rate.
            let max_sendable = if chain.mainnet_counterpart() == Chain::Xrp {
                let issue = crate::api::xrpl_amount::XrplIssue::parse(&token.contract)?;
                let rate = XrplClient::new(
                    self.endpoints_for(chain, &[EndpointCapability::Verification])
                        .await,
                )
                .fetch_issue_state(chain, &address, &issue, None)
                .await?
                .issuer
                .map_or(1_000_000_000, |issuer| issuer.transfer_rate);
                let places = u32::from(balance.decimals);
                let units = crate::amount::decimal::to_units(&balance.balance_display, places)
                    .ok_or_else(|| SpectraBridgeError::failure("Token balance unavailable"))?;
                crate::amount::decimal::from_units(units * 1_000_000_000 / u128::from(rate), places)
            } else {
                balance.balance_display.clone()
            };
            preview.spendable_balance = balance.balance_display.clone();
            preview.max_sendable = max_sendable;
            match chain.mainnet_counterpart() {
                Chain::Cardano => {
                    // The token's output carries the minimum ADA its size
                    // needs, which leaves the sender with it.
                    let params = KoiosClient::new(
                        self.endpoints_for(chain, &[EndpointCapability::Fee]).await,
                    )
                    .fetch_protocol_params()
                    .await?;
                    let carried =
                        crate::send::cardano::token_send_ada(&address, &token.contract, &params)?;
                    let carried = crate::amount::decimal::from_units(u128::from(carried), 6);
                    preview.estimated_network_fee =
                        crate::amount::decimal::add(&preview.estimated_network_fee, &carried)
                            .ok_or_else(|| {
                                SpectraBridgeError::failure("Invalid Cardano fee estimate")
                            })?;
                    preview.fee_rate_description =
                        Some("Network fee and the minimum ADA sent with the token".into());
                }
                Chain::Ton => {
                    preview.estimated_network_fee =
                        crate::amount::decimal::add(&preview.estimated_network_fee, "0.1")
                            .ok_or_else(|| {
                                SpectraBridgeError::failure("Invalid jetton gas budget")
                            })?;
                    preview.fee_rate_description =
                        Some("Network estimate and 0.1 TON attached to the jetton wallet".into());
                }
                _ => {}
            }
            return wrap(preview).map(Some);
        }
        let preview = match chain.mainnet_counterpart() {
            Chain::Near => {
                self.preview_near_send(
                    chain,
                    &address,
                    &destination,
                    crate::amount::input::parse_raw_amount(&amount, decimals)?,
                    None,
                )
                .await?
            }
            Chain::Litecoin => {
                match self
                    .preview_litecoin_owned_send(chain, &wallet_id, &amount, &destination)
                    .await?
                {
                    Some(preview) => preview,
                    None => return Ok(None),
                }
            }
            Chain::Peercoin => {
                self.preview_peercoin_owned_send(chain, &wallet_id, &amount, &destination)
                    .await?
            }
            _ if chain.uses_account_utxo() => {
                self.preview_account_send(chain, &wallet_id, &amount, &destination)
                    .await?
            }
            Chain::Tron => {
                self.fetch_tron_send_preview(
                    chain,
                    address,
                    holding.symbol.clone(),
                    token.map(|t| t.contract).unwrap_or_default(),
                )
                .await?
            }
            _ => self.fetch_shared_send_preview(chain, address).await?,
        };
        wrap(preview).map(Some)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
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
            .map(|preview| preview.estimated_network_fee.clone())
            .or_else(|| shape.fee_fallback.map(str::to_string))
            .or_else(|| {
                (shape.fee_field == crate::registry::SendFeeField::None).then(|| "0".into())
            })
            .ok_or_else(|| SpectraBridgeError::failure("Unable to estimate network fee"))?;
        let fee = crate::amount::decimal::canonical(&fee)
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
        if verdict == crate::send::SendAffordability::Unavailable {
            return Err(SpectraBridgeError::failure(
                "Unable to determine the available gas balance",
            ));
        }
        if let Some(message) = amount_refusal(&verdict) {
            return Err(SpectraBridgeError::Failure { message });
        }
        let fee_rate_svb = match preview.as_ref().map(|preview| &preview.terms) {
            Some(SendFeeTerms::Utxo {
                fee_rate_sat_vb, ..
            }) if matches!(
                chain.mainnet_counterpart(),
                Chain::Bitcoin | Chain::Litecoin
            ) =>
            {
                Some(fee_rate_sat_vb.to_string())
            }
            _ => None,
        };
        use crate::registry::SendFeeField;
        Ok(OwnedSendQuote {
            request: crate::send::SendExecutionRequest {
                token_standard: holding.token_standard,
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
                memo: None,
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
        crate::service::worker::run(async move {
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
                return Err(crate::error::SpectraBridgeError::failure(
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
                .await?;
            let SendFeeTerms::Evm { terms } = &preview.terms else {
                return Err(SpectraBridgeError::failure(
                    "Unable to estimate replacement fees",
                ));
            };
            let bump = crate::send::flow::evm_replacement_fee_bump(
                &terms.max_fee_per_gas_gwei,
                &terms.max_priority_fee_per_gas_gwei,
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
        let normalize = |address: &str| crate::send::flow::normalize_address(chain, address);
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
            if chain.uses_account_utxo() && wallet.chain_id == chain {
                owned.extend(self.known_utxo_addresses(wallet.id.clone(), chain).await?);
            }
        }
        Ok(owned)
    }
}

#[cfg(test)]
mod shortcut_amount_tests {
    use super::{send_total, shortcut_amount};

    #[test]
    fn shares_are_cut_to_display_precision_and_the_maximum_is_exact() {
        let exact = "0.012490233065563998".to_string();
        assert_eq!(shortcut_amount(exact.clone(), 25, 18), "0.0124902");
        assert_eq!(shortcut_amount(exact.clone(), 100, 18), exact);
        // Dust has no display form; it stays exact rather than become zero.
        let dust = "0.000000000000000001".to_string();
        assert_eq!(shortcut_amount(dust.clone(), 25, 18), dust);
    }

    /// A coin's total is what leaves the wallet, exactly; a token's fee is
    /// another asset's, so it has none, and nor does an unknown fee.
    #[test]
    fn a_total_adds_the_fee_only_where_the_coin_pays_it() {
        assert_eq!(
            send_total(true, "0.001", Some("0.000021")),
            Some("0.001021".to_string())
        );
        assert_eq!(
            send_total(true, "1", Some("0.000000000000000001")),
            Some("1.000000000000000001".to_string())
        );
        assert_eq!(send_total(false, "20", Some("0.000021")), None);
        assert_eq!(send_total(true, "0.001", None), None);
    }
}

#[cfg(test)]
#[path = "tests/send_owned.rs"]
mod owned_send_tests;
