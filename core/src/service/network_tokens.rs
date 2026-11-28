//! Token discovery and balances, including partial provider failures.

use super::*;
use crate::api::endpoint::EndpointApi;
use crate::api::error::{ApiError, OrDecode};

/// Keep the tokens that answered; leave out the ones that did not.
///
/// A token read has three outcomes and they are not interchangeable: a
/// balance, a legitimate zero, and "the chain did not say". Reporting the
/// third as zero would be a claim about funds — a max-send computes from it
/// and a user reads it as "gone". Failing the whole batch would let one
/// self-destructed contract stop every other token from updating.
///
/// A token missing from this list is not updated by the caller, which
/// leaves its last known balance in place — the one answer that claims
/// nothing.
fn readable_tokens<E: std::fmt::Display>(
    results: Vec<Result<TokenBalanceResult, E>>,
) -> Vec<TokenBalanceResult> {
    results
        .into_iter()
        .filter_map(|result| match result {
            Ok(balance) => Some(balance),
            Err(error) => {
                tracing::warn!(%error, "token balance unavailable; leaving it unchanged");
                None
            }
        })
        .collect()
}

/// Whether a token metadata read may reuse a recent answer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetadataRead {
    /// Asked of the chain now: a send scales its amount by the answer.
    Fresh,
    /// May reuse what a balance read learned in the last few minutes, where
    /// the protocol's client keeps such a cache (TRC-20).
    Shared,
}

fn validate_token_reads(
    chain: Chain,
    tokens: &mut [TokenDescriptor],
) -> Result<(), SpectraBridgeError> {
    for token in tokens {
        let standard =
            crate::catalog::tokens::resolve_token_standard(chain, token.standard, &token.contract)?;
        token.contract =
            crate::catalog::tokens::validate_protocol_identifier(chain, standard, &token.contract)?;
        token.standard = Some(standard);
        if !chain.allows_token_standard(standard) {
            return Err(SpectraBridgeError::invalid(format!(
                "{standard} balance reads are not supported"
            )));
        }
    }
    Ok(())
}

/// An Asset Hub asset's id, as its validated identifier writes it.
pub(crate) fn asset_id(contract: &str) -> Result<u32, SpectraBridgeError> {
    contract
        .parse()
        .map_err(|_| SpectraBridgeError::invalid("invalid Asset Hub asset ID"))
}

/// An EVM listing without the coin's own ERC-20 face: its balance is the
/// native one the coin's row already shows.
fn without_native_coin_face(
    chain: Chain,
    held: Vec<crate::api::HeldToken>,
) -> Vec<crate::api::HeldToken> {
    held.into_iter()
        .filter(|holding| {
            chain
                .evm_native_coin_contract()
                .is_none_or(|native| !holding.contract.eq_ignore_ascii_case(native))
        })
        .collect()
}

/// The same canonical identity used by catalog, custom tokens and storage.
fn holding_key(chain: Chain, contract: &str) -> Option<String> {
    crate::catalog::tokens::normalize_token_identifier(Some(contract.to_string()), chain)
}

impl WalletService {
    /// The URLs of an indexer `api` that lists holdings on `chain`.
    pub(super) async fn listing_endpoints(
        &self,
        chain: Chain,
        api: EndpointApi,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let required: &[crate::api::endpoint::EndpointCapability] =
            if api == EndpointApi::ToncenterV3 {
                &[
                    crate::api::endpoint::EndpointCapability::TokenDiscovery,
                    crate::api::endpoint::EndpointCapability::TokenBalance,
                    crate::api::endpoint::EndpointCapability::Verification,
                ]
            } else {
                &[crate::api::endpoint::EndpointCapability::TokenDiscovery]
            };
        self.api_endpoints(chain, api, required).await
    }

    /// Every token `address` holds on `chain`, from one listing, or `None`
    /// when no configured service can list it.
    ///
    /// A token contract only answers about a holder you name, so a list needs
    /// a node that indexes by owner (Solana, Sui) or an indexer (TronGrid,
    /// TON Center v3, Nearblocks, Aptos Indexer, Blockscout).
    pub(crate) async fn held_tokens(
        &self,
        chain: Chain,
        address: &str,
    ) -> Result<Option<Vec<crate::api::HeldToken>>, SpectraBridgeError> {
        if !chain.lists_held_tokens() {
            return Ok(None);
        }
        let discovery = [crate::api::endpoint::EndpointCapability::TokenDiscovery];
        // Choose the service by family; every read keeps the concrete
        // network's endpoints and identity.
        let held = match chain.mainnet_counterpart() {
            Chain::Solana => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                SolanaClient::new(endpoints)
                    .fetch_all_spl_balances(address)
                    .await?
                    .into_iter()
                    .map(|b| {
                        Ok(crate::api::HeldToken {
                            balance_raw: b.balance_raw,
                            contract: b.mint,
                            decimals: Some(b.decimals),
                        })
                    })
                    .collect::<Result<_, crate::api::error::ApiError>>()?
            }
            Chain::Sui => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                SuiClient::new(endpoints)
                    .fetch_all_coin_balances(address)
                    .await?
            }
            Chain::Aptos => {
                let endpoints = self
                    .listing_endpoints(chain, EndpointApi::AptosIndexer)
                    .await?;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                let expected = chain.aptos_chain_id()?;
                crate::api::aptos_indexer::AptosIndexerClient::new(Arc::new(endpoints), expected)
                    .fetch_holdings(address)
                    .await?
            }
            Chain::Near => {
                let endpoints = self
                    .listing_endpoints(chain, EndpointApi::Nearblocks)
                    .await?;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                crate::api::nearblocks::NearblocksClient::new(Arc::new(endpoints))
                    .fetch_ft_holdings(address)
                    .await?
            }
            Chain::Tron => {
                let accounts = self
                    .api_endpoints(chain, EndpointApi::TrongridV1, &discovery)
                    .await?;
                let nodes = self
                    .endpoints_for(
                        chain,
                        &[
                            crate::api::endpoint::EndpointCapability::TokenBalance,
                            crate::api::endpoint::EndpointCapability::Verification,
                        ],
                    )
                    .await;
                if nodes.is_empty() || accounts.is_empty() {
                    return Ok(None);
                }
                let trc10 = TronHttpClient::new(nodes)
                    .fetch_trc10_holdings(chain, address)
                    .await?;
                let mut held: Vec<_> =
                    crate::api::trongrid_v1::TrongridClient::new(Arc::new(accounts))
                        .fetch_trc20_holdings(address)
                        .await?
                        .into_iter()
                        .map(|(contract, balance_raw)| crate::api::HeldToken {
                            contract,
                            balance_raw,
                            decimals: None,
                        })
                        .collect();
                held.extend(trc10);
                held
            }
            Chain::Ton => {
                let v3 = self
                    .listing_endpoints(chain, EndpointApi::ToncenterV3)
                    .await?;
                if v3.is_empty() {
                    return Ok(None);
                }
                crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(v3))
                    .fetch_jetton_balances(chain, address)
                    .await?
            }
            // A node lists every trust line of the account; a negative
            // balance is one it owes, not one it holds.
            Chain::Xrp => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                let places = crate::registry::TokenStandard::TrustLineToken
                    .fixed_decimals()
                    .expect("issued currencies are kept at fixed places");
                crate::api::xrpl_json_rpc::XrplClient::new(endpoints)
                    .fetch_trust_lines(address, None)
                    .await?
                    .into_iter()
                    .filter_map(|line| {
                        Some(crate::api::HeldToken {
                            contract: line.issue.identifier(),
                            balance_raw: line.balance.to_units(places)?,
                            decimals: Some(places as u8),
                        })
                    })
                    .collect()
            }
            // Every unspent output names the assets it holds; their sum is
            // what the address holds, at decimals read in one request.
            Chain::Cardano => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                let client = crate::api::koios::KoiosClient::new(endpoints);
                let mut held: std::collections::BTreeMap<
                    crate::registry::cardano_asset::CardanoAssetId,
                    u128,
                > = Default::default();
                for utxo in client.fetch_utxos(address).await? {
                    for asset in &utxo.assets {
                        let sum = held.entry(asset.id()?).or_default();
                        *sum = sum
                            .checked_add(u128::from(asset.amount()?))
                            .ok_or_else(|| SpectraBridgeError::failure("Cardano asset overflow"))?;
                    }
                }
                let decimals = client
                    .fetch_asset_decimals(&held.keys().cloned().collect::<Vec<_>>())
                    .await?;
                held.into_iter()
                    .map(|(asset, balance_raw)| crate::api::HeldToken {
                        contract: asset.identifier(),
                        balance_raw,
                        decimals: decimals.get(&asset).copied(),
                    })
                    .collect()
            }
            // An account's balances name every trustline it has.
            Chain::Stellar => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                crate::api::horizon::HorizonClient::new(endpoints)
                    .fetch_account_state(address)
                    .await?
                    .map(|state| state.trustlines)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|line| {
                        Some(crate::api::HeldToken {
                            contract: line.asset.identifier(),
                            balance_raw: u128::try_from(line.balance).ok()?,
                            decimals: Some(7),
                        })
                    })
                    .collect()
            }
            _ if chain.is_evm() => {
                let explorers = self
                    .listing_endpoints(chain, EndpointApi::Blockscout)
                    .await?;
                if explorers.is_empty() {
                    return Ok(None);
                }
                let client = crate::api::blockscout::BlockscoutClient::new();
                without_native_coin_face(
                    chain,
                    crate::api::http::race(&explorers, |base| {
                        let client = &client;
                        async move { client.fetch_token_holdings(address, &base).await }
                    })
                    .await?,
                )
            }
            _ => return Ok(None),
        };
        Ok(Some(held))
    }

    /// A token's decimals as its contract states them: the one reader of
    /// that fact, for balances and sends alike. A protocol that fixes every
    /// token's places answers without a request; unreadable metadata is an
    /// error, never a guess.
    pub(crate) async fn token_decimals(
        &self,
        chain: Chain,
        contract: &str,
        read: MetadataRead,
    ) -> Result<u8, SpectraBridgeError> {
        let standard = crate::catalog::tokens::resolve_token_standard(chain, None, contract)?;
        crate::catalog::tokens::validate_protocol_identifier(chain, standard, contract)?;
        if !chain.allows_token_standard(standard) {
            return Err(SpectraBridgeError::invalid(format!(
                "{standard} metadata reads are not supported"
            )));
        }
        if let Some(places) = standard.fixed_decimals() {
            return Ok(crate::api::checked_token_decimals(u128::from(places))?);
        }
        let endpoints = self
            .endpoints_for(
                chain,
                &[crate::api::endpoint::EndpointCapability::TokenBalance],
            )
            .await;
        let decimals = match chain.mainnet_counterpart() {
            _ if chain.is_evm() => {
                EvmClient::new(endpoints, chain.evm_chain_id()?)
                    .fetch_erc20_metadata(contract)
                    .await?
                    .decimals
            }
            Chain::Tron => {
                let client = match read {
                    MetadataRead::Fresh => TronHttpClient::new(endpoints),
                    MetadataRead::Shared => TronHttpClient::with_metadata_cache(
                        endpoints,
                        chain,
                        self.trc20_metadata.clone(),
                    ),
                };
                if standard == crate::registry::TokenStandard::Trc10 {
                    client.fetch_trc10_metadata(chain, contract).await?.decimals
                } else {
                    client.read_metadata(contract).await?.decimals
                }
            }
            Chain::Solana => {
                SolanaClient::new(endpoints)
                    .fetch_transfer_mint(contract)
                    .await?
                    .decimals
            }
            Chain::Near => {
                NearClient::new(endpoints)
                    .fetch_ft_metadata(contract)
                    .await?
                    .decimals
            }
            Chain::Cardano => {
                let asset = crate::registry::cardano_asset::CardanoAssetId::parse(contract)?;
                KoiosClient::new(endpoints)
                    .fetch_asset_decimals(std::slice::from_ref(&asset))
                    .await?
                    .get(&asset)
                    .copied()
                    .or_decode("asset decimals unavailable")?
            }
            Chain::Sui => SuiClient::new(endpoints)
                .fetch_coin_decimals(contract)
                .await
                .or_decode("coin decimals unavailable")?,
            Chain::Aptos => AptosClient::new(endpoints)
                .fetch_token_decimals(contract)
                .await
                .or_decode("token decimals unavailable")?,
            Chain::Starknet => {
                crate::api::starknet_json_rpc::StarknetClient::new(endpoints)
                    .fetch_decimals(&crate::derivation::starknet::decode_address(
                        chain, contract,
                    )?)
                    .await?
            }
            Chain::Polkadot => {
                SubstrateClient::new(endpoints)
                    .fetch_asset_decimals(chain, asset_id(contract)?)
                    .await?
            }
            Chain::Vechain => {
                crate::api::thor_rest::ThorClient::new(endpoints)
                    .fetch_token_decimals(contract)
                    .await?
            }
            Chain::Ton => crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(
                self.api_endpoints(
                    chain,
                    EndpointApi::ToncenterV3,
                    &[
                        crate::api::endpoint::EndpointCapability::Verification,
                        crate::api::endpoint::EndpointCapability::TokenBalance,
                    ],
                )
                .await?,
            ))
            .fetch_jetton_decimals(chain, contract)
            .await
            .or_decode("jetton decimals unavailable")?,
            _ => {
                return Err(SpectraBridgeError::failure(format!(
                    "{} has no token metadata reader",
                    chain.str_id()
                )));
            }
        };
        Ok(crate::api::checked_token_decimals(u128::from(decimals))?)
    }

    /// Balances of `tokens` read off one listing of what the address holds.
    ///
    /// A listing is the whole of what the address holds, so a token missing
    /// from it holds none: that zero is the listing's answer, not a guess.
    /// A token the listing names without decimals reads them from its
    /// contract, and one whose decimals cannot be read is left out.
    async fn balances_from_holdings(
        &self,
        chain: Chain,
        held: Vec<crate::api::HeldToken>,
        tokens: &[TokenDescriptor],
    ) -> Vec<TokenBalanceResult> {
        let mut tokens = tokens.to_vec();
        if let Err(error) = validate_token_reads(chain, &mut tokens) {
            tracing::warn!(%error, "unsupported token listing read; balances unchanged");
            return Vec::new();
        }
        let held: HashMap<String, crate::api::HeldToken> = held
            .into_iter()
            .filter_map(|holding| Some((holding_key(chain, &holding.contract)?, holding)))
            .collect();
        let reads = tokens.iter().map(|token| {
            let holding = holding_key(chain, &token.contract).and_then(|key| held.get(&key));
            async move {
                let (raw, decimals) = match holding {
                    None => (0, token.decimals),
                    Some(holding) => (
                        holding.balance_raw,
                        match holding.decimals {
                            Some(decimals) => decimals,
                            None => {
                                self.token_decimals(chain, &holding.contract, MetadataRead::Shared)
                                    .await?
                            }
                        },
                    ),
                };
                Ok::<_, SpectraBridgeError>(TokenBalanceResult::known(
                    chain,
                    token,
                    token.contract.clone(),
                    raw,
                    decimals,
                ))
            }
        });
        readable_tokens(futures::future::join_all(reads).await)
    }

    /// Balances of the tokens a wallet knows on `chain`.
    ///
    /// Where a configured service lists what the address holds, one listing
    /// answers for every known token, however many there are. Otherwise, or
    /// when the listing fails, each token is asked about in turn.
    pub(crate) async fn known_token_balances(
        &self,
        chain: Chain,
        address: String,
        mut tokens: Vec<TokenDescriptor>,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        validate_token_reads(chain, &mut tokens)?;
        match self.held_tokens(chain, &address).await {
            Ok(Some(held)) => Ok(self.balances_from_holdings(chain, held, &tokens).await),
            Ok(None) => self.fetch_token_balances(chain, address, tokens).await,
            Err(error) => {
                tracing::warn!(%error, chain = chain.str_id(), "token listing failed; reading each token");
                self.fetch_token_balances(chain, address, tokens).await
            }
        }
    }
}

impl WalletService {
    /// Every token this address actually holds, named where the catalog knows
    /// the contract and left unnamed where it does not.
    ///
    /// The complement of `fetch_token_balances`, which asks the chain about a
    /// list the caller already has: this asks what is there. That inverts
    /// three things at once —
    ///
    /// * **decimals come from the chain**, not from a copy that can disagree
    ///   with the contract it describes;
    /// * a token the catalog has never heard of still appears, instead of
    ///   being invisible until someone adds a row;
    /// * one listing replaces one call per known token.
    ///
    /// What the catalog still decides is the **name**. A discovered token's
    /// on-chain symbol is written by whoever deployed it, so it is never read
    /// here — `symbol` is the catalog's or empty, and `is_known` says which.
    /// A front end renders the contract address for the rest, which is the one
    /// string an attacker cannot choose.
    pub(crate) async fn discover_token_balances(
        &self,
        chain: crate::registry::Chain,
        address: String,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        let this = self.clone();
        crate::service::worker::run(async move {
            let this = &this;
            // Not an empty list: a chain nothing can list is not an address
            // that holds nothing, and the difference is what a user reads as
            // "my tokens are gone". A token contract answers only about a
            // holder named to it, so listing them needs an indexer.
            let held = this.held_tokens(chain, &address).await?.ok_or_else(|| {
                SpectraBridgeError::failed(
                    "%@ needs an address indexer to list the tokens an address holds. Add an endpoint that lists them.",
                    [chain.chain_display_name()],
                )
            })?;
            let known: HashMap<String, crate::catalog::tokens::TokenDeploymentEntry> =
                crate::catalog::tokens::list_token_deployments(Some(chain))
                    .into_iter()
                    .filter(|t| !t.is_native())
                    .filter_map(|t| Some((holding_key(chain, &t.contract)?, t)))
                    .collect();
            let rows = held.into_iter().map(|b| {
                let entry = holding_key(chain, &b.contract).and_then(|key| known.get(&key));
                async move {
                    // The chain's own count wins over the catalog's, and where
                    // neither vouches for one, zero is the only honest answer:
                    // the display then reads as the raw base-unit count it is,
                    // next to a contract address and no name.
                    let decimals = match b.decimals {
                        Some(decimals) => Some(decimals),
                        None => this
                            .token_decimals(chain, &b.contract, MetadataRead::Shared)
                            .await
                            .ok(),
                    }
                    .or_else(|| entry.and_then(|e| u8::try_from(e.decimals).ok()))
                    .unwrap_or(0);
                    TokenBalanceResult {
                        standard: entry
                            .and_then(|e| e.token_standard)
                            .or_else(|| chain.token_standard_for_identifier(&b.contract)),
                        contract_address: entry.map_or(b.contract, |e| e.contract.clone()),
                        symbol: entry.map(|e| e.symbol.clone()).unwrap_or_default(),
                        decimals,
                        balance_raw: b.balance_raw.to_string(),
                        balance_display: crate::amount::decimal::from_units(
                            b.balance_raw,
                            u32::from(decimals),
                        ),
                        is_known: entry.is_some(),
                    }
                }
            });
            Ok(futures::future::join_all(rows).await)
        })
        .await
    }
}

impl WalletService {
    /// Balances for a list of tokens, each asked of its own contract.
    ///
    /// For Solana `contract` is the mint address; for Sui it is the coin
    /// type; for Aptos the fungible asset's metadata address or a legacy coin
    /// type; for TON it is the jetton master address. A token that cannot be
    /// read is left out, never reported as zero.
    pub async fn fetch_token_balances(
        &self,
        chain: crate::registry::Chain,
        address: String,
        mut tokens: Vec<TokenDescriptor>,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        validate_token_reads(chain, &mut tokens)?;
        let endpoints = self
            .endpoints_for(
                chain,
                &[crate::api::endpoint::EndpointCapability::TokenBalance],
            )
            .await;

        // Choose the protocol by family while every read keeps the concrete
        // network's endpoints and identity.
        let results: Vec<TokenBalanceResult> = match chain.mainnet_counterpart() {
            Chain::Tron => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(TronHttpClient::with_metadata_cache(
                    endpoints,
                    chain,
                    self.trc20_metadata.clone(),
                ));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let contract = t.contract.clone();
                        let holder = address.clone();
                        let symbol = t.symbol.clone();
                        let standard = t.standard_on(chain);
                        async move {
                            if standard == Some(crate::registry::TokenStandard::Trc10) {
                                let b = client
                                    .fetch_trc10_balance(chain, &contract, &holder)
                                    .await?;
                                return Ok::<_, ApiError>(TokenBalanceResult {
                                    standard,
                                    contract_address: contract,
                                    symbol: if symbol.is_empty() {
                                        b.metadata.symbol
                                    } else {
                                        symbol
                                    },
                                    decimals: b.metadata.decimals,
                                    balance_raw: b.balance_raw.to_string(),
                                    balance_display: b.balance_display,
                                    is_known: true,
                                });
                            }
                            let b = client.fetch_trc20_balance(&contract, &holder).await?;
                            Ok::<_, ApiError>(TokenBalanceResult {
                                standard,
                                contract_address: contract,
                                symbol: if b.symbol.is_empty() {
                                    symbol
                                } else {
                                    b.symbol
                                },
                                decimals: b.decimals,
                                balance_raw: b.balance_raw,
                                balance_display: b.balance_display,
                                is_known: true,
                            })
                        }
                    })
                    .collect();
                readable_tokens(join_all(futs).await)
            }
            Chain::Solana => {
                // One request per mint, so an unreadable mint is that mint's
                // answer and not the whole wallet's.
                let client = SolanaClient::new(endpoints);
                let (client, owner) = (&client, address.as_str());
                let reads = tokens.iter().map(|token| async move {
                    // No account for the mint is a real zero. The mint's own
                    // decimal count comes with its account when there is one;
                    // the catalog's only stands in for a balance that is zero
                    // either way.
                    let (raw, decimals) = client
                        .fetch_spl_balance(owner, &token.contract)
                        .await?
                        .map_or((0, token.decimals), |b| (b.balance_raw, b.decimals));
                    Ok::<_, ApiError>(TokenBalanceResult::known(
                        chain,
                        token,
                        token.contract.clone(),
                        raw,
                        decimals,
                    ))
                });
                readable_tokens(futures::future::join_all(reads).await)
            }
            Chain::Near => {
                let client = NearClient::new(endpoints);
                let (client, holder) = (&client, address.as_str());
                self.own_decimals_balances(chain, &tokens, move |contract| async move {
                    client.fetch_ft_balance_of(&contract, holder).await
                })
                .await
            }
            Chain::Sui => {
                let client = SuiClient::new(endpoints);
                let (client, owner) = (&client, address.as_str());
                self.own_decimals_balances(chain, &tokens, move |coin_type| async move {
                    client.fetch_coin_balance(owner, &coin_type).await
                })
                .await
            }
            Chain::Aptos => {
                let client = AptosClient::new(endpoints);
                let (client, owner) = (&client, address.as_str());
                self.own_decimals_balances(chain, &tokens, move |identifier| async move {
                    client
                        .fetch_token_balance(owner, &identifier)
                        .await
                        .map(u128::from)
                })
                .await
            }
            // A contract call reads the balance and the token's own
            // `decimals()` together, one token at a time.
            Chain::Vechain => {
                let client = crate::api::thor_rest::ThorClient::new(endpoints);
                let (client, holder) = (&client, address.as_str());
                let reads = tokens.iter().map(|token| async move {
                    let (raw, decimals) =
                        client.fetch_token_balance(&token.contract, holder).await?;
                    Ok::<_, ApiError>(TokenBalanceResult::known(
                        chain,
                        token,
                        token.contract.clone(),
                        raw,
                        decimals,
                    ))
                });
                readable_tokens(futures::future::join_all(reads).await)
            }
            // Every token an ERC-20 contract the holder's balance is read
            // from, its decimals from the same contract.
            Chain::Starknet => {
                let client = crate::api::starknet_json_rpc::StarknetClient::new(endpoints);
                let holder = crate::derivation::starknet::decode_address(chain, &address)?;
                let (client, holder) = (&client, &holder);
                self.own_decimals_balances(chain, &tokens, move |contract| async move {
                    let token = crate::derivation::starknet::decode_address(chain, &contract)
                        .map_err(|error| ApiError::invalid(error.to_string()))?;
                    client.fetch_token_balance(&token, holder).await
                })
                .await
            }
            // Every asset from one pinned state; the asset's own metadata
            // states its decimals.
            Chain::Polkadot => {
                let holder = crate::derivation::polkadot::decode_ss58(chain, &address)?;
                let ids = tokens
                    .iter()
                    .map(|token| asset_id(&token.contract))
                    .collect::<Result<Vec<_>, _>>()?;
                let reads = SubstrateClient::new(endpoints)
                    .fetch_asset_balances(chain, &ids, &holder)
                    .await?;
                readable_tokens(
                    tokens
                        .iter()
                        .zip(reads)
                        .map(|(token, read)| {
                            read.map(|(raw, decimals)| {
                                TokenBalanceResult::known(
                                    chain,
                                    token,
                                    token.contract.clone(),
                                    raw,
                                    decimals,
                                )
                            })
                        })
                        .collect(),
                )
            }
            Chain::Ton => {
                // A jetton balance lives in a wallet contract derived per
                // holder, so even one jetton is read off the holder's list.
                let held = self
                    .held_tokens(chain, &address)
                    .await?
                    .ok_or_else(|| SpectraBridgeError::failure("no TON indexer configured"))?;
                self.balances_from_holdings(chain, held, &tokens).await
            }
            // One read of the account's lines or outputs answers for every
            // asset.
            Chain::Xrp | Chain::Stellar | Chain::Cardano => {
                let held = self.held_tokens(chain, &address).await?.ok_or_else(|| {
                    SpectraBridgeError::failure("no node configured to list the account's assets")
                })?;
                self.balances_from_holdings(chain, held, &tokens).await
            }
            // The EVM family.
            _ if chain.is_evm() => {
                // A row with no contract is a bad row, not a bad chain: it
                // cannot be read, and failing the request over it would take
                // every other token with it.
                let tokens: Vec<_> = tokens
                    .iter()
                    .filter(|token| {
                        let readable = !token.contract.is_empty();
                        if !readable {
                            tracing::warn!(symbol = %token.symbol, "token row has no contract");
                        }
                        readable
                    })
                    .collect();
                let contracts: Vec<String> =
                    tokens.iter().map(|t| t.contract.to_lowercase()).collect();
                // The contract's own `decimals()`, alongside the balance. A
                // catalog row that disagrees with the contract can only be the
                // one that is wrong, and the two numbers together are what a
                // balance means.
                let reads = EvmClient::new(endpoints, chain.evm_chain_id()?)
                    .fetch_erc20_balances(&address, &contracts)
                    .await;
                readable_tokens(
                    tokens
                        .iter()
                        .zip(contracts)
                        .zip(reads)
                        .map(|((token, contract), read)| {
                            read.map(|(raw, decimals)| {
                                TokenBalanceResult::known(chain, token, contract, raw, decimals)
                            })
                        })
                        .collect(),
                )
            }
            _ => {
                return Err(SpectraBridgeError::failure(format!(
                    "fetch_token_balances: unsupported chain: {chain:?}"
                )));
            }
        };

        Ok(results)
    }
}

impl WalletService {
    /// Each token read as two questions at once, its balance from `balance`
    /// and its decimals from its own contract: the token's own count wins
    /// over the catalog's, which can only ever be the one that is wrong.
    async fn own_decimals_balances<F, Fut>(
        &self,
        chain: Chain,
        tokens: &[TokenDescriptor],
        balance: F,
    ) -> Vec<TokenBalanceResult>
    where
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<u128, ApiError>>,
    {
        let reads = tokens.iter().map(|token| {
            let raw = balance(token.contract.clone());
            async move {
                let (raw, decimals) = tokio::join!(
                    raw,
                    self.token_decimals(chain, &token.contract, MetadataRead::Shared)
                );
                Ok::<_, SpectraBridgeError>(TokenBalanceResult::known(
                    chain,
                    token,
                    token.contract.clone(),
                    raw?,
                    decimals?,
                ))
            }
        });
        readable_tokens(futures::future::join_all(reads).await)
    }
}

/// Token descriptor passed across UniFFI without JSON-shuttle marshalling.
#[derive(Debug, Clone)]
pub struct TokenDescriptor {
    /// The actual deployment protocol; `None` resolves from identifier shape.
    pub standard: Option<crate::registry::TokenStandard>,
    pub contract: String,
    pub symbol: String,
    pub decimals: u8,
    pub name: Option<String>,
}

impl TokenDescriptor {
    pub(crate) fn standard_on(&self, chain: Chain) -> Option<crate::registry::TokenStandard> {
        self.standard
            .or_else(|| chain.token_standard_for_identifier(&self.contract))
    }
}

/// Typed token-balance result returned via UniFFI.
#[derive(Debug, Clone)]
pub struct TokenBalanceResult {
    /// The token's protocol, resolved from the read when the caller named none.
    pub standard: Option<crate::registry::TokenStandard>,
    pub contract_address: String,
    /// The catalog's symbol, or empty when the catalog does not know this
    /// contract.
    ///
    /// Empty is the anti-phishing property, not an omission: a discovered
    /// token's on-chain name and symbol are written by whoever deployed it, so
    /// rendering them would let an airdrop call itself "USDC". A front end
    /// showing the contract address instead is showing the one thing the
    /// attacker cannot forge.
    pub symbol: String,
    pub decimals: u8,
    pub balance_raw: String,
    pub balance_display: String,
    /// The catalog has an entry for this contract. False for anything found by
    /// discovery that nobody has vouched for.
    pub is_known: bool,
}

impl TokenBalanceResult {
    /// A catalog token's balance of `raw` base units at the `decimals` the
    /// read found, under `contract` as the read spells it.
    fn known(
        chain: Chain,
        token: &TokenDescriptor,
        contract: String,
        raw: u128,
        decimals: u8,
    ) -> Self {
        Self {
            standard: token.standard_on(chain),
            contract_address: contract,
            symbol: token.symbol.clone(),
            decimals,
            balance_raw: raw.to_string(),
            balance_display: crate::amount::decimal::from_units(raw, u32::from(decimals)),
            is_known: true,
        }
    }
}

#[cfg(test)]
#[path = "tests/network_tokens.rs"]
mod tests;
