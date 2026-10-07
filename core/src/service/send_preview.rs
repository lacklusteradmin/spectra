//! Send previews and fee estimates. Signing and rebroadcast live in sibling modules.
use super::*;
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
    ) -> Result<Option<crate::send::preview_types::EvmSendPreview>, SpectraBridgeError> {
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
        let native = |raw: u128, source: &'static str| NativeFeeEstimate {
            raw: raw.to_string(),
            display: crate::decimal::from_units(raw, u32::from(chain.native_decimals())),
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

    pub(crate) async fn fetch_utxo_fee_preview_json(
        &self,
        chain: crate::registry::Chain,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<String, SpectraBridgeError> {
        let family = chain.mainnet_counterpart();
        if !matches!(
            family,
            Chain::Bitcoin | Chain::Dogecoin | Chain::BitcoinCash | Chain::BitcoinSV
        ) {
            return Err(SpectraBridgeError::failure(format!(
                "fetch_utxo_fee_preview_json: unsupported chain: {family:?}"
            )));
        }
        let utxos = self
            .utxo_client(chain, &[EndpointCapability::Utxo])
            .await
            .fetch_utxos(&address)
            .await?;
        let rate = if fee_rate_svb > 0 {
            fee_rate_svb
        } else {
            let fees = self.utxo_client(chain, &[EndpointCapability::Fee]).await;
            match family {
                // Bitcoin is quoted live, or not previewed.
                Chain::Bitcoin => fees
                    .fetch_fee_rate(3)
                    .await
                    .map(|r| r.sats_per_vbyte.ceil() as u64)?,
                // A live quote where one answers, else the relay floor.
                Chain::BitcoinCash => fees
                    .fetch_fee_rate(3)
                    .await
                    .map(|r| (r.sats_per_vbyte.ceil() as u64).max(1))
                    .unwrap_or(1),
                _ => 1,
            }
        };
        let values: Vec<u64> = utxos.into_iter().map(|u| u.value).collect();
        Ok(utxo_fee_preview_json(values, rate))
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
    pub(crate) async fn fetch_evm_send_preview_json(
        &self,
        chain_id: crate::registry::Chain,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
        overrides: crate::send::evm::EvmSendOverrides,
    ) -> Result<String, SpectraBridgeError> {
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
        let gas_limit = prepared.gas_limit;
        let balance_wei_val: u128 = bal_res?
            .balance_wei
            .parse()
            .map_err(|_| SpectraBridgeError::failure("invalid EVM balance"))?;

        let estimated_fee_wei = prepared.maximum_fee_wei()?;
        let gwei = |wei: u128| crate::decimal::from_units(wei, 9);

        // A token read that failed is not a zero holding: everything the send
        // sheet decides from this — whether the amount fits, whether it is the
        // whole balance — would be decided against a number nobody read.
        let spendable_balance = match token_res {
            Some(token) => {
                let token = token.map_err(SpectraBridgeError::from)?;
                let raw: u128 = token
                    .balance_raw
                    .parse()
                    .map_err(|_| SpectraBridgeError::failure("invalid ERC-20 balance"))?;
                crate::decimal::from_units(raw, u32::from(token.decimals))
            }
            None => {
                crate::decimal::from_units(balance_wei_val.saturating_sub(estimated_fee_wei), 18)
            }
        };

        Ok(json!({
            "nonce": nonce,
            "gas_limit": gas_limit,
            "max_fee_per_gas_wei": prepared.max_fee_per_gas.to_string(),
            "max_priority_fee_per_gas_wei": prepared.max_priority_fee_per_gas.to_string(),
            "estimated_fee_wei": estimated_fee_wei.to_string(),
            "additional_fee_wei": prepared.additional_fee_wei.to_string(),
            "spendable_balance": spendable_balance,
            "is_token": crate::api::evm_json_rpc::is_erc20_transfer(&data_hex),
            "native_balance_wei": balance_wei_val.to_string(),
            "fee_rate_description": format!("Max {} gwei / Priority {} gwei",
                gwei(prepared.max_fee_per_gas), gwei(prepared.max_priority_fee_per_gas)),
        })
        .to_string())
    }

    pub(crate) async fn fetch_tron_send_preview_json_on_chain(
        &self,
        chain: Chain,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<String, SpectraBridgeError> {
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
            return Ok(json!({
                "estimated_fee_trx": crate::decimal::from_units(u128::from(state.fee_budget_sun), u32::from(chain.native_decimals())),
                "fee_limit_sun": 0_i64,
                "spendable_balance": state.token.balance_display,
                "max_sendable": state.token.balance_display,
                "fee_rate_description": "TRC-10 paid bandwidth and account activation upper bound",
            }).to_string());
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
            let decimals = u32::from(Chain::Tron.native_decimals());
            let spendable = crate::decimal::from_units(
                u128::from(balance_sun.saturating_sub(fee_sun)),
                decimals,
            );
            return Ok(json!({
                "estimated_fee_trx": crate::decimal::from_units(u128::from(fee_sun), decimals),
                "fee_limit_sun": 0_i64,
                "spendable_balance": spendable,
                "max_sendable": spendable,
                "fee_rate_description": "Static bandwidth estimate",
            })
            .to_string());
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

        // The estimate is what a transfer usually burns; the limit is the
        // most the signed transaction lets it burn.
        Ok(json!({
            "estimated_fee_trx": crate::decimal::from_units(
                u128::from(crate::send::tron::TRC20_TYPICAL_FEE_SUN),
                u32::from(Chain::Tron.native_decimals()),
            ),
            "fee_limit_sun": crate::send::tron::TRC20_FEE_LIMIT_SUN,
            "spendable_balance": token_balance,
            "max_sendable": token_balance,
            "fee_rate_description": "Static energy estimate",
        })
        .to_string())
    }

    pub(crate) async fn fetch_simple_chain_send_preview_json(
        &self,
        chain: crate::registry::Chain,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        if chain.mainnet_counterpart() == Chain::Near {
            let preview = self.preview_near_send(chain, &address, "", 0, None).await?;
            return Ok(json!({
                "fee_display": preview.estimatedNetworkFee,
                "fee_raw": preview.feeBudgetYoctoNear,
                "fee_rate_description": preview.feeRateDescription,
                "balance_display": preview.spendableBalance,
                "max_sendable": preview.maxSendable,
            })
            .to_string());
        }
        if matches!(
            chain.mainnet_counterpart(),
            Chain::Polkadot | Chain::Bittensor
        ) {
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
            let decimals = u32::from(chain.native_decimals());
            return Ok(json!({
                "fee_display": crate::decimal::from_units(fee, decimals),
                "fee_raw": fee.to_string(),
                "fee_rate_description": "Substrate RPC quote; existential deposit retained",
                "balance_display": crate::decimal::from_units(balance, decimals),
                "max_sendable": crate::decimal::from_units(balance.saturating_sub(fee), decimals),
                "estimated_transaction_bytes": bytes,
            })
            .to_string());
        }
        let (fee, balance) = tokio::try_join!(
            self.native_fee_estimate(chain),
            self.fetch_native_balance_summary(chain, address),
        )?;

        // `max_sendable` is balance minus fee, so a zero standing in for either
        // one is a wrong maximum offered to the user: an unread fee makes the
        // whole balance look sendable, an unread balance makes none of it.
        let unreadable = |what: &str, value: &str| {
            SpectraBridgeError::failure(format!("{chain} {what}: not a number: {value:?}"))
        };
        let fee_display = crate::decimal::canonical(&fee.display)
            .ok_or_else(|| unreadable("fee", &fee.display))?;
        let fee_raw = fee.raw;
        let fee_rate_description = fee.source.to_string();

        let balance_display = crate::decimal::canonical(&balance.amount_display)
            .ok_or_else(|| unreadable("balance", &balance.amount_display))?;
        let max_sendable = crate::decimal::sub_or_zero(&balance_display, &fee_display)
            .ok_or_else(|| unreadable("balance", &balance_display))?;

        let mut value = json!({
            "fee_display":          fee_display,
            "fee_raw":              fee_raw,
            "fee_rate_description": fee_rate_description,
            "balance_display":      balance_display,
            "max_sendable":         max_sendable,
        });
        if let Some(price) = fee.gas_unit_price_octas {
            value["gas_unit_price_octas"] = json!(price);
            value["max_gas_amount"] = json!(chain.aptos_max_gas_amount().expect("Aptos gas limit"));
        }
        Ok(value.to_string())
    }
}

#[cfg(test)]
#[path = "tests/send_preview.rs"]
mod tests;

impl WalletService {
    /// Typed EVM send preview: fetches the raw preview JSON then decodes it
    /// into `EvmSendPreview` with the caller-supplied nonce / fee
    /// overrides applied. Returns `None` when the decoder rejects the payload.
    pub async fn fetch_evm_send_preview(
        &self,
        chain_id: crate::registry::Chain,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::send::ethereum::EvmCustomFeeConfiguration>,
    ) -> Result<Option<crate::send::preview_types::EvmSendPreview>, SpectraBridgeError> {
        let overrides = crate::send::ethereum::EvmSendOverridesInput {
            nonce: explicit_nonce,
            custom_fees: custom_fees.clone(),
            ..Default::default()
        }
        .resolve(chain_id)?;
        let raw = self
            .fetch_evm_send_preview_json(chain_id, from, to, value_wei, data_hex, overrides)
            .await?;
        Ok(crate::send::preview_decode::build_evm_send_preview_record(
            crate::send::ethereum::EvmPreviewDecodeInput {
                raw_json: raw,
                explicit_nonce,
                custom_fees,
            },
        ))
    }
}

impl WalletService {
    pub async fn fetch_bitcoin_hd_send_preview(
        &self,
        chain: crate::registry::Chain,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<Option<crate::send::preview_types::BitcoinSendPreview>, SpectraBridgeError> {
        if chain.mainnet_counterpart() != Chain::Bitcoin {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("{chain} is not a Bitcoin network").into(),
            });
        }
        let (balance, rate) = tokio::try_join!(
            self.bitcoin_xpub_balance(chain, xpub, receive_count, change_count),
            self.bitcoin_fee_rate(chain),
        )?;
        Ok(
            crate::send::preview_decode::build_bitcoin_hd_send_preview_record(
                balance.confirmed_sats,
                rate.sats_per_vbyte,
            ),
        )
    }
    pub async fn fetch_dogecoin_send_preview(
        &self,
        address: String,
        requested_amount: String,
    ) -> Result<Option<crate::send::preview_types::DogecoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview_json(Chain::Dogecoin, address, 0)
            .await?;
        Ok(crate::send::preview_decode::build_dogecoin_send_preview_record(raw, &requested_amount))
    }
    pub async fn fetch_simple_chain_send_preview(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
    ) -> Result<Option<crate::send::preview_decode::SimpleChainPreview>, SpectraBridgeError> {
        let chain =
            chain_id
                .simple_preview_chain()
                .ok_or_else(|| SpectraBridgeError::InvalidInput {
                    message: format!("{chain_id} has no shared-path send preview").into(),
                })?;
        let raw = self
            .fetch_simple_chain_send_preview_json(chain_id, address)
            .await?;
        Ok(crate::send::preview_decode::build_simple_chain_preview(
            raw, chain,
        ))
    }
    pub async fn fetch_utxo_fee_preview(
        &self,
        chain_id: crate::registry::Chain,
        address: String,
        fee_rate_svb: u64,
        destination_address: String,
    ) -> Result<Option<crate::send::preview_types::BitcoinSendPreview>, SpectraBridgeError> {
        if chain_id.mainnet_counterpart() == Chain::Litecoin {
            let wallet_id = self
                .wallet_state
                .read()
                .await
                .wallets
                .iter()
                .find(|wallet| wallet.address_on(chain_id) == Some(address.as_str()))
                .map(|wallet| wallet.id.clone())
                .ok_or_else(|| {
                    SpectraBridgeError::invalid("Litecoin fee preview requires an owned wallet")
                })?;
            return self
                .litecoin_preview_at_rate(
                    chain_id,
                    &wallet_id,
                    "0",
                    &destination_address,
                    fee_rate_svb,
                )
                .await;
        }
        let raw = self
            .fetch_utxo_fee_preview_json(chain_id, address, fee_rate_svb)
            .await?;
        Ok(crate::send::preview_decode::build_utxo_send_preview_record(
            raw,
        ))
    }
}

impl WalletService {
    pub(super) async fn preview_litecoin_owned_send(
        &self,
        chain: Chain,
        wallet_id: &str,
        amount: &str,
        destination: &str,
    ) -> Result<Option<crate::send::preview_types::BitcoinSendPreview>, SpectraBridgeError> {
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
    ) -> Result<Option<crate::send::preview_types::BitcoinSendPreview>, SpectraBridgeError> {
        let amount = u64::try_from(crate::send::amount_input::parse_raw_amount(amount, 8)?)
            .map_err(|_| SpectraBridgeError::invalid("Amount exceeds protocol range"))?;
        if amount > chain.litecoin_max_money()? {
            return Err(SpectraBridgeError::invalid(
                "Amount exceeds Litecoin MAX_MONEY",
            ));
        }
        if !destination.trim().is_empty() {
            let script = crate::derivation::utxo_address::parse_utxo_address(chain, destination)?
                .script_pubkey();
            if amount > 0
                && amount < crate::send::litecoin::litecoin_dust_threshold(chain, &script)?
            {
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
