//! Send previews and fee estimates. Signing and rebroadcast live in sibling modules.
use super::*;
use crate::send::preview_types::{SendFeeTerms, SendPreview};
impl WalletService {
    /// Preview a stored asset on the wallet's actual network without exposing its secret.
    pub async fn preview_owned_evm_send(
        &self,
        wallet_id: String,
        holding_key: String,
        amount: String,
        destination: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::send::ethereum::EvmCustomFeeConfiguration>,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let state = self.app_state().await;
        let wallet = state
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: "wallet does not exist".into(),
            })?;
        let holding = wallet
            .holdings
            .iter()
            .find(|h| h.deployment_id() == holding_key)
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: "holding does not exist".into(),
            })?;
        let (family, token) =
            super::send_destination::destination_probe_asset(holding, &state.token_preferences)?;
        if !family.is_evm() {
            return Err(SpectraBridgeError::failure(
                "EVM preview requires an EVM asset",
            ));
        }
        let chain = super::send_execution::send_chain_for(&state, &wallet_id, family)?;
        let from = wallet
            .address_on(chain)
            .ok_or_else(|| SpectraBridgeError::failure("wallet has no address on this network"))?
            .to_owned();
        let destination = if destination.trim().is_empty() {
            from.clone()
        } else {
            self.resolve_send_destination(chain, destination)
                .await?
                .address
        };
        // The stored holding says what is sent: its token's contract on this
        // network, or the network's own coin.
        let deployment_id = holding.deployment_id();
        let assembly = crate::send::ethereum::prepare_evm_send_assembly(
            crate::send::ethereum::EvmSendAssemblyInput {
                chain_id: chain,
                deployment_id,
                from_address: from.clone(),
                resolved_destination: destination,
                amount,
                token: token.map(|t| crate::send::ethereum::EvmSupportedToken {
                    symbol: t.symbol,
                    contract_address: t.contract,
                    decimals: t.decimals.into(),
                }),
            },
        )
        .map_err(|e| SpectraBridgeError::InvalidInput {
            message: e.to_string().into(),
        })?;
        let nonce = match explicit_nonce {
            Some(nonce) => nonce,
            None => i64::try_from(self.next_send_nonce(chain, &from).await?)
                .map_err(|_| SpectraBridgeError::failure("Nonce exceeds supported range"))?,
        };
        self.fetch_evm_send_preview(
            chain,
            from,
            assembly.to_address,
            assembly.value_wei,
            assembly.data_hex,
            Some(nonce),
            custom_fees,
        )
        .await
    }
}

impl WalletService {
    /// Bitcoin's fee rate, in sat/vB.
    ///
    /// `chain` is the Bitcoin network to quote for: a testnet's fees are its
    /// own.
    pub(crate) async fn bitcoin_fee_rate(
        &self,
        chain: Chain,
    ) -> Result<crate::api::utxo::FeeRate, SpectraBridgeError> {
        Ok(self
            .utxo_client(chain, &[EndpointCapability::Fee])
            .await
            .fetch_fee_rate(6)
            .await?)
    }

    /// A chain's fee quoted in its own native unit, live where the chain has
    /// an RPC that answers and static where the catalog carries the number.
    ///
    /// Testnets use the same protocol with their own endpoints and precision.
    /// Bitcoin and EVM have separate preview paths.
    pub(crate) async fn native_fee_estimate(
        &self,
        chain: Chain,
    ) -> Result<NativeFeeEstimate, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain, &[EndpointCapability::Fee]).await;
        let native = |units: u128, source: &'static str| NativeFeeEstimate {
            units,
            source,
            gas_unit_price_octas: None,
        };
        match chain.mainnet_counterpart() {
            // Chains with live RPC fee fetches.
            Chain::Near => {
                let client = NearClient::new(endpoints);
                client.verify_network(chain).await?;
                Ok(native(
                    client.transfer_fee_budget("", &"0".repeat(64)).await?,
                    "rpc",
                ))
            }
            Chain::Xrp => {
                let drops = XrplClient::new(endpoints).fetch_fee().await?;
                Ok(native(drops as u128, "rpc"))
            }
            Chain::Stellar => {
                let stroops = HorizonClient::new(endpoints).fetch_base_fee().await?;
                Ok(native(stroops as u128, "rpc"))
            }
            Chain::Aptos => {
                let price = AptosClient::new(endpoints).fetch_gas_price().await?;
                let max_gas = chain.aptos_max_gas_amount().expect("Aptos gas limit");
                let budget = price
                    .checked_mul(max_gas)
                    .filter(|budget| *budget > 0)
                    .ok_or_else(|| SpectraBridgeError::failure("Invalid fee"))?;
                let mut fee = native(u128::from(budget), "rpc");
                fee.gas_unit_price_octas = Some(price);
                Ok(fee)
            }
            // Every remaining supported chain gets its estimate from the registry.
            _ => match chain.static_fee_units() {
                Some(units) => Ok(native(units, "static")),
                None => Err(SpectraBridgeError::failure(format!(
                    "fee estimation not supported for {}",
                    chain.chain_display_name()
                ))),
            },
        }
    }

    /// Quote an EVM send: nonce, fee, gas limit, and what is spendable.
    ///
    /// "Spendable" is a fact about the asset the amount field moves. Gas is
    /// always paid in the chain's coin, but the amount is not always
    /// denominated in it:
    ///
    /// - a native transfer spends one asset for both, so its spendable is
    ///   `balance - fee`;
    /// - an ERC-20 transfer spends two, so the whole token balance is
    ///   spendable and the fee is a separate claim on the gas coin.
    ///
    /// Which case this is comes off the calldata, not off a caller-supplied
    /// descriptor: an ERC-20 transfer *is* `transfer(address,uint256)`
    /// addressed to the token contract, so the selector names the case and
    /// the contract's own `decimals()` scales the answer.
    ///
    /// `explicit_nonce` and `custom_fees` replace what the network would
    /// answer for them.
    pub async fn fetch_evm_send_preview(
        &self,
        chain_id: crate::registry::Chain,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::send::ethereum::EvmCustomFeeConfiguration>,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let overrides = crate::send::ethereum::EvmSendOverridesInput {
            nonce: explicit_nonce,
            custom_fees: custom_fees.clone(),
            ..Default::default()
        }
        .resolve(chain_id)?;
        let chain = evm_network(chain_id)?;
        let eps = self.endpoints_for(chain, &[EndpointCapability::Fee]).await;
        let client = EvmClient::new(eps, chain.evm_chain_id()?);

        if value_wei.is_empty() || !value_wei.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SpectraBridgeError::failure(
                "value_wei must be an unsigned integer",
            ));
        }
        let value_u128: u128 = value_wei
            .parse()
            .map_err(|_| SpectraBridgeError::failure("value_wei exceeds u128 range"))?;
        let data_opt: Option<&str> = if data_hex == "0x" || data_hex.is_empty() {
            None
        } else {
            Some(&data_hex)
        };
        // An ERC-20 transfer is addressed to the token contract, so the
        // destination *is* the token whose balance this send spends.
        let token_contract = data_opt
            .filter(|data| crate::api::evm_json_rpc::is_erc20_transfer(data))
            .map(|_| to.as_str());

        let verification = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Verification])
                .await,
            chain.evm_chain_id()?,
        );
        let balances = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::Balance])
                .await,
            chain.evm_chain_id()?,
        );
        let tokens = EvmClient::new(
            self.endpoints_for(chain, &[EndpointCapability::TokenBalance])
                .await,
            chain.evm_chain_id()?,
        );
        let (nonce_res, fee_res, bal_res, token_res) = tokio::join!(
            verification.fetch_nonce(&from),
            client.fetch_fee_estimate(),
            balances.fetch_balance(&from),
            async {
                match token_contract {
                    Some(contract) => Some(tokens.fetch_erc20_balance(contract, &from).await),
                    None => None,
                }
            }
        );

        let nonce = overrides.nonce.unwrap_or(nonce_res?);
        let fee = fee_res?;
        let mut overrides = overrides;
        overrides.nonce = Some(nonce);
        overrides.max_fee_per_gas_wei = Some(
            overrides
                .max_fee_per_gas_wei
                .unwrap_or(fee.max_fee_per_gas_wei),
        );
        overrides.max_priority_fee_per_gas_wei = Some(
            overrides
                .max_priority_fee_per_gas_wei
                .unwrap_or(fee.priority_fee_wei),
        );
        let prepared = crate::send::evm::prepare_transfer(
            &client,
            &from,
            &to,
            value_u128,
            &crate::api::evm_json_rpc::decode_hex(&data_hex)?,
            &overrides,
        )
        .await?;
        let native_balance_wei: u128 = bal_res?
            .balance_wei
            .parse()
            .map_err(|_| SpectraBridgeError::failure("invalid EVM balance"))?;
        // A token read that failed is not a zero holding: everything the send
        // sheet decides from this — whether the amount fits, whether it is the
        // whole balance — would be decided against a number nobody read.
        let token_balance = token_res
            .map(|token| {
                token
                    .map(|(raw, decimals)| crate::decimal::from_units(raw, u32::from(decimals)))
                    .map_err(SpectraBridgeError::from)
            })
            .transpose()?;
        Ok(crate::send::ethereum::evm_send_preview(
            &prepared,
            native_balance_wei,
            token_balance,
            custom_fees.is_some(),
        )?)
    }

    pub(crate) async fn fetch_tron_send_preview(
        &self,
        chain: Chain,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<SendPreview, SpectraBridgeError> {
        let trx = |sun: u128| crate::decimal::from_units(sun, u32::from(chain.native_decimals()));
        if !contract_address.is_empty() {
            let standard = chain.token_standard_for_identifier(&contract_address);
            crate::tokens::validate_protocol_identifier(chain, standard, &contract_address)?;
            if !chain.sends_token_standard(standard) {
                return Err(SpectraBridgeError::invalid(format!(
                    "{standard} transfers are not supported"
                )));
            }
        }
        let eps = self
            .endpoints_for(
                chain,
                if contract_address.is_empty() {
                    &[EndpointCapability::Balance]
                } else {
                    &[EndpointCapability::TokenBalance]
                },
            )
            .await;
        let client = TronHttpClient::new(eps);

        if !contract_address.is_empty()
            && chain.token_standard_for_identifier(&contract_address) == "TRC-10"
        {
            // The preview has no recipient. Reserve both activation fees and
            // all paid bandwidth; actual execution may use free or issuer bandwidth.
            let state = client
                .fetch_trc10_transfer_state(chain, &contract_address, &address, None, 512)
                .await?;
            if state.native_balance < state.fee_budget_sun {
                return Err(SpectraBridgeError::failure(
                    "Insufficient TRX for the TRC-10 bandwidth and account activation budget",
                ));
            }
            return Ok(SendPreview {
                estimated_network_fee: trx(u128::from(state.fee_budget_sun)),
                max_sendable: state.token.balance_display.clone(),
                spendable_balance: state.token.balance_display,
                fee_rate_description: Some(
                    "TRC-10 paid bandwidth and account activation upper bound".into(),
                ),
                ..Default::default()
            });
        }

        // The native asset, by the catalog's gas token rather than the string
        // "TRX" — the same fact the rest of the send path routes on.
        if contract_address.is_empty() {
            if symbol != Chain::Tron.coin_symbol() {
                return Err(SpectraBridgeError::failure("token identifier required"));
            }
            // TRX is the fee asset as well as the amount, so the fee comes out
            // of what is spendable. Only this branch needs the TRX balance.
            let balance_sun = client
                .fetch_balance(&address)
                .await
                .map(|b| b.sun)
                .map_err(SpectraBridgeError::from)?;
            let fee_sun = fee_or_static(Chain::Tron, None)?;
            let spendable = trx(u128::from(balance_sun.saturating_sub(fee_sun)));
            return Ok(SendPreview {
                estimated_network_fee: trx(u128::from(fee_sun)),
                max_sendable: spendable.clone(),
                spendable_balance: spendable,
                fee_rate_description: Some("Static bandwidth estimate".into()),
                ..Default::default()
            });
        }

        // A TRC-20's decimals are the contract's. `fetch_trc20_balance` reads
        // `decimals()` alongside the balance; the fixed `1e6` that stood here
        // is TRX's own scale, and reported an 18-decimal token as 10^12 times
        // the holding it actually is. Energy is paid in TRX, so the whole
        // token balance is spendable.
        let token = client
            .fetch_trc20_balance(&contract_address, &address)
            .await
            .map_err(SpectraBridgeError::from)?;
        let raw: u128 = token
            .balance_raw
            .parse()
            .map_err(|_| SpectraBridgeError::failure("invalid TRC-20 balance"))?;
        let token_balance = crate::decimal::from_units(raw, u32::from(token.decimals));

        // The estimate is what a transfer usually burns; the signed
        // transaction's fee limit is the most it lets it burn.
        Ok(SendPreview {
            estimated_network_fee: trx(u128::from(crate::send::tron::TRC20_TYPICAL_FEE_SUN)),
            max_sendable: token_balance.clone(),
            spendable_balance: token_balance,
            fee_rate_description: Some("Static energy estimate".into()),
            ..Default::default()
        })
    }

    /// The send preview of a chain on the shared path
    /// ([`Chain::has_shared_send_preview`]): its fee, its coin's balance, and
    /// the balance less the fee as the maximum.
    ///
    /// A fee or a balance that cannot be read is an error, never a zero: a
    /// zero standing in for either is a wrong maximum offered to the user.
    pub(crate) async fn fetch_shared_send_preview(
        &self,
        chain: Chain,
        address: String,
    ) -> Result<SendPreview, SpectraBridgeError> {
        if !chain.has_shared_send_preview() {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("{chain} has no shared-path send preview").into(),
            });
        }
        let decimals = u32::from(chain.native_decimals());
        let units = |amount: u128| crate::decimal::from_units(amount, decimals);
        match chain.mainnet_counterpart() {
            Chain::Near => return self.preview_near_send(chain, &address, "", 0, None).await,
            Chain::Polkadot | Chain::Bittensor => {
                let endpoints = self
                    .endpoints_for(
                        chain,
                        &[
                            EndpointCapability::Fee,
                            EndpointCapability::Balance,
                            EndpointCapability::Verification,
                        ],
                    )
                    .await;
                let (fee, balance, bytes) = crate::api::http::race(&endpoints, |endpoint| {
                    let address = &address;
                    async move {
                        crate::send::polkadot::preview_transfer(
                            &SubstrateClient::new(Arc::new(vec![endpoint])),
                            chain,
                            address,
                        )
                        .await
                    }
                })
                .await?;
                return Ok(SendPreview {
                    estimated_network_fee: units(fee),
                    spendable_balance: units(balance),
                    max_sendable: units(balance.saturating_sub(fee)),
                    fee_rate_description: Some(
                        "Substrate RPC quote; existential deposit retained".into(),
                    ),
                    estimated_transaction_bytes: i64::try_from(bytes).ok(),
                    terms: SendFeeTerms::Flat,
                });
            }
            Chain::Cardano => {
                let utxos = self.cardano_inputs(chain, &address).await?;
                let params =
                    KoiosClient::new(self.endpoints_for(chain, &[EndpointCapability::Fee]).await)
                        .fetch_protocol_params()
                        .await?;
                let (fee, max) = crate::send::cardano::ada_preview(&utxos, &params, &address)?;
                let total: u128 = utxos.iter().map(|utxo| u128::from(utxo.lovelace)).sum();
                return Ok(SendPreview {
                    estimated_network_fee: units(u128::from(fee)),
                    spendable_balance: units(total),
                    max_sendable: units(u128::from(max)),
                    fee_rate_description: Some(
                        "Protocol fee for this transaction's size; ADA held with tokens stays with them as change".into(),
                    ),
                    ..Default::default()
                });
            }
            _ => {}
        }
        let (fee, balance) = tokio::try_join!(
            self.native_fee_estimate(chain),
            self.fetch_native_balance_summary(chain, address),
        )?;
        let fee_display = units(fee.units);
        let balance_display =
            crate::decimal::canonical(&balance.amount_display).ok_or_else(|| {
                SpectraBridgeError::failure(format!(
                    "{chain} balance: not a number: {:?}",
                    balance.amount_display
                ))
            })?;
        let max_sendable = crate::decimal::sub_or_zero(&balance_display, &fee_display)
            .ok_or_else(|| SpectraBridgeError::failure(format!("{chain} balance unreadable")))?;
        let fee_rate_description = match fee.gas_unit_price_octas {
            Some(price) => format!("{price} octas/unit"),
            None => fee.source.to_string(),
        };
        Ok(SendPreview {
            estimated_network_fee: fee_display,
            spendable_balance: balance_display,
            max_sendable,
            fee_rate_description: Some(fee_rate_description),
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[path = "tests/send_preview.rs"]
mod tests;

impl WalletService {
    pub(super) async fn preview_litecoin_owned_send(
        &self,
        chain: Chain,
        wallet_id: &str,
        amount: &str,
        destination: &str,
    ) -> Result<Option<SendPreview>, SpectraBridgeError> {
        self.litecoin_preview_at_rate(chain, wallet_id, amount, destination, 0)
            .await
    }

    async fn litecoin_preview_at_rate(
        &self,
        chain: Chain,
        wallet_id: &str,
        amount: &str,
        destination: &str,
        fee_rate: u64,
    ) -> Result<Option<SendPreview>, SpectraBridgeError> {
        let amount = u64::try_from(crate::send::amount_input::parse_raw_amount(amount, 8)?)
            .map_err(|_| SpectraBridgeError::invalid("Amount exceeds protocol range"))?;
        if amount > chain.litecoin_max_money()? {
            return Err(SpectraBridgeError::invalid(
                "Amount exceeds Litecoin MAX_MONEY",
            ));
        }
        if !destination.trim().is_empty() {
            let recipient = crate::send::litecoin_mweb::prepared::CanonicalRecipient::parse(
                chain,
                destination,
            )?;
            if amount > 0 && amount < recipient.minimum_amount(chain)? {
                return Err(SpectraBridgeError::invalid(
                    "Litecoin recipient amount is below the dust threshold",
                ));
            }
        }
        let sources = self.account_utxo_send_sources(wallet_id, chain).await?;
        let sender = self
            .wallet_state
            .read()
            .await
            .wallets
            .iter()
            .find(|wallet| wallet.id == wallet_id)
            .and_then(|wallet| wallet.address_on(chain))
            .map(str::to_string)
            .ok_or_else(|| SpectraBridgeError::invalid("wallet has no Litecoin address"))?;
        let change = sources
            .iter()
            .find(|source| source.address == sender)
            .ok_or_else(|| SpectraBridgeError::invalid("Litecoin change address is not owned"))?;
        let inputs = self.collect_litecoin_inputs(chain, wallet_id).await?;
        let rate = if fee_rate > 0 {
            fee_rate
        } else {
            self.utxo_client(chain, &[EndpointCapability::Fee])
                .await
                .fetch_fee_rate(3)
                .await
                .map(|rate| rate.sats_per_vbyte.ceil().max(1.0) as u64)
                .unwrap_or(1)
        };
        Ok(crate::send::litecoin_quote::quote_inputs(
            chain,
            &inputs,
            &change.script_pubkey,
            destination,
            amount,
            rate,
        )?)
    }
}
