//! Dispatch prepared send inputs to protocol signing and submission.
use super::*;
impl WalletService {
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
    pub(super) async fn sign_and_broadcast_native(
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
                let priv_arr = crate::send::keys::Ed25519Seed::from_hex(&p.private_key_hex)?;
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
                let priv_arr = crate::send::keys::Ed25519Seed::from_hex(&p.private_key_hex)?;
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
                let priv_arr = crate::send::keys::Ed25519Seed::from_hex(&p.private_key_hex)?;
                let pub_arr: [u8; 32] = decode_hex_array(&p.public_key_hex, "public_key_hex")?;
                let client = AptosClient::new(endpoints);
                let r = client
                    .sign_and_submit(
                        &p.from,
                        &p.to,
                        p.octas,
                        &priv_arr,
                        &pub_arr,
                        chain.aptos_chain_id().ok_or("not an Aptos network")?,
                    )
                    .await?;
                Ok(serde_json::to_string(&r)?)
            }
            SendParams::Near(p) => {
                let priv_arr = decode_secret_array::<32>(&p.private_key_hex)?;
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
                let priv_arr = zeroize::Zeroizing::new(if priv_raw.len() == 32 {
                    let mut expanded = [0u8; 64];
                    expanded[..32].copy_from_slice(&priv_raw);
                    expanded
                } else {
                    priv_raw
                        .as_slice()
                        .try_into()
                        .map_err(|_| "privkey must be 32 or 64 bytes")?
                });
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
                let priv_arr = decode_secret_array::<64>(&p.private_key_hex)?;
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
                let priv_arr = decode_secret_array::<32>(&p.private_key_hex)?;
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
                let priv_arr = decode_secret_array::<32>(&p.private_key_hex)?;
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
                let priv_arr = decode_secret_array::<32>(&p.private_key_hex)?;
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
    pub(super) async fn sign_and_broadcast_shared_utxo(
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
    pub(super) async fn sign_and_broadcast_token(
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
                let priv_arr = decode_secret_array::<32>(&p.private_key_hex)?;
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
                let priv_arr = crate::send::keys::Ed25519Seed::from_hex(&p.private_key_hex)?;
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
}
