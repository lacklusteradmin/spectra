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
        request: crate::send::SendExecutionRequest,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let service = self.clone();
        // The submission and its record finish even if the UI task is cancelled.
        tokio::spawn(async move { service.execute_send_owned(request, None).await })
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
    }
}
impl WalletService {
    pub(super) async fn execute_confirmed_send(
        &self,
        request: crate::send::SendExecutionRequest,
        sender: String,
        automatic_nonce: bool,
    ) -> Result<crate::send::SendExecutionResult, SpectraBridgeError> {
        let service = self.clone();
        tokio::spawn(async move {
            service
                .execute_send_owned(request, Some((sender, automatic_nonce)))
                .await
        })
        .await
        .map_err(|e| SpectraBridgeError::from(e.to_string()))?
    }

    async fn execute_send_owned(
        &self,
        mut request: crate::send::SendExecutionRequest,
        confirmation: Option<(String, bool)>,
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
            let chain = send_chain_for(&self.app_state().await, &request.wallet_id, chain)?;
            let signer = self
                .resolve_send_identity(
                    chain,
                    &request.wallet_id,
                    password.as_ref().map(|p| p.as_str()),
                )
                .await?;
            if confirmation.as_ref().is_some_and(|(sender, _)| crate::send::flow::normalize_address(chain.chain_display_name(), sender) != signer.from_address) {
                return Err("Sending identity changed; review again".into());
            }
            // Serialize nonce selection through durable submission for this sender.
            let _sender_guard = if wants_sign_only {
                None
            } else {
                Some(self.lock_sender(chain, &signer.from_address).await?)
            };
            let reviewed_automatic_nonce = confirmation.as_ref().is_some_and(|(_, automatic)| *automatic);
            if chain.is_evm() && reviewed_automatic_nonce {
                let next = self.next_send_nonce(chain, &signer.from_address).await?;
                if request.evm_overrides.as_ref().and_then(|o| o.nonce).and_then(|n| u64::try_from(n).ok()) != Some(next) {
                    return Err("Send nonce changed; review again".into());
                }
            }
            let reserve_nonce = chain.is_evm()
                && (reviewed_automatic_nonce || request
                    .evm_overrides
                    .as_ref()
                    .and_then(|o| o.nonce)
                    .is_none());
            if chain.is_evm() && !wants_sign_only {
                let overrides = request.evm_overrides.get_or_insert_with(Default::default);
                if overrides.nonce.is_none() {
                    overrides.nonce = Some(
                        i64::try_from(self.next_send_nonce(chain, &signer.from_address).await?)
                            .map_err(|_| "EVM nonce exceeds supported range")?,
                    );
                }
            }
            let params = self
                .build_send_params(
                    chain,
                    &request,
                    &signer.from_address,
                    &signer.private_key_hex,
                    &signer.public_key_hex,
                )
                .await?;
            let saved = Arc::new(std::sync::Mutex::new(None));
            let protocol_result = if wants_sign_only {
                self.execute_protocol_send(chain, params).await?
            } else {
                let draft = self
                    .begin_send_record(chain, &request, &signer.from_address)
                    .await?;
                let service = self.clone();
                let saved_record = saved.clone();
                let journal: crate::send::payload::SubmissionJournal =
                    Arc::new(move |submission| {
                        let mut record = draft.clone();
                        let service = service.clone();
                        let saved = saved_record.clone();
                        Box::pin(async move {
                            record.transaction_hash = submission.transaction_hash.clone();
                            record.ethereum_nonce = submission
                                .nonce
                                .map(i64::try_from)
                                .transpose()
                                .map_err(|_| "EVM nonce exceeds supported range")?;
                            record.signed_transaction_payload = Some(
                                serde_json::to_string(&submission).map_err(|e| e.to_string())?,
                            );
                            record.signed_transaction_payload_format =
                                Some("core.submission_json".into());
                            service
                                .save_prepared_send_record(record.clone(), reserve_nonce)
                                .await
                                .map_err(|e| e.to_string())?;
                            *saved.lock().map_err(|_| "send record lock poisoned")? = Some(record);
                            Ok(())
                        })
                    });
                crate::send::payload::SUBMISSION_JOURNAL
                    .scope(journal, self.execute_protocol_send(chain, params))
                    .await?
            };

            let transaction_hash = protocol_result.transaction_hash().to_string();
            let evm = protocol_result.evm()?;
            let signed_payload = if wants_sign_only {
                let payload = protocol_result.signed_payload();
                if payload.is_empty() {
                    return Err("sign-only returned no signed payload".into());
                }
                Some(payload.to_string())
            } else {
                if transaction_hash.trim().is_empty() {
                    return Err("submission returned no transaction identifier; inspect the saved submission before retrying".into());
                }
                None
            };
            let result_json = serde_json::to_string(&protocol_result)?;
            let pending = saved
                .lock()
                .map_err(|_| "send record lock poisoned")?
                .clone();
            if let Some(mut record) = pending {
                if !transaction_hash.trim().is_empty() {
                    record.transaction_hash = Some(transaction_hash.clone());
                    record.failure_reason = None;
                }
                self.save_send_record(record).await?;
            }
            Ok(crate::send::SendExecutionResult {
                protocol_result_json: result_json,
                transaction_hash,
                payload_format: crate::send::payload::format_key_for(chain.send_chain()).into(),
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
) -> Result<Chain, SpectraBridgeError> {
    let wallet = state
        .wallets
        .iter()
        .find(|w| w.id.eq_ignore_ascii_case(wallet_id))
        .ok_or_else(|| SpectraBridgeError::InvalidInput {
            message: "send wallet does not exist".into(),
        })?;
    let selected = wallet
        .network_chain(&state.settings)
        .ok_or("wallet has an invalid network identity")?;
    if selected.mainnet_counterpart() == requested.mainnet_counterpart() && selected != requested {
        return Err(
            "selected asset network differs from wallet network; select an asset on that network"
                .into(),
        );
    }
    Ok(requested)
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
        if chain.mainnet_counterpart() == Chain::Solana {
            let client = crate::fetch::chains::solana::SolanaClient::new(endpoints);
            return Ok(Some(u32::from(
                client.fetch_transfer_mint(contract).await?.1,
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
