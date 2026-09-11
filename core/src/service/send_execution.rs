use super::*;
use zeroize::Zeroizing;

/// Validate before keys or network reads; token precision is checked again
/// after metadata resolution. Native precision belongs to the registry.
fn validate_execution_amount(
    chain: Chain,
    request: &crate::send::SendExecutionRequest,
) -> Result<(), SpectraBridgeError> {
    if chain.mainnet_counterpart() == Chain::Ton {
        crate::derivation::chains::ton::parse_ton_address(&request.to_address)
            .and_then(|a| a.for_network(chain.is_testnet()))
            .map_err(|message| SpectraBridgeError::InvalidInput { message })?;
    }
    if let Some(fee) = request.fee_amount {
        crate::send::payload::fee_units(fee, u32::from(chain.native_decimals()))?;
    }
    if let Some(budget) = request.gas_budget {
        crate::send::payload::fee_units(budget, u32::from(chain.native_decimals()))?;
    }
    if let Some(rate) = request.fee_rate_svb {
        crate::send::payload::fee_units(rate, 8)?;
    }
    let decimals = if request.contract_address.is_none() {
        u32::from(chain.native_decimals())
    } else {
        request
            .amount_str
            .trim()
            .split_once('.')
            .map_or(0, |(_, f)| f.len() as u32)
    };
    let raw = crate::send::amount_input::parse_raw_amount(&request.amount_str, decimals)?;
    if raw == 0 && (!chain.is_evm() || request.contract_address.is_some()) {
        return Err(SpectraBridgeError::InvalidInput {
            message: "amount must be positive".into(),
        });
    }
    Ok(())
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Derive key material, build the chain-specific payload, sign, and
    /// broadcast in a single call.
    ///
    /// The caller selects a stored wallet. Core resolves and validates its
    /// signing identity; callers cannot supply a competing chain name or key.
    pub async fn execute_send(
        &self,
        mut request: crate::send::SendExecutionRequest,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let password = request.password.take().map(Zeroizing::new);
        let result: Result<crate::send::SendExecutionResult, SpectraBridgeError> = async {
            let chain = Chain::from_str_id(&request.chain_id).ok_or_else(|| {
                SpectraBridgeError::InvalidInput {
                    message: format!("execute_send: unsupported chain_id: {}", request.chain_id),
                }
            })?;
            // The network this wallet is on decides what is signed and where it
            // goes. Without this the chain came straight from the request —
            // the family's mainnet — so with the app switched to Sepolia a
            // send still signed chain id 1 and went to mainnet endpoints: a
            // transaction the user believed was a testnet one, valid on
            // mainnet. `spectra send broadcast --sign-only` shows the signed
            // chain id, which is how this was found.
            let chain = send_chain_for(&self.app_state().await, &request.wallet_id, chain);
            // Refuse malformed overrides before reading or deriving signing material.
            if let Some(input) = &request.evm_overrides {
                input.resolve(chain)?;
            }
            // A caller that asked for a dry run must not get a real transfer
            // because this chain's builder has no way to stop before
            // broadcasting.
            let wants_sign_only = request.wants_sign_only();
            if wants_sign_only && !chain.supports_sign_only() {
                return Err(SpectraBridgeError::InvalidInput {
                    message: format!(
                        "{} cannot sign without broadcasting",
                        chain.chain_display_name()
                    ),
                });
            }
            validate_execution_amount(chain, &request)?;
            let signer = self
                .resolve_send_identity(
                    chain,
                    &request.wallet_id,
                    password.as_ref().map(|p| p.as_str()),
                )
                .await?;
            let params = self
                .build_send_params(
                    chain,
                    &request,
                    &signer.from_address,
                    &signer.private_key_hex,
                    &signer.public_key_hex,
                )
                .await?;
            let result_json = self.sign_and_broadcast_send(chain, params).await?;

            // Classify broadcast result.
            let send_chain = chain.send_chain();
            let outcome = crate::send::payload::classify_send_broadcast_result(
                send_chain,
                result_json.clone(),
            );

            // For EVM chains, decode the typed result here so Swift doesn't
            // have to round-trip through `decode_evm_send_result(json:)`.
            let evm = if chain.is_evm() {
                let fallback_nonce = request
                    .evm_overrides
                    .as_ref()
                    .and_then(|o| o.nonce)
                    .unwrap_or(0);
                Some(crate::send::ethereum::decode_evm_send_result_internal(
                    &result_json,
                    fallback_nonce,
                ))
            } else {
                None
            };

            // What was signed, for a run that stopped there. The EVM builder
            // decodes it above; the Bitcoin builder puts it in its result JSON
            // under the same name.
            //
            // `wants_sign_only`, the same question the gate above asked: a
            // caller asking through the EVM overrides is owed the payload too.
            let signed_payload = if wants_sign_only {
                let hex = match &evm {
                    Some(evm) => evm.raw_tx_hex.clone(),
                    None => crate::send::preview_decode::extract_json_string_field(
                        result_json.clone(),
                        "raw_tx_hex".to_string(),
                    ),
                };
                if hex.is_empty() {
                    // Nothing was broadcast, so there is nothing to undo — but
                    // a dry run that reported success with no transaction to
                    // show would be `supports_sign_only` promising what this
                    // builder does not do. Say so rather than hand back an
                    // empty string that reads like a payload.
                    return Err(SpectraBridgeError::Failure {
                        message: format!(
                            "{} signed without broadcasting but returned no payload",
                            chain.chain_display_name()
                        ),
                    });
                }
                Some(hex)
            } else {
                None
            };

            Ok(crate::send::SendExecutionResult {
                rebroadcast_payload: result_json,
                transaction_hash: outcome.transaction_hash,
                payload_format: outcome.payload_format,
                evm,
                signed_payload,
            })
        }
        .await;
        request.zeroize_sensitive_fields();
        result
    }
}

/// The chain a send is signed for: the network the wallet is on, within the
/// family the request named.
///
/// A wallet with no record of its own follows the app's selection. A request
/// naming a different family — which `resolve_send_identity` refuses anyway —
/// keeps the requested chain, so this cannot move a send onto another chain.
pub(crate) fn send_chain_for(
    state: &crate::store::state::CoreAppState,
    wallet_id: &str,
    requested: Chain,
) -> Chain {
    state
        .wallets
        .iter()
        .find(|wallet| wallet.id.eq_ignore_ascii_case(wallet_id))
        .and_then(|wallet| wallet.network_chain(&state.settings))
        .filter(|network| network.mainnet_counterpart() == requested.mainnet_counterpart())
        .unwrap_or(requested)
}

impl WalletService {
    /// None means no metadata reader for this family; provider failures are errors.
    async fn token_contract_decimals(
        &self,
        chain: Chain,
        contract: &str,
    ) -> Result<Option<u32>, SpectraBridgeError> {
        let endpoints = self.endpoints_for(chain.str_id()).await;
        if chain.is_evm() {
            let client = crate::fetch::chains::evm::EvmClient::new(endpoints, chain.evm_chain_id());
            return Ok(Some(u32::from(
                client.fetch_erc20_metadata(contract).await?.decimals,
            )));
        }
        if chain.mainnet_counterpart() == Chain::Tron {
            let client = crate::fetch::chains::tron::TronClient::new(endpoints);
            return Ok(Some(u32::from(
                client.fetch_trc20_metadata(contract).await?.decimals,
            )));
        }
        if chain.mainnet_counterpart() == Chain::Near {
            let client = crate::fetch::chains::near::NearClient::new(endpoints);
            return Ok(Some(u32::from(
                client.fetch_ft_metadata(contract).await?.decimals,
            )));
        }
        Ok(None)
    }

    /// Resolve an exact decimal amount into the protocol's integer units.
    async fn build_send_params(
        &self,
        chain: Chain,
        req: &crate::send::SendExecutionRequest,
        from_address: &str,
        priv_hex: &str,
        pub_hex: &Option<String>,
    ) -> Result<crate::service::send_params::ExecuteSendParams, SpectraBridgeError> {
        use crate::send::amount_input::parse_raw_amount;
        use crate::send::payload::{dogecoin_fee, fee_units};
        use crate::service::send_params::*;

        let mut overrides = req
            .evm_overrides
            .as_ref()
            .map(|input| input.resolve(chain))
            .transpose()?
            .unwrap_or_default();
        // One question for every chain and every route. The EVM builder read
        // it off the overrides and the Bitcoin builder had it hard-coded to
        // `false`, so "sign but do not broadcast" was reachable on one family
        // and by one route.
        overrides.sign_only = req.wants_sign_only();

        let from = from_address.to_string();
        let to = req.to_address.clone();
        let private_key_hex = crate::send::keys::SecretHex::from(priv_hex.to_string());
        let public_key_hex = pub_hex.clone();

        let raw_u128 = |dec: u32| parse_raw_amount(&req.amount_str, dec);
        let raw_u64 = |dec: u32| -> Result<u64, SpectraBridgeError> {
            u64::try_from(raw_u128(dec)?).map_err(|_| SpectraBridgeError::InvalidInput {
                message: "amount exceeds this protocol's u64 range".into(),
            })
        };
        validate_execution_amount(chain, req)?;

        if let Some(ref contract) = req.contract_address {
            // Only families without a metadata reader may use caller-supplied
            // precision. A failed read on EVM/Tron must stop the send.
            let decimals = self
                .token_contract_decimals(chain, contract)
                .await?
                .or(req.token_decimals)
                .ok_or_else(|| {
                    SpectraBridgeError::from(format!(
                        "execute_send: {contract} did not report its decimals and none were supplied"
                    ))
                })?;
            let params = match chain {
                c if c.is_evm() => SendTokenParams::Evm(
                    TokenAmountSendParams {
                        from,
                        contract: contract.clone(),
                        to,
                        amount_raw: raw_u128(decimals)?,
                        private_key_hex,
                    },
                    overrides,
                ),
                Chain::Tron => SendTokenParams::Tron(TronTokenSendParams {
                    from,
                    contract: contract.clone(),
                    to,
                    amount_raw: raw_u128(decimals)?,
                    fee_limit_sun: None,
                    private_key_hex,
                }),
                Chain::Solana => SendTokenParams::Solana(SolanaTokenSendParams {
                    from_pubkey_hex: public_key_hex.unwrap_or_default(),
                    to,
                    mint: contract.clone(),
                    amount_raw: raw_u64(decimals)?,
                    decimals: u8::try_from(decimals)
                        .map_err(|_| SpectraBridgeError::from("token decimals out of range"))?,
                    private_key_hex,
                }),
                Chain::Near => SendTokenParams::Near(NearTokenSendParams {
                    from,
                    contract: contract.clone(),
                    to,
                    amount_raw: raw_u128(decimals)?,
                    private_key_hex,
                    public_key_hex: public_key_hex.unwrap_or_default(),
                    gas_tgas: None,
                }),
                c => {
                    return Err(SpectraBridgeError::from(format!(
                        "execute_send: unsupported token chain: {c:?}"
                    )))
                }
            };
            return Ok(ExecuteSendParams::Token(params));
        }

        // Takes `from`/`to`/`private_key_hex` as parameters rather than
        // capturing the outer bindings: several arms below move those same
        // bindings by value (building a different struct), and a closure
        // that borrowed them for `.clone()` would hold that borrow live
        // across the whole match, conflicting with the moves.
        let utxo = |from: String,
                    to: String,
                    private_key_hex: crate::send::keys::SecretHex,
                    amount_sat: u64,
                    fee_sat: Option<u64>| {
            UtxoFixedFeeSendParams {
                from,
                to,
                amount_sat,
                fee_sat,
                private_key_hex,
                dust_threshold_sats: None,
            }
        };
        // The fallback fee is the chain's own — `Chain::static_fee_units`, which
        // is where the fee shown on the send screen comes from. It used to be a
        // literal per call site, and two of them disagreed with what the user
        // had just been shown: Litecoin signed 10_000 against a 1_000 estimate,
        // Bitcoin Cash 1_000 against 2_000.
        let scaled_amount = || -> Result<(u64, u64), SpectraBridgeError> {
            Ok((
                raw_u64(u32::from(chain.native_decimals()))?,
                req.fee_sat
                    .unwrap_or_else(|| chain.static_fee_units().unwrap_or_default() as u64),
            ))
        };

        let params = match chain {
            Chain::Bitcoin => SendParams::Bitcoin(BitcoinNativeSendParams {
                from,
                to,
                amount_sat: raw_u64(8)?,
                fee_rate_svb: Some(req.fee_rate_svb.unwrap_or(10.0)),
                private_key_hex,
                dust_threshold_sats: None,
                sign_only: req.wants_sign_only(),
            }),
            c if c.is_evm() => SendParams::Evm(
                EvmNativeSendParams {
                    from,
                    to,
                    value_wei: raw_u128(18)?,
                    private_key_hex,
                },
                overrides,
            ),
            Chain::Solana => SendParams::Solana(SolanaNativeSendParams {
                from_pubkey_hex: public_key_hex.unwrap_or_default(),
                to,
                lamports: raw_u64(9)?,
                private_key_hex,
            }),
            Chain::Dogecoin => {
                // Dogecoin estimates a 350-byte fee from DOGE/kB.
                let fee_rate_doge_per_kb = req.fee_rate_svb.unwrap_or(0.01);
                SendParams::Utxo(UtxoFixedFeeSendParams {
                    from,
                    to,
                    amount_sat: raw_u64(8)?,
                    fee_sat: Some(dogecoin_fee(fee_rate_doge_per_kb)?),
                    private_key_hex,
                    dust_threshold_sats: None,
                })
            }
            Chain::Xrp => SendParams::Xrp(XrpSendParams {
                from,
                to,
                drops: raw_u64(6)?,
                private_key_hex,
                public_key_hex,
            }),
            Chain::Litecoin => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::BitcoinCash => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Tron => SendParams::Tron(TronNativeSendParams {
                from,
                to,
                amount_sun: raw_u64(6)?,
                private_key_hex,
            }),
            Chain::Stellar => SendParams::Stellar(StellarSendParams {
                from,
                to,
                stroops: i64::try_from(raw_u128(7)?).map_err(|_| {
                    SpectraBridgeError::from("amount exceeds Stellar integer range")
                })?,
                private_key_hex,
                public_key_hex,
                network_passphrase: None,
            }),
            Chain::Cardano => SendParams::Cardano(CardanoSendParams {
                from,
                to,
                amount_lovelace: raw_u64(6)?,
                fee_lovelace: Some(fee_units(req.fee_amount.unwrap_or(0.17), 6)?),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                ttl_slots: None,
                min_change_lovelace: None,
            }),
            Chain::Polkadot => SendParams::Polkadot(PolkadotSendParams {
                from,
                to,
                planck: raw_u128(10)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                era: None,
                tip: None,
            }),
            Chain::Bittensor => SendParams::Bittensor(BittensorSendParams {
                from,
                to,
                rao: raw_u128(9)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Sui => SendParams::Sui(SuiSendParams {
                from,
                to,
                mist: raw_u64(9)?,
                gas_budget: Some(fee_units(req.gas_budget.unwrap_or(0.01), 9)?),
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Aptos => SendParams::Aptos(AptosSendParams {
                from,
                to,
                octas: raw_u64(8)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Ton => SendParams::Ton(TonSendParams {
                from,
                to,
                nanotons: raw_u64(9)?,
                comment: None,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
                subwallet_id: None,
                expiry_seconds: None,
                send_mode: None,
            }),
            Chain::Near => SendParams::Near(NearNativeSendParams {
                from,
                to,
                yocto_near: raw_u128(24)?,
                private_key_hex,
                public_key_hex: public_key_hex.unwrap_or_default(),
            }),
            Chain::Icp => SendParams::Icp(IcpSendParams {
                from,
                to,
                e8s: raw_u64(8)?,
                private_key_hex,
                public_key_hex,
            }),
            Chain::Monero => SendParams::Monero(MoneroSendParams {
                from,
                to,
                piconeros: raw_u64(12)?,
                priority: Some(u64::from(req.monero_priority.unwrap_or(2))),
            }),
            Chain::BitcoinSV => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Zcash => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Zcash(ZcashSendParams {
                    from,
                    to,
                    amount_sat,
                    fee_sat: Some(fee_sat),
                    private_key_hex,
                    dust_threshold_zats: None,
                })
            }
            Chain::BitcoinGold => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            Chain::Decred => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Decred(DecredSendParams {
                    from,
                    to,
                    amount_sat,
                    fee_sat: Some(fee_sat),
                    private_key_hex,
                    dust_threshold_atoms: None,
                })
            }
            Chain::Kaspa => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Kaspa(KaspaSendParams {
                    from,
                    to,
                    amount_sat,
                    fee_sat: Some(fee_sat),
                    private_key_hex,
                    min_fee_sompi: None,
                    dust_threshold_sompi: None,
                })
            }
            Chain::Dash => {
                let (amount_sat, fee_sat) = scaled_amount()?;
                SendParams::Utxo(utxo(from, to, private_key_hex, amount_sat, Some(fee_sat)))
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "execute_send: unsupported chain: {c:?}"
                )))
            }
        };
        Ok(ExecuteSendParams::Native(params))
    }
}

#[cfg(test)]
mod audit_execution_tests;
#[cfg(test)]
mod tests;
