//! Everything between "the user pressed send" and "the network has it":
//! fee estimation, per-chain signing, broadcast, and the previews that quote a
//! send before it is authorised.
//!
//! Signing material arrives per call and is scrubbed after use — nothing in
//! this module outlives the call that supplied it. New arms take a typed
//! record from [`super::send_params`], never a fresh ad-hoc JSON shape.

use super::*;

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Does this destination look unused for the asset this holding sends?
    ///
    /// Named by wallet and holding, not by chain and token descriptor. Which
    /// contract a symbol means is a catalog question and the catalog is core's,
    /// so the caller that used to answer it was reading core's token
    /// preferences to hand them straight back: the composer decided native vs
    /// token, built the descriptor, clamped its decimals into a `u8`, and
    /// silently showed no verdict at all when it could not identify the token.
    ///
    /// Swift also ran the read itself as four chain arms — Bitcoin, "every EVM
    /// chain", Tron, and everything else — that fetched different things and
    /// worded the answer three different ways. Two of the three wordings were
    /// built by string interpolation and never reached the locale files, so a
    /// Tron or EVM token send showed an English warning in a Chinese app.
    ///
    /// The history signal is one question now: has this address transacted on
    /// this chain. EVM adds the nonce because the balance probe returns it
    /// anyway and it needs no explorer key. Bitcoin used to ask
    /// `utxo_count > 0`, which is not that question — an address that received
    /// and later spent everything has history and no UTXOs, and got a warning
    /// stating it had "no transaction history", which was false.
    ///
    /// `destination_input` is what the user typed. Resolving it here rather
    /// than trusting a caller-supplied address keeps the probe asking about
    /// the address a send would actually reach; for one already resolved it is
    /// re-validation and nothing more.
    pub async fn send_destination_risk(
        &self,
        wallet_id: String,
        holding_key: String,
        destination_input: String,
    ) -> Result<SendDestinationRisk, SpectraBridgeError> {
        let (chain, token) = {
            let state = self.wallet_state.read().await;
            let holding = state
                .wallets
                .iter()
                .find(|w| w.id == wallet_id)
                .and_then(|wallet| {
                    wallet
                        .holdings
                        .iter()
                        .find(|h| format!("{}|{}", h.chain_name, h.symbol) == holding_key)
                })
                .ok_or_else(|| SpectraBridgeError::InvalidInput {
                    message: format!("no holding {holding_key} on wallet {wallet_id}"),
                })?;
            destination_probe_asset(holding, &state.token_preferences)?
        };
        let chain_id = chain.str_id().to_string();
        let address = self
            .resolve_send_destination(chain_id.clone(), destination_input)
            .await?
            .address;

        let balance_read = async {
            let display = match token {
                Some(descriptor) => self
                    .fetch_token_balances(chain_id.clone(), address.clone(), vec![descriptor])
                    .await?
                    .first()
                    .ok_or_else(|| SpectraBridgeError::from("token balance unavailable"))?
                    .balance_display
                    .clone(),
                None => {
                    self.fetch_native_balance_summary(chain_id.clone(), address.clone())
                        .await?
                        .amount_display
                }
            };
            let balance = display
                .parse::<f64>()
                .map_err(|_| SpectraBridgeError::from("invalid destination balance"))?;
            if !balance.is_finite() || balance < 0.0 {
                return Err(SpectraBridgeError::from("invalid destination balance"));
            }
            Ok::<_, SpectraBridgeError>(balance)
        };
        let history_read = async {
            // A positive nonce proves activity without an explorer lookup. Zero
            // alone cannot rule out incoming transfers; ask history in that case.
            if chain.is_evm() {
                let client = EvmClient::new(
                    self.endpoints_for(chain.str_id()).await,
                    chain.evm_chain_id(),
                );
                if client.fetch_nonce(&address).await? > 0 {
                    return Ok(true);
                }
            }
            Ok::<_, SpectraBridgeError>(
                self.fetch_history_summary(chain_id.clone(), address.clone())
                    .await?
                    .entry_count
                    > 0,
            )
        };
        let (balance, has_history) = tokio::try_join!(balance_read, history_read)?;

        Ok(SendDestinationRisk {
            balance_is_zero: balance <= 0.0,
            has_history,
        })
    }

    /// The address this send is going to, from whatever the user typed.
    ///
    /// Three Swift call sites answered this — the recipient probe, the EVM
    /// preview and submit — and each spelled the ENS rule as
    /// `chainName == "Ethereum"`, kept its own resolved-name cache or went
    /// without one, and normalized a resolved address in two of the three.
    /// The rule is `Chain::resolves_ens_names`, the cache is this service's,
    /// and the address comes back in the form everything downstream compares
    /// against: an address the address book holds and one a name resolved to
    /// now match, so a name-reached destination is no longer reported as new.
    ///
    /// A name that does not resolve, and any input that is neither a valid
    /// address for the chain nor a name it will look up, is an error. Nothing
    /// here guesses a destination.
    pub async fn resolve_send_destination(
        &self,
        chain_id: String,
        input: String,
    ) -> Result<SendDestinationResolution, SpectraBridgeError> {
        let chain = chain_for_id(&chain_id)?;
        let chain_name = chain.chain_display_name();
        let typed = input.trim().to_string();
        if typed.is_empty() {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("enter a {chain_name} destination address"),
            });
        }
        if crate::send::flow::is_valid_send_address(chain_name.to_string(), typed.clone()) {
            return Ok(SendDestinationResolution {
                address: crate::send::flow::normalized_send_address(chain_name.to_string(), typed),
                used_ens: false,
            });
        }
        if !(chain.resolves_ens_names() && crate::send::flow::is_ens_name_candidate(&typed)) {
            return Err(SpectraBridgeError::InvalidInput {
                message: format!("enter a valid {chain_name} destination address"),
            });
        }
        let key = typed.to_lowercase();
        if let Some(hit) = self.ens_resolutions.read().await.get(&key).cloned() {
            return Ok(SendDestinationResolution {
                address: hit,
                used_ens: true,
            });
        }
        let resolved = self
            .resolve_ens_name_typed(typed.clone())
            .await?
            .ok_or_else(|| SpectraBridgeError::InvalidInput {
                message: format!("unable to resolve ENS name '{typed}'"),
            })?;
        // The resolver answers in whatever case it likes; store and return the
        // chain's own form so a cache hit and a fresh lookup are the same
        // string.
        let address = crate::send::flow::normalized_send_address(chain_name.to_string(), resolved);
        self.ens_resolutions
            .write()
            .await
            .insert(key, address.clone());
        Ok(SendDestinationResolution {
            address,
            used_ens: true,
        })
    }

    /// Typed wrapper around `broadcast_raw`: runs the broadcast then extracts
    /// the named field (typically `"txid"` or `"digest"`) from the result JSON.
    /// Returns the field value as a string, or an empty string when missing.
    pub async fn broadcast_raw_extract(
        &self,
        chain_id: String,
        payload: String,
        result_field: String,
    ) -> Result<String, SpectraBridgeError> {
        let json = self.broadcast_raw(&chain_id, payload).await?;
        Ok(crate::send::preview_decode::extract_json_string_field(
            json,
            result_field,
        ))
    }
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

    /// Sign and broadcast, from an already-typed [`ExecuteSendParams`] —
    /// what `sign_and_send` and `sign_and_send_token` did as two separate
    /// JSON-in functions, merged into the one match their data now carries
    /// its own chain identity for. See `build_send_params` for what used to
    /// sit on the other side of the JSON these arms used to parse.
    pub(crate) async fn sign_and_broadcast_send(
        &self,
        chain: Chain,
        params: crate::service::send_params::ExecuteSendParams,
    ) -> Result<String, SpectraBridgeError> {
        use crate::service::send_params::ExecuteSendParams;
        let endpoints = self.endpoints_for(chain.str_id()).await;
        match params {
            ExecuteSendParams::Native(native) => {
                self.sign_and_broadcast_native(chain, native, endpoints)
                    .await
            }
            ExecuteSendParams::Token(token) => {
                self.sign_and_broadcast_token(chain, token, endpoints).await
            }
        }
    }

    /// Native transfers. The UTXO-model chains come first because four of
    /// them share `sign_and_broadcast_shared_utxo`; the account-model chains
    /// each build their own client and are otherwise one arm apiece.
    async fn sign_and_broadcast_native(
        &self,
        chain: Chain,
        native: crate::service::send_params::SendParams,
        endpoints: Arc<Vec<String>>,
    ) -> Result<String, SpectraBridgeError> {
        use crate::service::send_params::SendParams;
        match native {
            SendParams::Bitcoin(p) => {
                let client = BitcoinClient::new(HttpClient::shared(), endpoints);
                let send_params = BitcoinSendParams {
                    from_address: p.from,
                    private_key_hex: p.private_key_hex,
                    to_address: p.to,
                    amount_sats: p.amount_sat,
                    fee_rate: crate::fetch::chains::bitcoin::FeeRate {
                        sats_per_vbyte: p.fee_rate_svb.unwrap_or(10.0),
                    },
                    available_utxos: vec![],
                    network_chain_id: crate::registry::Chain::Bitcoin.str_id().to_string(),
                    enable_rbf: true,
                    dust_threshold: p.dust_threshold_sats,
                    pinned_utxos: None,
                    extra_outputs: vec![],
                    coin_selection: crate::send::chains::bitcoin::CoinSelectionStrategy::default(),
                    sign_only: p.sign_only,
                };
                let r = bitcoin_sign_and_broadcast(&client, send_params).await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Utxo(p) => {
                self.sign_and_broadcast_shared_utxo(chain, p, endpoints)
                    .await
            }
            SendParams::Zcash(p) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = ZcashClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(
                        &p.from,
                        &p.to,
                        p.amount_sat,
                        fee_or_static(chain, p.fee_sat),
                        &priv_bytes,
                        crate::send::chains::zcash::ZcashNetworkUpgrade::NU5,
                        p.dust_threshold_zats.unwrap_or(546),
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Decred(p) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = DecredClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(
                        &p.from,
                        &p.to,
                        p.amount_sat,
                        fee_or_static(chain, p.fee_sat),
                        &priv_bytes,
                        p.dust_threshold_atoms,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Kaspa(p) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = KaspaClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(
                        &p.from,
                        &p.to,
                        p.amount_sat,
                        fee_or_static(chain, p.fee_sat),
                        &priv_bytes,
                        p.min_fee_sompi,
                        p.dust_threshold_sompi,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Evm(p, overrides) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = EvmClient::new(endpoints, chain.evm_chain_id());
                let r = client
                    .sign_and_broadcast_with_overrides(
                        &p.from,
                        &p.to,
                        p.value_wei,
                        &priv_bytes,
                        overrides,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Solana(p) => {
                let from_arr: [u8; 32] = decode_hex_array(&p.from_pubkey_hex, "from_pubkey_hex")?;
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let client = SolanaClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(&from_arr, &p.to, p.lamports, &priv_arr)
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Xrp(p) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                // public_key_hex is optional: derive compressed secp256k1 pubkey when absent.
                let derived_pub: String;
                let pub_hex: &str = match p.public_key_hex.as_deref().filter(|s| !s.is_empty()) {
                    Some(s) => s,
                    None => {
                        use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey};
                        let secp = Secp256k1::new();
                        let secret = SecretKey::from_slice(&priv_bytes)
                            .map_err(|e| format!("bad privkey: {e}"))?;
                        derived_pub =
                            hex::encode(SecpPubKey::from_secret_key(&secp, &secret).serialize());
                        &derived_pub
                    }
                };
                let client = XrpClient::new(endpoints);
                let r = client
                    .sign_and_submit(&p.from, &p.to, p.drops, &priv_bytes, pub_hex)
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Tron(p) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = TronClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(&p.from, &p.to, p.amount_sun, &priv_bytes)
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Sui(p) => {
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let client = SuiClient::new(endpoints);
                let r = client
                    .sign_and_send(
                        &p.from,
                        &p.to,
                        p.mist,
                        p.gas_budget.unwrap_or(10_000_000),
                        &priv_arr,
                        &pub_arr,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Aptos(p) => {
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let client = AptosClient::new(endpoints);
                let r = client
                    .sign_and_submit(&p.from, &p.to, p.octas, &priv_arr, &pub_arr)
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Near(p) => {
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let client = NearClient::new(endpoints);
                let r = client
                    .sign_and_broadcast(&p.from, &p.to, p.yocto_near, &priv_arr, &pub_arr)
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Stellar(p) => {
                // Accept 32-byte seed (raw import) or 64-byte expanded key (derived).
                let priv_raw = decode_private_key(&p.private_key_hex)?;
                let priv_arr: [u8; 64] = if priv_raw.len() == 32 {
                    let mut expanded = [0u8; 64];
                    expanded[..32].copy_from_slice(&priv_raw);
                    expanded
                } else {
                    priv_raw
                        .try_into()
                        .map_err(|_| "privkey must be 32 or 64 bytes")?
                };
                // public_key_hex is optional: derive ed25519 verifying key when absent.
                let pub_arr: [u8; 32] = match p.public_key_hex.as_deref().filter(|s| !s.is_empty())
                {
                    Some(s) => hex::decode(s)
                        .map_err(|e| format!("pubkey hex: {e}"))?
                        .try_into()
                        .map_err(|_| "pubkey wrong length")?,
                    None => {
                        use ed25519_dalek::SigningKey;
                        let seed: [u8; 32] = priv_arr[..32]
                            .try_into()
                            .map_err(|_| "privkey seed too short")?;
                        SigningKey::from_bytes(&seed).verifying_key().to_bytes()
                    }
                };
                let network_passphrase = p
                    .network_passphrase
                    .as_deref()
                    .map(|s| s.as_bytes().to_vec());
                let client = StellarClient::new(endpoints);
                let r = client
                    .sign_and_submit(
                        &p.from,
                        &p.to,
                        p.stroops,
                        &priv_arr,
                        &pub_arr,
                        network_passphrase,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Cardano(p) => {
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let api_key = self.api_key_for(chain.str_id()).await.unwrap_or_default();
                let client = CardanoClient::new(endpoints, api_key);
                let r = client
                    .sign_and_broadcast(
                        &p.from,
                        &p.to,
                        p.amount_lovelace,
                        fee_or_static(chain, p.fee_lovelace),
                        &priv_arr,
                        &pub_arr,
                        p.ttl_slots,
                        p.min_change_lovelace,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Polkadot(p) => {
                let priv_arr: [u8; 32] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let subscan = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = PolkadotClient::new(endpoints, subscan, api_key);
                let r = client
                    .sign_and_submit(&p.from, &p.to, p.planck, &priv_arr, &pub_arr, p.era, p.tip)
                    .await?;
                json_response(&r)
            }
            SendParams::Bittensor(p) => {
                let priv_arr: [u8; 32] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let taostats = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = BittensorClient::new(endpoints, taostats, api_key);
                let r = client
                    .sign_and_submit(&p.from, &p.to, p.rao, &priv_arr, &pub_arr)
                    .await?;
                json_response(&r)
            }
            SendParams::Ton(p) => {
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = TonClient::new(endpoints, api_key);
                let seqno = client.fetch_seqno(&p.from).await?;
                let r = client
                    .sign_and_send(
                        &p.to,
                        p.nanotons,
                        seqno,
                        p.comment.as_deref(),
                        &priv_arr,
                        &pub_arr,
                        p.subwallet_id.map(|n| n as u32),
                        p.expiry_seconds.map(|n| n as u32),
                        p.send_mode.map(|n| n as u8),
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Icp(p) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                // public_key_hex is optional: derive compressed secp256k1 pubkey when absent.
                let derived_pub: Vec<u8>;
                let pub_bytes: &[u8] = match p.public_key_hex.as_deref().filter(|s| !s.is_empty()) {
                    Some(s) => {
                        derived_pub = hex::decode(s).map_err(|e| format!("pubkey hex: {e}"))?;
                        &derived_pub
                    }
                    None => {
                        use secp256k1::{PublicKey as SecpPubKey, Secp256k1, SecretKey};
                        let secp = Secp256k1::new();
                        let secret = SecretKey::from_slice(&priv_bytes)
                            .map_err(|e| format!("bad privkey: {e}"))?;
                        derived_pub = SecpPubKey::from_secret_key(&secp, &secret)
                            .serialize()
                            .to_vec();
                        &derived_pub
                    }
                };
                let client = IcpClient::new(endpoints);
                let r = client
                    .sign_and_submit(&p.from, &p.to, p.e8s, &priv_bytes, pub_bytes)
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Monero(p) => {
                // Probe and submit through the same endpoint: fallback to a
                // different wallet-rpc after the check could sign as another wallet.
                let mut unavailable =
                    SpectraBridgeError::from("no Monero wallet-rpc endpoint available");
                for endpoint in endpoints.iter() {
                    let client = MoneroClient::new(Arc::new(vec![endpoint.clone()]));
                    let address = match client.fetch_address(0).await {
                        Ok(address) => address,
                        Err(error) => {
                            unavailable = error.into();
                            continue;
                        }
                    };
                    if address != p.from {
                        return Err(SpectraBridgeError::InvalidInput {
                            message: "Monero wallet-rpc sender does not match the selected wallet"
                                .into(),
                        });
                    }
                    let result = client
                        .send(&p.to, p.piconeros, 0, p.priority.unwrap_or(2) as u32)
                        .await?;
                    return Ok(serde_json::to_string(&result)?);
                }
                Err(unavailable)
            }
        }
    }

    /// The five chains that share one signing shape: amount, flat fee, dust
    /// threshold, private key. Which client answers follows from the chain,
    /// because `build_send_params` never wraps one chain's params in a
    /// `SendParams::Utxo` meant for another.
    async fn sign_and_broadcast_shared_utxo(
        &self,
        chain: Chain,
        p: crate::service::send_params::UtxoFixedFeeSendParams,
        endpoints: Arc<Vec<String>>,
    ) -> Result<String, SpectraBridgeError> {
        let key = decode_private_key(&p.private_key_hex)?;
        let fee = fee_or_static(chain, p.fee_sat);
        let dust = p.dust_threshold_sats;

        match chain {
            Chain::Dogecoin => {
                let client = DogecoinClient::new(endpoints);
                json_response(
                    &client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sat, fee, &key, dust)
                        .await?,
                )
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(endpoints);
                json_response(
                    &client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sat, fee, &key, dust)
                        .await?,
                )
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(endpoints);
                json_response(
                    &client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sat, fee, &key, dust)
                        .await?,
                )
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(endpoints);
                json_response(
                    &client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sat, fee, &key, dust)
                        .await?,
                )
            }
            Chain::BitcoinGold => {
                let client = BitcoinGoldClient::new(endpoints);
                json_response(
                    &client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sat, fee, &key, dust)
                        .await?,
                )
            }
            Chain::Dash => {
                let client = DashClient::new(endpoints);
                json_response(
                    &client
                        .sign_and_broadcast(&p.from, &p.to, p.amount_sat, fee, &key, dust)
                        .await?,
                )
            }
            c => Err(SpectraBridgeError::from(format!(
                "sign_and_broadcast_send: {c:?} does not use the shared UTXO shape"
            ))),
        }
    }

    /// Token transfers: the four chains whose tokens Spectra can send.
    async fn sign_and_broadcast_token(
        &self,
        chain: Chain,
        token: crate::service::send_params::SendTokenParams,
        endpoints: Arc<Vec<String>>,
    ) -> Result<String, SpectraBridgeError> {
        use crate::service::send_params::SendTokenParams;
        match token {
            SendTokenParams::Evm(p, overrides) => {
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = EvmClient::new(endpoints, chain.evm_chain_id());
                let r = client
                    .sign_and_broadcast_erc20_with_overrides(
                        &p.from,
                        &p.contract,
                        &p.to,
                        p.amount_raw,
                        &priv_bytes,
                        overrides,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendTokenParams::Tron(p) => {
                // Tron — TRC-20. Addresses are base58, amount is in token
                // units, `fee_limit_sun` defaults to 100 TRX
                // (100_000_000 sun), which covers typical USDT transfers
                // (roughly 13-25 TRX actual cost).
                let priv_bytes = decode_private_key(&p.private_key_hex)?;
                let client = TronClient::new(endpoints);
                let r = client
                    .sign_and_broadcast_trc20(
                        &p.from,
                        &p.contract,
                        &p.to,
                        p.amount_raw,
                        p.fee_limit_sun.unwrap_or(100_000_000),
                        &priv_bytes,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendTokenParams::Near(p) => {
                // NEAR — NEP-141 fungible token transfer (ft_transfer).
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let client = NearClient::new(endpoints);
                let r = client
                    .sign_and_broadcast_ft_transfer(
                        &p.from,
                        &p.contract,
                        &p.to,
                        p.amount_raw,
                        &priv_arr,
                        &pub_arr,
                        p.gas_tgas,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendTokenParams::Solana(p) => {
                // Solana — SPL token transfer with idempotent ATA create.
                let from_arr: [u8; 32] = decode_hex_array(&p.from_pubkey_hex, "from_pubkey_hex")?;
                let priv_arr: [u8; 64] = decode_hex_array(&p.private_key_hex, "private_key_hex")?;
                let client = SolanaClient::new(endpoints);
                let r = client
                    .sign_and_broadcast_spl(
                        &from_arr,
                        &p.to,
                        &p.mint,
                        p.amount_raw,
                        p.decimals,
                        &priv_arr,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
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

    pub(crate) async fn broadcast_raw(
        &self,
        chain_id: &str,
        payload: String,
    ) -> Result<String, SpectraBridgeError> {
        let chain = Chain::from_str_id(chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!("broadcast_raw: chain {chain_id} not supported"))
        })?;
        let eps = self.endpoints_for(chain.str_id()).await;
        match chain {
            Chain::Bitcoin => {
                let client = BitcoinClient::new(HttpClient::shared(), eps);
                let txid = client.broadcast_raw_tx(&payload).await?;
                Ok(json!({ "txid": txid }).to_string())
            }
            Chain::Dogecoin => {
                let client = DogecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Litecoin => {
                let client = LitecoinClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinCash => {
                let client = BitcoinCashClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::BitcoinSV => {
                let client = BitcoinSvClient::new(eps);
                let res = client.broadcast_raw_tx(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Solana => {
                let client = SolanaClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Tron => {
                let client = TronClient::new(eps);
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            c if c.is_evm() => {
                let client = EvmClient::new(eps, c.evm_chain_id());
                let res = client.broadcast_raw(&payload).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Xrp => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let blob = val["tx_blob_hex"]
                    .as_str()
                    .ok_or("broadcast_raw xrp: missing tx_blob_hex")?
                    .to_string();
                let client = XrpClient::new(eps);
                let res = client.submit_signed_blob(&blob).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Stellar => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let xdr = val["signed_xdr_b64"]
                    .as_str()
                    .ok_or("broadcast_raw stellar: missing signed_xdr_b64")?
                    .to_string();
                let client = StellarClient::new(eps);
                let res = client.submit_envelope_b64(&xdr).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Cardano => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let cbor = val["cbor_hex"]
                    .as_str()
                    .ok_or("broadcast_raw cardano: missing cbor_hex")?
                    .to_string();
                let api_key = self.api_key_for(chain.str_id()).await.unwrap_or_default();
                let client = CardanoClient::new(eps, api_key);
                let res = client.submit_tx(&cbor).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Polkadot => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let ext_hex = val["extrinsic_hex"]
                    .as_str()
                    .ok_or("broadcast_raw polkadot: missing extrinsic_hex")?
                    .to_string();
                let subscan = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = PolkadotClient::new(eps, subscan, api_key);
                let res = client.submit_extrinsic_hex(&ext_hex).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Sui => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_bytes = val["tx_bytes_b64"]
                    .as_str()
                    .ok_or("broadcast_raw sui: missing tx_bytes_b64")?
                    .to_string();
                let sig = val["sig_b64"]
                    .as_str()
                    .ok_or("broadcast_raw sui: missing sig_b64")?
                    .to_string();
                let client = SuiClient::new(eps);
                let res = client.execute_signed_tx(&tx_bytes, &sig).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Aptos => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let body_json = val["signed_body_json"]
                    .as_str()
                    .ok_or("broadcast_raw aptos: missing signed_body_json")?
                    .to_string();
                let client = AptosClient::new(eps);
                let res = client.submit_signed_body(&body_json).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Ton => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let boc = val["boc_b64"]
                    .as_str()
                    .ok_or("broadcast_raw ton: missing boc_b64")?
                    .to_string();
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = TonClient::new(eps, api_key);
                let res = client.send_boc(&boc).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Near => {
                let val: serde_json::Value = serde_json::from_str(&payload)?;
                let tx_b64 = val["signed_tx_b64"]
                    .as_str()
                    .ok_or("broadcast_raw near: missing signed_tx_b64")?
                    .to_string();
                let client = NearClient::new(eps);
                let res = client.broadcast_signed_tx_b64(&tx_b64).await?;
                Ok(serde_json::to_string(&res)?)
            }
            Chain::Icp => Err(SpectraBridgeError::from(
                "ICP rebroadcast is not supported".to_string(),
            )),
            c => Err(SpectraBridgeError::from(format!(
                "broadcast_raw: chain {c:?} not supported"
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
mod fee_estimates_are_typed {
    use crate::registry::Chain;
    use crate::service::WalletService;

    /// The static-fee chains quote the catalog's number, scaled by their own
    /// decimals. This went through a serialized `FeePreview` and a
    /// `serde_json::from_str` in the caller before; the numbers are the same
    /// ones, reached without the round trip. No network: `static_fee_units`
    /// is catalog data, so these arms never build a client.
    #[tokio::test]
    async fn a_static_fee_chain_quotes_the_catalog_scaled_by_its_decimals() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        // (chain, raw units, display)
        for (chain, raw, display) in [
            (Chain::Solana, "5000", "0.000005"),     // 9 decimals
            (Chain::Cardano, "170000", "0.17"),      // 6 decimals
            (Chain::Sui, "1000", "0.000001"),        // 9 decimals
            (Chain::Icp, "10000", "0.0001"),         // 8 decimals
            (Chain::Polkadot, "160000000", "0.016"), // 10 decimals
        ] {
            let fee = service.native_fee_estimate(chain).await.expect("fee");
            assert_eq!(fee.raw, raw, "{}", chain.chain_display_name());
            assert_eq!(fee.display, display, "{}", chain.chain_display_name());
            assert_eq!(fee.source, "static");
        }
    }

    /// NEAR's fee does not fit the `u128 -> display` path the others take —
    /// it is carried as the string it is, which is why it had its own arm
    /// before and still does.
    #[tokio::test]
    async fn near_carries_its_fee_as_a_string() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let fee = service.native_fee_estimate(Chain::Near).await.expect("fee");
        assert_eq!(fee.raw, "1000000000000000000000");
        assert_eq!(fee.display, "0.001");
        assert_eq!(fee.source, "static");
    }

    /// A chain with no fee to quote is an error naming it, where the JSON
    /// version returned `{"note": "fee estimation not supported…"}` that the
    /// caller then read zeros out of. Nothing routes such a chain here —
    /// `simple_preview_chain` covers eleven, all of which answer — so this is
    /// the guard, not a live path.
    #[tokio::test]
    async fn a_chain_with_no_fee_is_a_named_error() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let err = service
            .native_fee_estimate(Chain::Ethereum)
            .await
            .expect_err("EVM has its own preview path, not this one");
        assert!(format!("{err:?}").contains("Ethereum"));
    }
}

/// A wallet holding one asset, so a probe named by wallet and holding has
/// something to look up. `token` is `None` for the chain's own asset, and
/// otherwise the contract and precision of the row that vouches for it —
/// which goes into the token preferences beside the holding, because an
/// unvouched token is a refusal rather than a probe.
#[cfg(test)]
async fn seed_probe_holding(
    service: &WalletService,
    chain: Chain,
    symbol: &str,
    token: Option<(&str, u32)>,
) -> String {
    use crate::store::state::WalletSummary;
    use crate::store::wallet_domain::{
        AssetHolding, CoreTokenHostingChain, CoreTokenPreferenceCategory, CoreTokenPreferenceEntry,
    };
    let chain_name = chain.chain_display_name().to_string();
    let mut state = service.wallet_state.write().await;
    let mut wallet = WalletSummary::single_address(
        "probe-wallet",
        "Probe",
        chain_name.clone(),
        "sender",
        None,
        false,
    );
    wallet.holdings = vec![AssetHolding {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        coin_gecko_id: String::new(),
        chain_name: chain_name.clone(),
        token_standard: String::new(),
        contract_address: token.map(|(contract, _)| contract.to_string()),
        amount: 1.0,
        price_usd: 0.0,
    }];
    state.wallets.push(wallet);
    if let Some((contract, decimals)) = token {
        let hosting =
            CoreTokenHostingChain::from_chain_name(&chain_name).expect("the chain hosts tokens");
        state.token_preferences.push(CoreTokenPreferenceEntry {
            category: CoreTokenPreferenceCategory::Stablecoin,
            is_built_in: false,
            is_enabled: true,
            token: crate::tokens::TokenEntry {
                chain: hosting.chain_name().to_string(),
                name: symbol.to_string(),
                symbol: symbol.to_string(),
                token_standard: String::new(),
                contract: contract.to_string(),
                coingecko_id: String::new(),
                decimals,
                tags: Vec::new(),
                color: String::new(),
                asset_name: String::new(),
                enabled: true,
            },
        });
    }
    format!("{chain_name}|{symbol}")
}

#[cfg(test)]
mod a_destination_probe_refuses_before_it_guesses {
    use crate::service::WalletService;

    /// An asset core cannot identify is an error, not a verdict.
    ///
    /// The shape this replaces had a `default` arm that answered
    /// `(nil, nil)` — no warning — for anything it did not recognise, so a
    /// chain the front end could not resolve looked exactly like a
    /// destination that had passed the check. Silence is the wrong answer to
    /// "is this address safe to send to"; the caller has to know the question
    /// was not asked. The composer that named the token itself had the same
    /// hole from the other side: it cleared the probe and showed nothing when
    /// it could not identify one. No network: both refusals precede the reads.
    #[tokio::test]
    async fn an_unfindable_holding_is_an_error_and_not_a_clean_verdict() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let missing = service
            .send_destination_risk(
                "no-such-wallet".into(),
                "Bitcoin|BTC".into(),
                "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq".into(),
            )
            .await;
        assert!(
            missing.is_err(),
            "a holding core does not have must not answer with a verdict"
        );

        // The wallet is there and holds the asset, but nothing vouches for the
        // contract — so there is no balance to ask about, and saying so is the
        // answer.
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let key = super::seed_probe_holding(
            &service,
            crate::registry::Chain::Ethereum,
            "TOK",
            Some((&format!("0x{}", "22".repeat(20)), 6)),
        )
        .await;
        service.wallet_state.write().await.token_preferences.clear();
        let untracked = service
            .send_destination_risk("probe-wallet".into(), key, format!("0x{}", "33".repeat(20)))
            .await;
        let refusal = untracked
            .expect_err("an unvouched token has no balance to report")
            .to_string();
        assert!(
            refusal.contains("TOK") && refusal.contains("tracks"),
            "the refusal names the asset and why: {refusal}"
        );
    }
}

/// What a destination probe asks about, from the holding being sent.
///
/// `None` is the chain's own asset. A token the user does not track has no
/// descriptor and is a refusal rather than a fallback: the probe would
/// otherwise read the *chain's* balance and report it as the token's, which is
/// what three of Swift's four arms did, or read nothing and show no verdict,
/// which is what the EVM arm did. Neither says "we could not check".
fn destination_probe_asset(
    holding: &crate::store::wallet_domain::AssetHolding,
    preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> Result<(Chain, Option<TokenDescriptor>), SpectraBridgeError> {
    let chain = Chain::from_display_name(&holding.chain_name).ok_or_else(|| {
        SpectraBridgeError::InvalidInput {
            message: format!("unknown chain: {}", holding.chain_name),
        }
    })?;
    if holding.symbol == chain.coin_symbol() {
        return Ok((chain, None));
    }
    let identity =
        super::maintenance::send_token_identity(holding, preferences).ok_or_else(|| {
            SpectraBridgeError::InvalidInput {
                message: format!(
                    "{} on {} is not a token this wallet tracks",
                    holding.symbol, holding.chain_name
                ),
            }
        })?;
    // The catalog's precision, not a clamp of it. The caller that used to build
    // this descriptor wrote `UInt8(clamping:)`, which turns an impossible 300
    // into a plausible 255 and reads a balance off by 45 decimal places.
    let decimals =
        u8::try_from(identity.decimals).map_err(|_| SpectraBridgeError::InvalidInput {
            message: format!(
                "{} on {} declares {} decimals",
                holding.symbol, holding.chain_name, identity.decimals
            ),
        })?;
    Ok((
        chain,
        Some(TokenDescriptor {
            contract: identity.contract,
            symbol: holding.symbol.clone(),
            decimals,
            name: None,
        }),
    ))
}

#[cfg(test)]
mod destination_resolution_tests {
    use crate::service::WalletService;

    /// A valid address comes back in the chain's own form, and says no name
    /// was involved.
    ///
    /// The EVM branch this replaces lowercased through `normalizeEVMAddress`,
    /// which is the same answer here — but it was Swift's spelling of a
    /// registry rule, applied on EVM chains only. Offline: no branch that
    /// touches the network is reached.
    #[tokio::test]
    async fn a_valid_address_is_normalized_and_not_a_name() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let resolved = service
            .resolve_send_destination(
                "ethereum".into(),
                "  0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  ".into(),
            )
            .await
            .expect("a valid EVM address resolves to itself");
        assert_eq!(
            resolved.address,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(!resolved.used_ens);
    }

    /// Nothing typed is refused rather than resolved to the empty string.
    #[tokio::test]
    async fn an_empty_destination_is_refused() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let err = service
            .resolve_send_destination("bitcoin".into(), "   ".into())
            .await
            .expect_err("an empty destination is not an address");
        assert!(format!("{err:?}").contains("Bitcoin"));
    }

    /// A `.eth` name on a chain that does not run the registry is refused
    /// before any lookup, which is the stricter of the two readings and what
    /// `Chain::resolves_ens_names` now states once.
    ///
    /// Offline by construction: the refusal happens before the resolver is
    /// called, so a network-less test proves the branch and not the timeout.
    #[tokio::test]
    async fn a_name_is_not_looked_up_off_the_chain_that_registers_it() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for chain_id in ["arbitrum", "base", "polygon", "bitcoin"] {
            let err = service
                .resolve_send_destination(chain_id.into(), "vitalik.eth".into())
                .await
                .expect_err("a name off Ethereum is not a destination");
            assert!(
                format!("{err:?}").contains("valid"),
                "{chain_id} should refuse the name, got {err:?}"
            );
        }
    }

    /// An unknown chain is an error, not a destination.
    #[tokio::test]
    async fn an_unknown_chain_resolves_nothing() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        assert!(service
            .resolve_send_destination("not-a-chain".into(), "0xabc".into())
            .await
            .is_err());
    }

    /// A cached name answers without a lookup, in the normalized form.
    ///
    /// The cache is seeded directly because the lookup itself needs the ENS
    /// API; what this pins is that a hit reports `used_ens` and does not fall
    /// through to the network, which a service with no endpoints would fail.
    #[tokio::test]
    async fn a_cached_name_answers_without_the_network() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        service.ens_resolutions.write().await.insert(
            "vitalik.eth".into(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        );
        let resolved = service
            .resolve_send_destination("ethereum".into(), "  Vitalik.ETH ".into())
            .await
            .expect("a cached name resolves");
        assert_eq!(
            resolved.address,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(resolved.used_ens, "a name lookup must say it was one");
    }
}

#[cfg(test)]
mod failed_reads {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

    /// The send path is the caller that must not read an absent balance as a
    /// zero. `fetch_token_balances` leaves out a token it could not read so a
    /// refresh keeps the last known amount on screen; here that same absence
    /// has to be a refusal, because everything a send decides — whether this
    /// is the whole balance, whether the amount fits — is computed from it.
    #[tokio::test]
    async fn an_unreadable_token_balance_is_not_an_empty_wallet() {
        let server = MockServer::start().await;
        // Only the token read fails. The balance and the history run
        // concurrently and `try_join!` reports whichever errors first, so a
        // mock that failed both would be asserting on which future lost a
        // race — and did, intermittently, under a loaded test run.
        Mock::given(any())
            .respond_with(|req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let is_token_read = body["method"] == "eth_call";
                ResponseTemplate::new(200).set_body_json(if is_token_read {
                    json!({
                        "jsonrpc": "2.0", "id": body["id"],
                        "error": {"code": -32000, "message": "no code at address"},
                    })
                } else {
                    json!({"jsonrpc": "2.0", "id": body["id"], "result": "0x1"})
                })
            })
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let contract = format!("0x{}", "22".repeat(20));
        let key =
            seed_probe_holding(&service, Chain::Ethereum, "TEST", Some((&contract, 18))).await;
        let risk = service
            .send_destination_risk("probe-wallet".into(), key, format!("0x{}", "33".repeat(20)))
            .await;
        assert!(
            risk.unwrap_err().to_string().contains("unavailable"),
            "an unread balance must not reach a send as zero"
        );
    }

    #[tokio::test]
    async fn evm_preview_requires_every_rpc_and_valid_amount() {
        for failed in [
            "",
            "eth_getTransactionCount",
            "eth_getBalance",
            "eth_estimateGas",
            "eth_feeHistory",
            "reward",
        ] {
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let method = body["method"].as_str().unwrap();
                let result = match method {
                    "eth_getTransactionCount" => json!("0x7"),
                    "eth_getBalance" => json!("0xde0b6b3a7640000"),
                    "eth_estimateGas" => json!("0x7530"),
                    "eth_feeHistory" if failed == "reward" => json!({"baseFeePerGas":["0x1"]}),
                    "eth_feeHistory" => json!({"baseFeePerGas":["0x1"],"reward":[["0x2"]]}),
                    _ => panic!("unexpected method {method}"),
                };
                let response = if failed == method {
                    json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":"unavailable"}})
                } else { json!({"jsonrpc":"2.0","id":body["id"],"result":result}) };
                ResponseTemplate::new(200).set_body_json(response)
            }).mount(&server).await;
            let service = WalletService::new_typed(vec![ChainEndpoints {
                chain_id: "ethereum".into(),
                endpoints: vec![server.uri()],
                api_key: None,
            }])
            .unwrap();
            let preview = service
                .fetch_evm_send_preview(
                    "ethereum",
                    "from".into(),
                    "to".into(),
                    "1".into(),
                    "0x".into(),
                )
                .await;
            if failed.is_empty() {
                let value: serde_json::Value = serde_json::from_str(&preview.unwrap()).unwrap();
                assert_eq!(value["nonce"], 7);
                assert_eq!(value["gas_limit"], 30000);
            } else {
                assert!(preview.is_err(), "{failed}");
            }
            let before = server.received_requests().await.unwrap().len();
            for value in ["bad", "-1", "+1", "340282366920938463463374607431768211456"] {
                assert!(service
                    .fetch_evm_send_preview(
                        "ethereum",
                        "from".into(),
                        "to".into(),
                        value.into(),
                        "0x".into()
                    )
                    .await
                    .is_err());
            }
            assert_eq!(server.received_requests().await.unwrap().len(), before);
        }
    }

    #[tokio::test]
    async fn trc20_zero_is_valid_but_failed_metadata_is_not_a_zero_balance() {
        use wiremock::matchers::body_partial_json;
        for valid in [true, false] {
            let server = MockServer::start().await;
            for (selector, result) in [
                ("balanceOf(address)", "0".repeat(64)),
                ("decimals()", format!("{:064x}", 6)),
                ("symbol()", format!("{:0<64}", hex::encode("TEST"))),
            ] {
                let response = if !valid && selector == "symbol()" {
                    json!({})
                } else {
                    json!({"constant_result":[result]})
                };
                Mock::given(body_partial_json(json!({"function_selector":selector})))
                    .respond_with(ResponseTemplate::new(200).set_body_json(response))
                    .mount(&server)
                    .await;
            }
            let service = WalletService::new_typed(vec![ChainEndpoints {
                chain_id: "tron".into(),
                endpoints: vec![server.uri()],
                api_key: None,
            }])
            .unwrap();
            let result = service
                .fetch_token_balances(
                    "tron".into(),
                    "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7".into(),
                    vec![TokenDescriptor {
                        contract: "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t".into(),
                        symbol: "TEST".into(),
                        decimals: 18,
                        name: None,
                    }],
                )
                .await;
            let rows = result.expect("one token's failure is not the request's");
            if valid {
                // A contract that answers zero holds zero.
                assert_eq!(rows[0].balance_raw, "0");
                assert_eq!(rows[0].decimals, 6);
            } else {
                // A contract that does not answer is left out. It must never
                // arrive as a zero: `send_destination_risk` reads this row and
                // would take the absence for an empty wallet.
                assert!(rows.is_empty(), "{rows:?}");
            }
        }
    }
}

/// A send preview's "spendable" is a fact about the asset the amount field
/// moves, not about whatever the chain pays gas in. Both previews here used to
/// answer with the gas coin's own arithmetic whatever was being sent.
#[cfg(test)]
mod a_preview_quotes_the_asset_it_moves {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};

    /// One ETH held, and 250 of an 8-decimal token.
    const ETH_BALANCE_WEI: &str = "0xde0b6b3a7640000";
    const TOKEN_DECIMALS: u128 = 8;
    const TOKEN_RAW: u128 = 25_000_000_000; // 250.0 at 8 decimals
    const TOKEN_DISPLAY: f64 = 250.0;

    /// Answer one JSON-RPC call. `eth_call` dispatches on the ABI selector so
    /// the token contract can hold a balance the account does not.
    fn rpc_result(method: &str, params: &serde_json::Value) -> serde_json::Value {
        match method {
            "eth_getTransactionCount" => json!("0x7"),
            "eth_getBalance" => json!(ETH_BALANCE_WEI),
            "eth_estimateGas" => json!("0x7530"),
            "eth_feeHistory" => json!({"baseFeePerGas": ["0x1"], "reward": [["0x2"]]}),
            "eth_call" => {
                let data = params[0]["data"].as_str().unwrap_or_default();
                let selector = data.trim_start_matches("0x").get(..8).unwrap_or_default();
                match selector {
                    "70a08231" => json!(format!("0x{TOKEN_RAW:064x}")),
                    "313ce567" => json!(format!("0x{TOKEN_DECIMALS:064x}")),
                    "95d89b41" => json!(format!("0x{:0<64}", hex::encode("TEST"))),
                    other => panic!("unexpected eth_call selector {other}"),
                }
            }
            other => panic!("unexpected method {other}"),
        }
    }

    /// A node that answers single calls and JSON-RPC batches alike —
    /// `fetch_erc20_metadata` batches its two reads and `balanceOf` does not.
    async fn evm_node() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let answer = |call: &serde_json::Value| {
                    json!({
                        "jsonrpc": "2.0",
                        "id": call["id"],
                        "result": rpc_result(call["method"].as_str().unwrap(), &call["params"]),
                    })
                };
                ResponseTemplate::new(200).set_body_json(match body.as_array() {
                    Some(batch) => json!(batch.iter().map(answer).collect::<Vec<_>>()),
                    None => answer(&body),
                })
            })
            .mount(&server)
            .await;
        server
    }

    /// `value_wei` and `data_hex` are what `prepare_evm_send_assembly` hands
    /// this call for the send in question — a token transfer carries its
    /// amount in the calldata and moves no ether, so its value is zero.
    async fn preview(
        server: &MockServer,
        to: String,
        value_wei: &str,
        data_hex: String,
    ) -> serde_json::Value {
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let raw = service
            .fetch_evm_send_preview(
                "ethereum",
                format!("0x{}", "11".repeat(20)),
                to,
                value_wei.into(),
                data_hex,
            )
            .await
            .expect("preview");
        serde_json::from_str(&raw).unwrap()
    }

    /// An ERC-20 send moves the token, so the token's balance — scaled by the
    /// contract's own decimals — is what is spendable. This answered with the
    /// sender's ETH balance, which the send sheet then rendered through the
    /// token's formatter: 1 ETH shown as "1 USDC", and "Max" filling in a
    /// number the token transfer could not move.
    #[tokio::test]
    async fn an_erc20_send_is_limited_by_the_token_and_not_by_the_ether() {
        let server = evm_node().await;
        let contract = format!("0x{}", "22".repeat(20));
        // transfer(0x33…, 1)
        let data = format!("0xa9059cbb{:0>64}{:0>64}", "33".repeat(20), "1");
        let value = preview(&server, contract, "0", data).await;

        assert_eq!(value["spendable_balance"], json!(TOKEN_DISPLAY));
        assert_ne!(
            value["spendable_balance"].as_f64().unwrap().round(),
            1.0,
            "the gas coin's balance is not the token's"
        );
        // The fee is still quoted in the gas coin: it is a separate claim.
        assert!(value["estimated_fee_eth"].as_f64().unwrap() > 0.0);
    }

    /// A native send pays its fee out of the balance it is moving, so the fee
    /// still comes off the top. Unchanged — asserted here so the token arm
    /// cannot be made to swallow this one.
    #[tokio::test]
    async fn a_native_send_still_nets_the_fee_off_its_own_balance() {
        let server = evm_node().await;
        let value = preview(
            &server,
            format!("0x{}", "33".repeat(20)),
            "1000000000000000",
            "0x".into(),
        )
        .await;

        let fee = value["estimated_fee_eth"].as_f64().unwrap();
        assert!(fee > 0.0);
        assert_eq!(value["spendable_balance"].as_f64().unwrap(), 1.0 - fee);
    }

    /// TRC-20 decimals are the contract's. The fixed `1e6` that stood here is
    /// TRX's own scale, so an 18-decimal token was quoted at 10^12 times the
    /// holding it is — and "Max" offered it.
    #[tokio::test]
    async fn a_trc20_preview_scales_by_the_contract_and_not_by_trx() {
        use wiremock::matchers::body_partial_json;

        let server = MockServer::start().await;
        // 4.2 of an 18-decimal token.
        let raw: u128 = 4_200_000_000_000_000_000;
        for (selector, result) in [
            ("balanceOf(address)", format!("{raw:064x}")),
            ("decimals()", format!("{:064x}", 18)),
            ("symbol()", format!("{:0<64}", hex::encode("TEST"))),
        ] {
            Mock::given(body_partial_json(json!({"function_selector": selector})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"constant_result": [result]})),
                )
                .mount(&server)
                .await;
        }
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "tron".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let raw_json = service
            .fetch_tron_send_preview(
                "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7".into(),
                "TEST".into(),
                "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t".into(),
            )
            .await
            .expect("preview");
        let value: serde_json::Value = serde_json::from_str(&raw_json).unwrap();

        assert_eq!(value["spendable_balance"], json!(4.2));
        assert_eq!(value["max_sendable"], json!(4.2));
    }

    /// A token balance nobody could read is not a zero holding — the send
    /// sheet decides whether the amount fits from this number.
    #[tokio::test]
    async fn an_unreadable_trc20_balance_refuses_rather_than_quoting_zero() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "tron".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let result = service
            .fetch_tron_send_preview(
                "TLa2f6VPqDgRE67v1736s7bJ8Ray5wYjU7".into(),
                "TEST".into(),
                "TR7NHqjeKQxGTCi8q8ZY4pL8otgjLj6t".into(),
            )
            .await;
        assert!(
            result.is_err(),
            "an unread balance must not quote a maximum"
        );
    }
}

#[cfg(test)]
mod destination_probe_tests {
    use super::*;
    use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
    #[tokio::test]
    async fn evm_activity_uses_one_balance_read_and_never_guesses_after_failure() {
        for nonce in [Some("0x1"), Some("0x0"), None] {
            let server = MockServer::start().await;
            Mock::given(any()).respond_with(move |request: &Request| {
                let body: serde_json::Value = request.body_json().unwrap();
                let method = body["method"].as_str().unwrap();
                let value = match method {
                    "eth_getBalance" => Some("0x0"),
                    "eth_getTransactionCount" => nonce,
                    _ => panic!("unexpected RPC {method}")
                };
                ResponseTemplate::new(200).set_body_json(match value {
                    Some(value) => json!({"jsonrpc":"2.0","id":body["id"],"result":value}),
                    None => json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":"offline"}})
                })
            }).mount(&server).await;
            // Zero nonce on BNB needs its keyed explorer: without a key the
            // result is unknown/error, rather than an invented empty history.
            let service = WalletService::new_typed(vec![ChainEndpoints {
                chain_id: Chain::BnbChain.str_id().into(),
                endpoints: vec![server.uri()],
                api_key: None,
            }])
            .unwrap();
            let key = seed_probe_holding(&service, Chain::BnbChain, "BNB", None).await;
            let result = service
                .send_destination_risk("probe-wallet".into(), key, format!("0x{}", "44".repeat(20)))
                .await;
            if nonce == Some("0x1") {
                let risk = result.unwrap();
                assert!(risk.has_history);
                assert!(risk.balance_is_zero);
            } else {
                assert!(result.is_err());
            }
            let requests = server.received_requests().await.unwrap();
            let balances = requests
                .iter()
                .filter(|r| {
                    r.body_json::<serde_json::Value>().unwrap()["method"] == "eth_getBalance"
                })
                .count();
            assert!(balances <= 1, "duplicate balance request");
            if nonce == Some("0x1") {
                assert_eq!(balances, 1);
                assert_eq!(requests.len(), 2);
            }
        }
    }
    #[tokio::test]
    async fn successful_empty_history_is_distinct_from_unknown() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(|request: &Request| {
                let is_history = request.url.query().unwrap_or("").contains("details=txs");
                ResponseTemplate::new(200).set_body_json(if is_history {
                    json!({"transactions":[]})
                } else {
                    json!({"balance":"0"})
                })
            })
            .mount(&server)
            .await;
        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "litecoin".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let key = seed_probe_holding(&service, Chain::Litecoin, "LTC", None).await;
        let risk = service
            .send_destination_risk(
                "probe-wallet".into(),
                key,
                "ltc1qw508d6qejxtdg4y5r3zarvary0c5xw7kgmn4n9".into(),
            )
            .await
            .unwrap();
        assert!(!risk.has_history);
        assert!(risk.balance_is_zero);
    }
}
