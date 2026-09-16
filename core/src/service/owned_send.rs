//! Send decisions over owned wallet identity. UI inputs contain only user edits.
use super::*;
use crate::send::flow::SendPreview;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn preview_owned_send(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: String,
        destination: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::ethereum_send::EvmCustomFeeConfiguration>,
    ) -> Result<Option<SendPreview>, SpectraBridgeError> {
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("wallet does not exist")?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_key() == holding_key)
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
        if chain.is_evm() {
            return Ok(self
                .preview_owned_evm_send(
                    wallet_id,
                    holding_key,
                    amount,
                    destination,
                    explicit_nonce,
                    custom_fees,
                )
                .await?
                .map(|preview| SendPreview::Ethereum { preview }));
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
            Chain::Bitcoin => if let Some(xpub) =
                wallet.xpub.as_ref().filter(|x| !x.trim().is_empty())
            {
                self.fetch_bitcoin_hd_send_preview_typed(
                    chain.str_id().into(),
                    xpub.clone(),
                    20,
                    20,
                )
                .await?
            } else {
                self.fetch_utxo_fee_preview_typed(chain.str_id().into(), address, 0, destination)
                    .await?
            }
            .map(|preview| SendPreview::Utxo { preview }),
            Chain::BitcoinCash | Chain::BitcoinSV | Chain::Litecoin => self
                .fetch_utxo_fee_preview_typed(chain.str_id().into(), address, 0, destination)
                .await?
                .map(|preview| SendPreview::Utxo { preview }),
            Chain::Dogecoin => {
                let priority = state
                    .settings
                    .fee_priority_by_chain
                    .get(chain.chain_display_name())
                    .copied()
                    .unwrap_or(crate::store::state::FeePriority::Normal);
                // This legacy provider preview accepts a display amount; signing parses the exact input separately.
                self.fetch_dogecoin_send_preview_typed(
                    address,
                    amount.parse().map_err(|_| "invalid amount")?,
                    priority.as_raw().to_string(),
                )
                .await?
                .map(|preview| SendPreview::Dogecoin { preview })
            }
            Chain::Tron => self
                .fetch_tron_send_preview_typed(
                    address,
                    holding.symbol.clone(),
                    token.map(|t| t.contract).unwrap_or_default(),
                )
                .await?
                .map(|preview| SendPreview::Tron { preview }),
            _ => Some(
                self.fetch_simple_chain_send_preview_typed(chain.str_id().into(), address)
                    .await?
                    .into(),
            ),
        };
        Ok(preview)
    }

    pub async fn self_send_confirmation(
        &self,
        wallet_id: String,
        holding_key: String,
        destination: String,
        amount: f64,
        pending: Option<crate::store::PendingSelfSendConfirmationInput>,
    ) -> Result<crate::store::SelfSendConfirmationPlan, SpectraBridgeError> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("amount must be finite and non-negative".into());
        }
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or("wallet does not exist")?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_key() == holding_key)
            .ok_or("holding does not exist")?;
        let chain = holding.network().ok_or("invalid asset network")?;
        super::send_execution::send_chain_for(&state, &wallet_id, chain)?;
        let destination = self
            .resolve_send_destination(chain.str_id().into(), destination)
            .await?
            .address;
        let mut owned = Vec::new();
        for wallet in &state.wallets {
            if let Some(address) = wallet.address_on(chain) {
                owned.push(address.to_string());
            }
            owned.extend(
                self.owned_addresses_for_wallet(
                    wallet.id.clone(),
                    Some(chain.chain_display_name().into()),
                )
                .await,
            );
            if chain.supports_deep_utxo_discovery()
                && wallet.network_chain(&state.settings) == Some(chain)
            {
                owned.extend(
                    self.known_utxo_addresses(wallet.id.clone(), chain.str_id().into())
                        .await?,
                );
            }
        }
        Ok(crate::store::core_self_send_confirmation(
            crate::store::SelfSendConfirmationRequest {
                pending_confirmation: pending,
                wallet_id,
                chain_name: chain.chain_display_name().into(),
                symbol: holding.symbol.clone(),
                destination_address: destination,
                amount,
                now_unix: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| "invalid clock")?
                    .as_secs_f64(),
                window_seconds: 20.0,
                owned_addresses: owned,
            },
        ))
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
        overrides: Option<crate::ethereum_send::EvmSendOverridesInput>,
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
            .find(|h| h.deployment_key() == holding_key)
            .ok_or("holding does not exist")?;
        let chain = holding.network().ok_or("invalid network")?;
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
            .await?;
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
            .find(|h| h.deployment_key() == holding_key)
            .ok_or("holding was removed")?;
        let verdict = crate::send::send_affordability(crate::send::SendAffordabilityInput {
            is_native: holding.is_native(),
            chain_name: chain.chain_display_name().into(),
            symbol: holding.symbol.clone(),
            amount: preflight.amount,
            network_fee: fee,
            holding_balance: holding.amount,
            gas_balance: wallet
                .holdings
                .iter()
                .find(|h| h.is_native() && h.network() == Some(chain))
                .map(|h| h.amount),
        });
        use crate::send::SendAffordability;
        match verdict {
            SendAffordability::Affordable => {}
            SendAffordability::Unavailable => return Err("Unable to determine the available gas balance".into()),
            SendAffordability::AmountPlusFeeExceedsBalance { symbol, required } =>
                return Err(format!("Insufficient {symbol} for amount plus network fee (requires {required} {symbol})").into()),
            SendAffordability::AmountExceedsBalance { symbol } =>
                return Err(format!("Insufficient {symbol} balance").into()),
            SendAffordability::FeeExceedsGasBalance { gas_symbol, fee, chain_name } =>
                return Err(format!("Insufficient {gas_symbol} for the {chain_name} network fee ({fee} {gas_symbol})").into()),
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
            .find(|h| h.is_native() && h.network() == Some(chain))
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
            self.fetch_evm_tx_nonce_typed(pending.chain_id, pending.transaction_hash)
                .await?,
        )
        .map_err(|_| "nonce exceeds supported range")?;
        let preview = self
            .preview_owned_evm_send(
                wallet.id.clone(),
                holding.deployment_key(),
                amount.clone(),
                destination.clone(),
                Some(nonce),
                None,
            )
            .await?
            .ok_or("Unable to estimate replacement fees")?;
        let bump = crate::send::flow::core_evm_replacement_fee_bump(
            Some(preview.maxFeePerGasGwei.to_string()),
            Some(preview.maxPriorityFeePerGasGwei.to_string()),
            preview.maxFeePerGasGwei,
            preview.maxPriorityFeePerGasGwei,
        );
        Ok(OwnedReplacementDraft {
            wallet_id: wallet.id.clone(),
            holding_key: holding.deployment_key(),
            destination,
            amount,
            nonce,
            max_fee_gwei: bump.max_fee_gwei,
            priority_fee_gwei: bump.priority_fee_gwei,
        })
    }
}
