//! Send previews and fee estimates. Signing and rebroadcast live in sibling modules.
use super::*;
#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Typed EVM send preview: fetches the raw preview JSON then decodes it
    /// into `EvmSendPreview` with the caller-supplied nonce / fee
    /// overrides applied. Returns `None` when the decoder rejects the payload.
    pub async fn fetch_evm_send_preview_typed(
        &self,
        chain_id: String,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
        explicit_nonce: Option<i64>,
        custom_fees: Option<crate::ethereum_send::EvmCustomFeeConfiguration>,
    ) -> Result<Option<crate::wallet_core::EvmSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_evm_send_preview(&chain_id, from, to, value_wei, data_hex)
            .await?;
        Ok(crate::send::preview_decode::build_evm_send_preview_record(
            crate::ethereum_send::EvmPreviewDecodeInput {
                raw_json: raw,
                explicit_nonce,
                custom_fees,
            },
        ))
    }
    /// Typed Tron send preview wrapper around `fetch_tron_send_preview` +
    /// `build_tron_send_preview_record`.
    pub async fn fetch_tron_send_preview_typed(
        &self,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<Option<crate::wallet_core::TronSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_tron_send_preview(address, symbol, contract_address)
            .await?;
        Ok(crate::send::preview_decode::build_tron_send_preview_record(
            raw,
        ))
    }

    /// Typed UTXO fee preview wrapper (BTC / LTC / BCH / BSV single-address
    /// flow). Fuses `fetch_utxo_fee_preview` + `build_utxo_send_preview_record`.
    pub async fn fetch_utxo_fee_preview_typed(
        &self,
        chain_id: String,
        address: String,
        fee_rate_svb: u64,
        destination_address: String,
    ) -> Result<Option<crate::wallet_core::BitcoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview(&chain_id, address, fee_rate_svb)
            .await?;
        let preview = crate::send::preview_decode::build_utxo_send_preview_record(raw);
        let overhead = Chain::from_str_id(&chain_id)
            .map(|chain| chain.extra_output_overhead_bytes(&destination_address))
            .unwrap_or(0);
        Ok(preview.map(|preview| {
            crate::send::preview_decode::with_extra_output_overhead(preview, overhead)
        }))
    }

    /// Typed Dogecoin send preview: runs the UTXO fee-preview fetch on the
    /// Dogecoin chain then decodes with the requested amount + fee priority.
    pub async fn fetch_dogecoin_send_preview_typed(
        &self,
        address: String,
        requested_amount: f64,
        fee_priority: String,
    ) -> Result<Option<crate::wallet_core::DogecoinSendPreview>, SpectraBridgeError> {
        let raw = self
            .fetch_utxo_fee_preview(Chain::Dogecoin.str_id(), address, 0)
            .await?;
        Ok(
            crate::send::preview_decode::build_dogecoin_send_preview_record(
                raw,
                requested_amount,
                fee_priority,
            ),
        )
    }

    /// Typed Bitcoin HD send preview: concurrently fetches the xpub balance
    /// and the Bitcoin fee estimate then decodes into `BitcoinSendPreview`.
    ///
    /// `chain_id` is the network the wallet is on, the way the balance refresh
    /// takes it. It used to be Bitcoin's mainnet whatever was selected, which
    /// only ever cost a wrong fee estimate — until `execute_send` started
    /// following the wallet's network: a testnet send was then priced, and its
    /// spendable balance read, against mainnet.
    pub async fn fetch_bitcoin_hd_send_preview_typed(
        &self,
        chain_id: String,
        xpub: String,
        receive_count: u32,
        change_count: u32,
    ) -> Result<Option<crate::wallet_core::BitcoinSendPreview>, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id)
            .filter(|chain| chain.mainnet_counterpart() == Chain::Bitcoin)
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: format!("{chain_id:?} is not a Bitcoin network"),
            })?;
        let (balance, rate) = tokio::try_join!(
            self.bitcoin_xpub_balance(chain.str_id(), xpub, receive_count, change_count),
            self.bitcoin_fee_rate(chain),
        )?;
        Ok(
            crate::send::preview_decode::build_bitcoin_hd_send_preview_record(
                balance.confirmed_sats,
                rate.sats_per_vbyte,
            ),
        )
    }

    /// Typed simple-chain send preview: fuses `fetch_simple_chain_send_preview`
    /// + `build_simple_chain_preview` so Swift never sees the intermediate JSON.
    /// The `chain: SimpleChain` argument is gone: it was derivable from
    /// `chain_id`, and the only way for a caller to get one was an eleven-entry
    /// table in Swift keyed by display name — a second spelling of the registry,
    /// handed back to the registry's owner.
    pub async fn fetch_simple_chain_send_preview_typed(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<crate::send::preview_decode::SimpleChainPreview, SpectraBridgeError> {
        let chain = crate::registry::Chain::from_str_id(&chain_id)
            .and_then(|chain| chain.simple_preview_chain())
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: format!("{chain_id} has no shared-path send preview"),
            })?;
        let raw = self
            .fetch_simple_chain_send_preview(&chain_id, address)
            .await?;
        Ok(crate::send::preview_decode::build_simple_chain_preview(
            raw, chain,
        ))
    }
}

impl WalletService {
    /// Bitcoin's fee rate, in sat/vB.
    ///
    /// Split out of a `fetch_fee_estimate` that returned three different JSON
    /// shapes by chain — this one, EVM's `EvmFeeEstimate`, and a flat native
    /// amount for everyone else — as a `String` its callers parsed back. The
    /// EVM arm had no reachable caller at all: EVM previews build their own
    /// `EvmClient` and call `fetch_fee_estimate()` on it directly, so nothing
    /// ever asked this function for an EVM chain. What is left is two shapes
    /// with one caller each, so each caller gets its own typed function and
    /// neither goes through JSON.
    ///
    /// `chain` is the Bitcoin network to quote for: a testnet's fees are its
    /// own, and reading mainnet's for it was a number about a different chain.
    pub(crate) async fn bitcoin_fee_rate(
        &self,
        chain: Chain,
    ) -> Result<crate::fetch::chains::bitcoin::FeeRate, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let client = BitcoinClient::new(HttpClient::shared(), endpoints);
        Ok(client.fetch_fee_rate(6).await?)
    }

    /// A chain's fee quoted in its own native unit, live where the chain has
    /// an RPC that answers and static where the catalog carries the number.
    ///
    /// Only the eleven chains `simple_preview_chain` covers reach this, which
    /// is why Bitcoin and EVM have no arm: they have their own preview paths.
    pub(crate) async fn native_fee_estimate(
        &self,
        chain: Chain,
    ) -> Result<NativeFeeEstimate, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain.str_id()).await;
        let native = |raw: u128, source: &'static str| NativeFeeEstimate {
            raw: raw.to_string(),
            display: format_decimals(raw, chain.native_decimals()),
            source,
        };
        match chain {
            // Chains with live RPC fee fetches.
            Chain::Xrp => {
                let drops = XrpClient::new(endpoints).fetch_fee().await?;
                Ok(native(drops as u128, "rpc"))
            }
            Chain::Stellar => {
                let stroops = StellarClient::new(endpoints).fetch_base_fee().await?;
                Ok(native(stroops as u128, "rpc"))
            }
            Chain::Aptos => {
                let price = AptosClient::new(endpoints).fetch_gas_price().await?;
                Ok(native(price as u128, "rpc"))
            }
            // NEAR's static fee overflows u128 — carry it as the string it is.
            Chain::Near => Ok(NativeFeeEstimate {
                raw: "1000000000000000000000".to_string(),
                display: "0.001".to_string(),
                source: "static",
            }),
            // Every remaining supported chain returns a flat static fee from
            // `Chain::static_fee_units`. One arm replaces 18 near-identical ones.
            other => match other.static_fee_units() {
                Some(units) => Ok(native(units, "static")),
                None => Err(SpectraBridgeError::from(format!(
                    "fee estimation not supported for {}",
                    other.chain_display_name()
                ))),
            },
        }
    }

    pub(crate) async fn fetch_utxo_fee_preview(
        &self,
        chain_id: &str,
        address: String,
        fee_rate_svb: u64,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_utxo_fee_preview: unsupported chain_id: {chain_id}"
            ))
        })?;
        let eps = self.endpoints_for(chain.str_id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client
                        .fetch_fee_rate(3)
                        .await
                        .map(|r| r.sats_per_vbyte.ceil() as u64)
                        .unwrap_or(5)
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_koin).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client.fetch_fee_rate(3).await
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 {
                    fee_rate_svb
                } else {
                    client.fetch_fee_rate(3).await
                };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let utxos = client.fetch_utxos(&address).await?;
                let rate = if fee_rate_svb > 0 { fee_rate_svb } else { 1 };
                let values: Vec<u64> = utxos.into_iter().map(|u| u.value_sat).collect();
                Ok(utxo_fee_preview_json(values, rate))
            }
            c => Err(SpectraBridgeError::from(format!(
                "fetch_utxo_fee_preview: unsupported chain: {c:?}"
            ))),
        }
    }

    /// Quote an EVM send: nonce, fee, gas limit, and what is spendable.
    ///
    /// "Spendable" is a fact about the asset the amount field moves, and this
    /// used to answer it with the native balance whatever was being sent — so
    /// a USDC send offered the sender's ETH balance as its maximum, rendered
    /// with USDC's own formatter. Gas is always paid in the chain's coin, but
    /// the amount is not always denominated in it:
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
    pub(crate) async fn fetch_evm_send_preview(
        &self,
        chain_id: &str,
        from: String,
        to: String,
        value_wei: String,
        data_hex: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = chain_for_evm_id(chain_id)?;
        let eps = self.endpoints_for(chain.str_id()).await;
        let client = EvmClient::new(eps, chain.evm_chain_id());

        if value_wei.is_empty() || !value_wei.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SpectraBridgeError::from(
                "value_wei must be an unsigned integer",
            ));
        }
        let value_u128: u128 = value_wei
            .parse()
            .map_err(|_| SpectraBridgeError::from("value_wei exceeds u128 range"))?;
        let data_opt: Option<&str> = if data_hex == "0x" || data_hex.is_empty() {
            None
        } else {
            Some(&data_hex)
        };
        // An ERC-20 transfer is addressed to the token contract, so the
        // destination *is* the token whose balance this send spends.
        let token_contract = data_opt
            .filter(|data| crate::fetch::chains::evm::is_erc20_transfer(data))
            .map(|_| to.as_str());

        let (nonce_res, fee_res, gas_res, bal_res, token_res) = tokio::join!(
            client.fetch_nonce(&from),
            client.fetch_fee_estimate(),
            client.estimate_gas(&from, &to, value_u128, data_opt),
            client.fetch_balance(&from),
            async {
                match token_contract {
                    Some(contract) => Some(client.fetch_erc20_balance(contract, &from).await),
                    None => None,
                }
            }
        );

        let nonce = nonce_res?;
        let fee = fee_res?;
        let gas_limit = gas_res?;
        let balance_wei_val: u128 = bal_res?
            .balance_wei
            .parse()
            .map_err(|_| SpectraBridgeError::from("invalid EVM balance"))?;

        let estimated_fee_wei: u128 = (gas_limit as u128).saturating_mul(fee.max_fee_per_gas_wei);
        let max_fee_gwei = fee.max_fee_per_gas_wei as f64 / 1_000_000_000.0;
        let priority_fee_gwei = fee.priority_fee_wei as f64 / 1_000_000_000.0;
        let estimated_fee_eth = estimated_fee_wei as f64 / 1e18;

        // A token read that failed is not a zero holding: everything the send
        // sheet decides from this — whether the amount fits, whether it is the
        // whole balance — would be decided against a number nobody read.
        let spendable_balance = match token_res {
            Some(token) => {
                let token = token.map_err(SpectraBridgeError::from)?;
                let raw: u128 = token
                    .balance_raw
                    .parse()
                    .map_err(|_| SpectraBridgeError::from("invalid ERC-20 balance"))?;
                token_display_balance(raw, token.decimals)
            }
            None => (balance_wei_val.saturating_sub(estimated_fee_wei)) as f64 / 1e18,
        };

        Ok(json!({
            "nonce": nonce,
            "gas_limit": gas_limit,
            "max_fee_per_gas_gwei": max_fee_gwei,
            "max_priority_fee_per_gas_gwei": priority_fee_gwei,
            "estimated_fee_eth": estimated_fee_eth,
            "spendable_balance": spendable_balance,
            "is_token": crate::fetch::chains::evm::is_erc20_transfer(&data_hex),
            "native_balance_wei": balance_wei_val.to_string(),
            "fee_rate_description": format!("Max {:.2} gwei / Priority {:.2} gwei",
                max_fee_gwei, priority_fee_gwei),
        })
        .to_string())
    }

    pub(crate) async fn fetch_tron_send_preview(
        &self,
        address: String,
        symbol: String,
        contract_address: String,
    ) -> Result<String, SpectraBridgeError> {
        let eps = self.endpoints_for("tron").await;
        let client = TronClient::new(eps);

        // The native asset, by the catalog's gas token rather than the string
        // "TRX" — the same fact the rest of the send path routes on.
        if symbol == Chain::Tron.coin_symbol() || contract_address.is_empty() {
            // TRX is the fee asset as well as the amount, so the fee comes out
            // of what is spendable. Only this branch needs the TRX balance;
            // reading it for a token send too was a wasted call whose result
            // nothing looked at.
            let trx_balance = client
                .fetch_balance(&address)
                .await
                .map(|b| b.sun as f64 / 1_000_000.0)
                .map_err(SpectraBridgeError::from)?;
            let fee_trx = 1.0_f64;
            let spendable = (trx_balance - fee_trx).max(0.0);
            return Ok(json!({
                "estimated_fee_trx": fee_trx,
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
            .map_err(|_| SpectraBridgeError::from("invalid TRC-20 balance"))?;
        let token_balance = token_display_balance(raw, token.decimals);

        let fee_trx = 15.0_f64;
        let fee_limit_sun: i64 = 15_000_000;
        Ok(json!({
            "estimated_fee_trx": fee_trx,
            "fee_limit_sun": fee_limit_sun,
            "spendable_balance": token_balance,
            "max_sendable": token_balance,
            "fee_rate_description": "Static energy estimate",
        })
        .to_string())
    }

    pub(crate) async fn fetch_simple_chain_send_preview(
        &self,
        chain_id: &str,
        address: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id)
            .ok_or_else(|| SpectraBridgeError::from(format!("unknown chain_id: {chain_id}")))?;
        let (fee, balance) = tokio::try_join!(
            self.native_fee_estimate(chain),
            self.fetch_native_balance_summary(chain_id.to_string(), address),
        )?;

        let fee_display = fee.display.parse::<f64>().unwrap_or(0.0);
        let fee_raw = fee.raw;
        let fee_rate_description = fee.source.to_string();

        let balance_display = summary_display_balance(chain_id, &balance);
        let max_sendable = (balance_display - fee_display).max(0.0);

        Ok(json!({
            "fee_display":          fee_display,
            "fee_raw":              fee_raw,
            "fee_rate_description": fee_rate_description,
            "balance_display":      balance_display,
            "max_sendable":         max_sendable,
        })
        .to_string())
    }
}

#[cfg(test)]
#[path = "send/tests.rs"]
mod tests;
