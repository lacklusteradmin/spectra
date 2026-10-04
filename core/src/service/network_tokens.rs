//! Token discovery and balances, including partial provider failures.

use super::*;
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

fn validate_token_reads(
    chain: Chain,
    tokens: &mut [TokenDescriptor],
) -> Result<(), SpectraBridgeError> {
    for token in tokens {
        let standard = token.standard_on(chain).to_string();
        token.contract =
            crate::tokens::validate_protocol_identifier(chain, &standard, &token.contract)?;
        token.standard = standard.clone();
        if !chain.reads_token_standard(&standard) {
            return Err(SpectraBridgeError::invalid(format!(
                "{standard} balance reads are not supported"
            )));
        }
    }
    Ok(())
}

impl WalletService {
    /// TronGrid v1 account API bases declaring `required`: the catalog's and
    /// custom `trongrid-v1` entries under the catalog transport. Under an
    /// explicit override, or when none declares it, each primary node's own
    /// `/v1/accounts`, since TronGrid serves both APIs from one host.
    pub(crate) async fn tron_account_endpoints(
        &self,
        chain: Chain,
        primary: &[String],
        required: &[EndpointCapability],
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let accounts = if self
            .uses_catalog_endpoints
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.api_endpoints(chain, crate::EndpointApi::TrongridV1, required)
                .await?
        } else {
            Vec::new()
        };
        if !accounts.is_empty() {
            return Ok(accounts);
        }
        Ok(primary
            .iter()
            .map(|url| format!("{}/v1/accounts", url.trim_end_matches('/')))
            .collect())
    }
}

/// The same canonical identity used by catalog, custom tokens and storage.
fn holding_key(chain: Chain, contract: &str) -> Option<String> {
    crate::tokens::normalize_token_identifier(Some(contract.to_string()), chain)
}

impl WalletService {
    /// The URLs of an indexer `api` that lists holdings on `chain`: the
    /// catalog's and custom ones under the catalog transport, only custom
    /// ones under an explicit override, which names every service it means.
    async fn listing_endpoints(
        &self,
        chain: Chain,
        api: crate::EndpointApi,
    ) -> Result<Vec<String>, SpectraBridgeError> {
        let required: &[EndpointCapability] = if api == crate::EndpointApi::ToncenterV3 {
            &[
                EndpointCapability::TokenDiscovery,
                EndpointCapability::TokenBalance,
                EndpointCapability::Verification,
            ]
        } else {
            &[EndpointCapability::TokenDiscovery]
        };
        if self
            .uses_catalog_endpoints
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            self.api_endpoints(chain, api, required).await
        } else {
            Ok(self.custom_api_endpoints(chain, &[api], required).await)
        }
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
        let discovery = [EndpointCapability::TokenDiscovery];
        let held = match chain {
            Chain::Solana | Chain::SolanaDevnet => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                SolanaClient::new(endpoints)
                    .fetch_all_spl_balances(address)
                    .await?
                    .into_iter()
                    .map(|b| crate::api::HeldToken {
                        contract: b.mint,
                        balance_raw: b.balance_raw.parse().unwrap_or(0),
                        decimals: Some(b.decimals),
                    })
                    .collect()
            }
            Chain::Sui | Chain::SuiTestnet => {
                let endpoints = self.endpoints_for(chain, &discovery).await;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                SuiClient::new(endpoints)
                    .fetch_all_coin_balances(address)
                    .await?
            }
            Chain::Aptos | Chain::AptosTestnet => {
                let endpoints = self
                    .listing_endpoints(chain, crate::EndpointApi::AptosIndexer)
                    .await?;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                let expected = chain
                    .aptos_chain_id()
                    .ok_or_else(|| SpectraBridgeError::failure("Missing Aptos network identity"))?;
                crate::api::aptos_indexer::AptosIndexerClient::new(Arc::new(endpoints), expected)
                    .fetch_holdings(address)
                    .await?
            }
            Chain::Near | Chain::NearTestnet => {
                let endpoints = self
                    .listing_endpoints(chain, crate::EndpointApi::Nearblocks)
                    .await?;
                if endpoints.is_empty() {
                    return Ok(None);
                }
                crate::api::nearblocks::NearblocksClient::new(Arc::new(endpoints))
                    .fetch_ft_holdings(address)
                    .await?
            }
            Chain::Tron | Chain::TronNile => {
                let primary = self.endpoints_for(chain, &discovery).await;
                let accounts = self
                    .tron_account_endpoints(chain, &primary, &discovery)
                    .await?;
                let nodes = self
                    .endpoints_for(
                        chain,
                        &[
                            EndpointCapability::TokenBalance,
                            EndpointCapability::Verification,
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
            Chain::Ton | Chain::TonTestnet => {
                let v3 = self
                    .listing_endpoints(chain, crate::EndpointApi::ToncenterV3)
                    .await?;
                if v3.is_empty() {
                    return Ok(None);
                }
                crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(v3))
                    .fetch_jetton_balances(chain, address)
                    .await?
            }
            c if c.is_evm() => {
                let explorers = self
                    .listing_endpoints(chain, crate::EndpointApi::Blockscout)
                    .await?;
                if explorers.is_empty() {
                    return Ok(None);
                }
                let client = crate::api::blockscout::BlockscoutClient::new();
                crate::api::http::race(&explorers, |base| {
                    let client = &client;
                    async move { client.fetch_token_holdings(address, &base).await }
                })
                .await?
            }
            _ => return Ok(None),
        };
        Ok(Some(held))
    }

    /// A token's decimals as its contract states them, for a holding whose
    /// listing did not carry them.
    async fn token_decimals(&self, chain: Chain, contract: &str) -> Result<u8, SpectraBridgeError> {
        let standard = chain.token_standard_for_identifier(contract);
        crate::tokens::validate_protocol_identifier(chain, standard, contract)?;
        if !chain.reads_token_standard(standard) {
            return Err(SpectraBridgeError::invalid(format!(
                "{standard} metadata reads are not supported"
            )));
        }
        let balance = [EndpointCapability::TokenBalance];
        let decimals = match chain {
            Chain::Sui | Chain::SuiTestnet => {
                SuiClient::new(self.endpoints_for(chain, &balance).await)
                    .fetch_coin_decimals(contract)
                    .await
                    .or_decode("coin decimals unavailable")?
            }
            Chain::Tron | Chain::TronNile => {
                let client = TronHttpClient::with_metadata_cache(
                    self.endpoints_for(chain, &balance).await,
                    chain,
                    self.trc20_metadata.clone(),
                );
                if standard == "TRC-10" {
                    client.fetch_trc10_metadata(chain, contract).await?.decimals
                } else {
                    client.read_metadata(contract).await?.decimals
                }
            }
            Chain::Ton | Chain::TonTestnet => {
                crate::api::toncenter_v3::ToncenterV3Client::new(Arc::new(
                    self.api_endpoints(
                        chain,
                        crate::EndpointApi::ToncenterV3,
                        &[
                            EndpointCapability::Verification,
                            EndpointCapability::TokenBalance,
                        ],
                    )
                    .await?,
                ))
                .fetch_jetton_decimals(chain, contract)
                .await
                .or_decode("jetton decimals unavailable")?
            }
            c if c.is_evm() => {
                EvmClient::new(self.endpoints_for(chain, &balance).await, c.evm_chain_id()?)
                    .fetch_erc20_metadata(contract)
                    .await?
                    .decimals
            }
            c => {
                return Err(SpectraBridgeError::failure(format!(
                    "{} lists token decimals with its holdings",
                    c.str_id()
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
                            None => self.token_decimals(chain, &holding.contract).await?,
                        },
                    ),
                };
                Ok::<_, SpectraBridgeError>(TokenBalanceResult {
                    standard: token.standard_on(chain).to_string(),
                    contract_address: token.contract.clone(),
                    symbol: token.symbol.clone(),
                    decimals,
                    balance_raw: raw.to_string(),
                    balance_display: crate::decimal::from_units(raw, u32::from(decimals)),
                    is_known: true,
                })
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

#[uniffi::export(async_runtime = "tokio")]
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
    pub async fn discover_token_balances(
        &self,
        chain: crate::registry::Chain,
        address: String,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            // Not an empty list: a chain nothing can list is not an address
            // that holds nothing, and the difference is what a user reads as
            // "my tokens are gone".
            let held = this.held_tokens(chain, &address).await?.ok_or_else(|| {
                SpectraBridgeError::failure(format!(
                    "discover_token_balances: {} cannot enumerate holdings with the \
                     configured services; a token contract only answers about a \
                     holder you name, so listing them needs an indexer",
                    chain.str_id()
                ))
            })?;
            let known: HashMap<String, crate::tokens::TokenDeploymentEntry> =
                crate::tokens::list_token_deployments(Some(chain))
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
                        None => this.token_decimals(chain, &b.contract).await.ok(),
                    }
                    .or_else(|| entry.and_then(|e| u8::try_from(e.decimals).ok()))
                    .unwrap_or(0);
                    TokenBalanceResult {
                        standard: entry.map(|e| e.token_standard.clone()).unwrap_or_else(|| {
                            chain.token_standard_for_identifier(&b.contract).into()
                        }),
                        contract_address: entry.map_or(b.contract, |e| e.contract.clone()),
                        symbol: entry.map(|e| e.symbol.clone()).unwrap_or_default(),
                        decimals,
                        balance_raw: b.balance_raw.to_string(),
                        balance_display: crate::decimal::from_units(
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
            .endpoints_for(chain, &[EndpointCapability::TokenBalance])
            .await;

        // A token read as two questions to one client: its balance and its
        // own decimals.
        macro_rules! own_decimals_token_balances {
            ($Client:ty, $endpoints:expr, $balance:ident, $decimals:ident) => {{
                use futures::future::join_all;
                let client = std::sync::Arc::new(<$Client>::new($endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let address = address.clone();
                        let contract = t.contract.clone();
                        let symbol = t.symbol.clone();
                        let standard = t.standard_on(chain).to_string();
                        async move {
                            // Balance and decimals together: the token's own
                            // count wins over the catalog's, which can only
                            // ever be the one that is wrong.
                            let (raw, own) = tokio::join!(
                                client.$balance(&address, &contract),
                                client.$decimals(&contract)
                            );
                            let raw = raw?;
                            let decimals = crate::api::checked_token_decimals(u128::from(
                                own.or_decode("token decimals unavailable")?,
                            ))?;
                            Ok::<_, ApiError>(TokenBalanceResult {
                                standard,
                                contract_address: contract,
                                symbol,
                                decimals,
                                balance_raw: raw.to_string(),
                                balance_display: crate::decimal::from_units(
                                    raw as u128,
                                    u32::from(decimals),
                                ),
                                is_known: true,
                            })
                        }
                    })
                    .collect();
                readable_tokens(join_all(futs).await)
            }};
        }

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
                        let standard = t.standard_on(chain).to_string();
                        async move {
                            if standard == "TRC-10" {
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
                use futures::future::join_all;
                // One request per mint, which is what `fetch_spl_balances`
                // fans out to anyway — asked separately so an unreadable mint
                // is that mint's answer and not the whole wallet's.
                let client = std::sync::Arc::new(SolanaClient::new(endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let address = address.clone();
                        let mint = t.contract.clone();
                        let symbol = t.symbol.clone();
                        let standard = t.standard_on(chain).to_string();
                        let catalog_decimals = t.decimals;
                        async move {
                            let found = client
                                .fetch_spl_balances(&address, std::slice::from_ref(&mint))
                                .await?;
                            // An empty answer is the owner holding no account
                            // for this mint, which is a real zero. The mint's
                            // own decimal count comes with the parsed account
                            // when there is one; the catalog's only has to
                            // stand in for a balance that is zero either way.
                            let balance = found.into_iter().next();
                            Ok::<_, ApiError>(TokenBalanceResult {
                                standard,
                                contract_address: mint,
                                symbol,
                                decimals: balance
                                    .as_ref()
                                    .map(|b| b.decimals)
                                    .unwrap_or(catalog_decimals),
                                balance_raw: balance
                                    .as_ref()
                                    .map(|b| b.balance_raw.clone())
                                    .unwrap_or_else(|| "0".to_string()),
                                balance_display: balance
                                    .map(|b| b.balance_display)
                                    .unwrap_or_else(|| "0".to_string()),
                                is_known: true,
                            })
                        }
                    })
                    .collect();
                readable_tokens(join_all(futs).await)
            }
            Chain::Near => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(NearClient::new(endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let contract = t.contract.clone();
                        let holder = address.clone();
                        let symbol = t.symbol.clone();
                        let standard = t.standard_on(chain).to_string();
                        async move {
                            let (raw, meta) = tokio::join!(
                                client.fetch_ft_balance_of(&contract, &holder),
                                client.fetch_ft_metadata(&contract)
                            );
                            let raw = raw?;
                            let decimals =
                                crate::api::checked_token_decimals(u128::from(meta?.decimals))?;
                            let display = crate::decimal::from_units(raw, u32::from(decimals));
                            Ok::<_, ApiError>(TokenBalanceResult {
                                standard,
                                contract_address: contract,
                                symbol,
                                decimals,
                                balance_raw: raw.to_string(),
                                balance_display: display,
                                is_known: true,
                            })
                        }
                    })
                    .collect();
                readable_tokens(join_all(futs).await)
            }
            Chain::Sui => own_decimals_token_balances!(
                SuiClient,
                endpoints,
                fetch_coin_balance,
                fetch_coin_decimals
            ),
            Chain::Aptos => own_decimals_token_balances!(
                AptosClient,
                endpoints,
                fetch_token_balance,
                fetch_token_decimals
            ),
            Chain::Ton => {
                // A jetton balance lives in a wallet contract derived per
                // holder, so even one jetton is read off the holder's list.
                let held = self
                    .held_tokens(chain, &address)
                    .await?
                    .ok_or_else(|| SpectraBridgeError::failure("no TON indexer configured"))?;
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
                            read.map(|(raw, decimals)| TokenBalanceResult {
                                standard: token.standard_on(chain).to_string(),
                                contract_address: contract,
                                symbol: token.symbol.clone(),
                                decimals,
                                balance_raw: raw.to_string(),
                                balance_display: crate::decimal::from_units(
                                    raw,
                                    u32::from(decimals),
                                ),
                                is_known: true,
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

#[cfg(test)]
mod a_listing_answers_for_every_known_token {
    use super::holding_key;
    use crate::registry::Chain;
    use crate::service::{TokenDescriptor, WalletService};

    /// Offline nothing is configured, so nothing can list holdings, and the
    /// answer says so rather than returning a list that reads as "holds
    /// nothing".
    #[tokio::test]
    async fn a_chain_nothing_can_list_says_so() {
        let service = WalletService::new(Vec::new()).expect("service");
        for chain in Chain::all() {
            let err = service
                .discover_token_balances(chain, "whatever".into())
                .await
                .expect_err("no service is configured, so nothing can list");
            assert!(
                err.to_string().contains("cannot enumerate"),
                "{}: {err}",
                chain.str_id()
            );
        }
    }

    /// A known token absent from a listing holds none; one present takes the
    /// listing's balance and decimals; a listing row nobody knows is ignored.
    #[tokio::test]
    async fn absent_is_zero_and_present_is_the_listed_balance() {
        let service = WalletService::new(Vec::new()).expect("service");
        let usdc = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
        let usdt = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
        let descriptor = |contract: &str| TokenDescriptor {
            standard: String::new(),
            contract: contract.into(),
            symbol: "TKN".into(),
            decimals: 18,
            name: None,
        };
        let rows = service
            .balances_from_holdings(
                Chain::Ethereum,
                vec![
                    crate::api::HeldToken {
                        contract: usdc.to_lowercase(),
                        balance_raw: 2_500_000,
                        decimals: Some(6),
                    },
                    crate::api::HeldToken {
                        contract: format!("0x{}", "99".repeat(20)),
                        balance_raw: 1,
                        decimals: Some(0),
                    },
                ],
                &[descriptor(usdc), descriptor(usdt)],
            )
            .await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].balance_display, "2.5");
        assert_eq!(
            rows[0].decimals, 6,
            "the listing's decimals, not the caller's"
        );
        assert_eq!(rows[1].balance_raw, "0");
    }

    /// The catalog spells a jetton master user-friendly; the indexer, raw.
    #[test]
    fn a_ton_master_matches_in_either_spelling() {
        assert_eq!(
            holding_key(
                Chain::Ton,
                "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs"
            ),
            holding_key(
                Chain::Ton,
                "0:B113A994B5024A16719F69139328EB759596C38A25F59028B146FECDC3621DFE"
            )
        );
        assert_eq!(
            holding_key(Chain::Ethereum, "0xABC"),
            holding_key(Chain::Ethereum, "0xabc")
        );
    }
}

#[cfg(test)]
mod decimals_come_from_the_chain {
    use crate::registry::Chain;
    use crate::service::{ChainEndpoints, TokenDescriptor, WalletService};
    use serde_json::json;

    /// A balance's decimals are the contract's, not the caller's: where the
    /// catalog disagrees with the contract, the catalog is the one that can
    /// only be wrong.
    ///
    /// A provider failure must remain an error, not an invented zero balance.
    #[tokio::test]
    async fn an_unreadable_contract_is_left_out_rather_than_reported_as_zero() {
        let service = WalletService::new(Vec::new()).expect("service");
        for chain in Chain::all().filter(|chain| {
            chain.hosts_tokens()
                && (chain.is_evm()
                    || matches!(
                        chain.mainnet_counterpart(),
                        Chain::Tron
                            | Chain::Near
                            | Chain::Ton
                            | Chain::Sui
                            | Chain::Aptos
                            | Chain::Solana
                    ))
        }) {
            let contract = match chain.mainnet_counterpart() {
                Chain::Tron => "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
                Chain::Solana => "11111111111111111111111111111111",
                Chain::Sui => "0x2::coin::T",
                Chain::Aptos => "0x1",
                Chain::Near => "token.near",
                Chain::Ton => "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs",
                _ => "0x1111111111111111111111111111111111111111",
            };
            let results = service
                .fetch_token_balances(
                    chain,
                    "whoever".into(),
                    vec![TokenDescriptor {
                        standard: String::new(),
                        contract: contract.into(),
                        symbol: "TEST".into(),
                        decimals: 18,
                        name: None,
                    }],
                )
                .await;
            match chain.mainnet_counterpart() {
                // TON reads every jetton the address holds in one call, so a
                // failure there is the chain's answer and not one token's.
                Chain::Ton => assert!(results.is_err(), "ton"),
                _ => assert!(
                    results
                        .unwrap_or_else(|e| panic!("{}: {e}", chain.str_id()))
                        .is_empty(),
                    "{} fabricated a balance for a contract it could not read",
                    chain.str_id()
                ),
            }
        }
    }

    /// One unreadable contract is that contract's answer, not the wallet's.
    /// Collecting the batch into a single `Result` meant a self-destructed
    /// token stopped every other token in the wallet from refreshing, and the
    /// only caller discards the error, so it stopped silently.
    #[tokio::test]
    async fn an_unreadable_token_does_not_take_the_readable_ones_with_it() {
        use wiremock::{Mock, MockServer, Request, ResponseTemplate, matchers::any};
        let holder = format!("0x{}", "33".repeat(20));
        let good = format!("0x{}", "11".repeat(20));
        let bad = format!("0x{}", "22".repeat(20));
        let server = MockServer::start().await;
        let refused = bad.clone();
        Mock::given(any())
            .respond_with(move |req: &Request| {
                let body: serde_json::Value = req.body_json().unwrap();
                let calls = body
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![body.clone()]);
                let replies: Vec<_> = calls
                    .iter()
                    .map(|call| {
                        let to = call["params"][0]["to"].as_str().unwrap_or_default();
                        if to.eq_ignore_ascii_case(&refused) {
                            return json!({"jsonrpc":"2.0","id":call["id"],
                                   "error":{"code":-32000,"message":"no code at address"}});
                        }
                        // balanceOf and decimals are both plain uint256 words;
                        // symbol decodes to empty, which the catalog covers.
                        json!({"jsonrpc":"2.0","id":call["id"],
                               "result":format!("0x{:064x}", 6)})
                    })
                    .collect();
                ResponseTemplate::new(200).set_body_json(if body.is_array() {
                    json!(replies)
                } else {
                    replies[0].clone()
                })
            })
            .mount(&server)
            .await;

        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: crate::EndpointCapability::ALL.to_vec(),
            chain_id: crate::registry::Chain::Ethereum,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let descriptor = |contract: &str| TokenDescriptor {
            standard: String::new(),
            contract: contract.into(),
            symbol: "TEST".into(),
            decimals: 18,
            name: None,
        };
        let rows = service
            .fetch_token_balances(
                crate::registry::Chain::Ethereum,
                holder,
                vec![descriptor(&bad), descriptor(&good)],
            )
            .await
            .expect("one bad contract is not a failed request");
        assert_eq!(rows.len(), 1, "only the readable contract answers");
        assert!(rows[0].contract_address.eq_ignore_ascii_case(&good));
    }

    /// An Aptos fungible asset is read by its metadata address through the
    /// primary store, and a legacy coin by its type through `coin::balance`.
    /// Both used to be read as a `CoinStore<T>`, which no fungible asset has,
    /// so every catalog Aptos token was left out.
    #[tokio::test]
    async fn aptos_reads_a_fungible_asset_and_a_coin_through_view_functions() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let owner = "0x6a2c9b3d1f64c2ae7c0e79c56b4b2f8e07c1b5f2aa7ec8bb0e0f2e7c4b6d8e1a";
        let usdc = "0xbae207659db88bea0cbead6da0ed00aac12edcdda169e591cd41c94180b46f3b";
        let coin = "0x5e156f1207d0ebfa19a9eeff00d62a282278fb8719f4fab3a586a0a2c0fffbea::coin::T";
        let metadata = "0x1::fungible_asset::Metadata";
        let server = MockServer::start().await;
        for (request, reply) in [
            (
                json!({"function": "0x1::primary_fungible_store::balance",
                       "type_arguments": [metadata], "arguments": [owner, usdc]}),
                json!(["2500000"]),
            ),
            (
                json!({"function": "0x1::fungible_asset::decimals",
                       "type_arguments": [metadata], "arguments": [usdc]}),
                json!([6]),
            ),
            (
                json!({"function": "0x1::coin::balance",
                       "type_arguments": [coin], "arguments": [owner]}),
                json!(["700"]),
            ),
            (
                json!({"function": "0x1::coin::decimals",
                       "type_arguments": [coin], "arguments": []}),
                json!([2]),
            ),
        ] {
            Mock::given(method("POST"))
                .and(path("/view"))
                .and(body_json(request))
                .respond_with(ResponseTemplate::new(200).set_body_json(reply))
                .expect(1)
                .mount(&server)
                .await;
        }

        let service = WalletService::new(vec![ChainEndpoints {
            capabilities: crate::EndpointCapability::ALL.to_vec(),
            chain_id: Chain::Aptos,
            endpoints: vec![server.uri()],
        }])
        .unwrap();
        let descriptor = |contract: &str| TokenDescriptor {
            standard: String::new(),
            contract: contract.into(),
            symbol: "TEST".into(),
            // Wrong on purpose: the token's own count is the one used.
            decimals: 18,
            name: None,
        };
        let rows = service
            .fetch_token_balances(
                Chain::Aptos,
                owner.into(),
                vec![descriptor(usdc), descriptor(coin)],
            )
            .await
            .expect("both tokens are readable");
        let read: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.contract_address.as_str(),
                    r.decimals,
                    r.balance_raw.as_str(),
                    r.balance_display.as_str(),
                )
            })
            .collect();
        assert_eq!(read, [(usdc, 6, "2500000", "2.5"), (coin, 2, "700", "7")]);
        assert_eq!(rows[0].standard, "AIP-21");
        assert_eq!(rows[1].standard, "Aptos Coin");
    }

    #[tokio::test]
    async fn unsupported_or_wrong_protocol_is_rejected_before_requests_and_cannot_become_listing_zero()
     {
        use wiremock::MockServer;
        let server = MockServer::start().await;
        let service = WalletService::new(vec![ChainEndpoints {
            chain_id: Chain::Tron,
            endpoints: vec![server.uri()],
            capabilities: crate::EndpointCapability::ALL.to_vec(),
        }])
        .unwrap();
        let token = TokenDescriptor {
            standard: "TRC-20".into(),
            contract: "1002000".into(),
            symbol: "T10".into(),
            decimals: 6,
            name: None,
        };
        let mut legacy = [TokenDescriptor {
            standard: "TRC-10".into(),
            ..token.clone()
        }];
        super::validate_token_reads(Chain::Tron, &mut legacy).expect("TRC-10 is supported on Tron");
        for result in [
            service
                .fetch_token_balances(Chain::Tron, "owner".into(), vec![token.clone()])
                .await,
            service
                .known_token_balances(Chain::Tron, "owner".into(), vec![token.clone()])
                .await,
        ] {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("invalid token identifier for protocol")
            );
        }
        assert!(
            service
                .balances_from_holdings(Chain::Tron, Vec::new(), &[token])
                .await
                .is_empty()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        let descriptor = TokenDescriptor {
            standard: "ERC-20".into(),
            contract: "11111111111111111111111111111111".into(),
            symbol: "BAD".into(),
            decimals: 6,
            name: None,
        };
        assert!(
            service
                .fetch_token_balances(Chain::Solana, "owner".into(), vec![descriptor])
                .await
                .unwrap_err()
                .to_string()
                .contains("protocol does not belong")
        );
    }

    /// An empty list is not a fetch.
    #[tokio::test]
    async fn no_tokens_is_no_round_trip() {
        let service = WalletService::new(Vec::new()).expect("service");
        let out = service
            .fetch_token_balances(
                crate::registry::Chain::Ethereum,
                "whoever".into(),
                Vec::new(),
            )
            .await
            .expect("an empty request cannot fail");
        assert!(out.is_empty());
    }
}
