//! Token discovery and balances, including partial provider failures.
use super::*;

/// Keep the tokens that answered; leave out the ones that did not.
///
/// A token read has three outcomes and they are not interchangeable: a
/// balance, a legitimate zero, and "the chain did not say". Reporting the
/// third as zero is what this code used to do, and a zero is a claim about
/// funds — a max-send computes from it and a user reads it as "gone". So
/// the fabricated zeros went. Collecting the batch into one `Result` went
/// too far the other way: one self-destructed contract failed every other
/// token with it, and the only caller does `try?`, so a wallet's whole
/// token list silently stopped updating until that contract was removed.
///
/// A token missing from this list is not updated by the caller, which
/// leaves its last known balance in place — the one answer that claims
/// nothing.
fn readable_tokens(results: Vec<Result<TokenBalanceResult, String>>) -> Vec<TokenBalanceResult> {
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

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Fetch balances for a list of tokens in one call.
    ///
    /// For Solana `contract` is the mint address; for Sui / Aptos it is the
    /// coin type; for TON it is the jetton master address.
    ///
    /// Tokens that fail to fetch are returned with `balance_raw = "0"` so the
    /// caller always gets back the full list.
    /// Every token this address actually holds, named where the catalog knows
    /// the contract and left unnamed where it does not.
    ///
    /// The complement of `fetch_token_balances`, which asks the chain about a
    /// list the caller already has: this asks the chain what is there. That
    /// inverts three things at once —
    ///
    /// * **decimals come from the chain**, not from a copy that can disagree
    ///   with the contract it describes;
    /// * a token the catalog has never heard of still appears, instead of
    ///   being invisible until someone adds a row;
    /// * one call replaces one call per known token.
    ///
    /// What the catalog still decides is the **name**. A discovered token's
    /// on-chain symbol is written by whoever deployed it, so it is never read
    /// here — `symbol` is the catalog's or empty, and `is_known` says which.
    /// A front end renders the contract address for the rest, which is the one
    /// string an attacker cannot choose.
    pub async fn discover_token_balances(
        &self,
        chain_id: String,
        address: String,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        let chain = Chain::from_str_id(&chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "discover_token_balances: unsupported chain_id: {chain_id}"
            ))
        })?;
        // The registry says which chains have a node that answers "what does
        // this address hold?". Refusing here rather than in the match below
        // keeps the two from drifting apart, which is how a chain ends up
        // silently reporting an empty wallet.
        if !chain.entry().enumerates_holdings {
            return Err(SpectraBridgeError::from(format!(
                "discover_token_balances: {} cannot enumerate holdings; \
                 a token contract only answers about a holder you name, so \
                 listing them needs an indexer",
                chain.str_id()
            )));
        }
        let endpoints = self.endpoints_for(chain.str_id()).await;
        // Not `unwrap_or_default()` on any arm: a node that will not answer is
        // not an address that holds nothing, and the difference is what a user
        // reads as "my tokens are gone".
        let held: Vec<crate::fetch::chains::HeldToken> = match chain {
            Chain::Solana | Chain::SolanaDevnet => SolanaClient::new(endpoints)
                .fetch_all_spl_balances(&address)
                .await
                .map_err(SpectraBridgeError::from)?
                .into_iter()
                .map(|b| crate::fetch::chains::HeldToken {
                    contract: b.mint,
                    balance_raw: b.balance_raw.parse().unwrap_or(0),
                    decimals: Some(b.decimals),
                    symbol: None,
                })
                .collect(),
            Chain::Tron | Chain::TronNile => TronClient::with_metadata_cache(
                endpoints,
                chain.str_id(),
                self.trc20_metadata.clone(),
            )
            .fetch_all_trc20_balances(&address)
            .await
            .map_err(SpectraBridgeError::from)?,
            Chain::Sui | Chain::SuiTestnet => SuiClient::new(endpoints)
                .fetch_all_coin_balances(&address)
                .await
                .map_err(SpectraBridgeError::from)?,
            Chain::Aptos | Chain::AptosTestnet => AptosClient::new(endpoints)
                .fetch_all_coin_balances(&address)
                .await
                .map_err(SpectraBridgeError::from)?,
            Chain::Ton | Chain::TonTestnet => {
                // The v3 API is the only one that enumerates jetton wallets; it
                // lives in the chain's Secondary endpoint slot.
                let v3 = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                TonClient::new(endpoints, api_key)
                    .with_v3_endpoints(v3)
                    .fetch_all_jetton_balances(&address)
                    .await
                    .map_err(SpectraBridgeError::from)?
            }
            // Unreachable: the registry gate above rejects every chain that
            // has no client arm here, and the test below holds the two together.
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "discover_token_balances: {c:?} is marked enumerable but has no client"
                )))
            }
        };
        let known: std::collections::HashMap<String, crate::tokens::TokenEntry> =
            crate::tokens::list_tokens(chain.str_id().to_string())
                .into_iter()
                .map(|t| (t.contract.clone(), t))
                .collect();
        Ok(held
            .into_iter()
            .map(|b| {
                let entry = known.get(&b.contract);
                // The chain's own count wins over the catalog's, and where
                // neither vouches for one, zero is the only honest answer: the
                // display then reads as the raw base-unit count it is, next to
                // a contract address and no name.
                let decimals: u8 = b
                    .decimals
                    .or_else(|| entry.and_then(|e| u8::try_from(e.decimals).ok()))
                    .unwrap_or(0);
                TokenBalanceResult {
                    contract_address: b.contract,
                    symbol: entry
                        .map(|e| e.symbol.clone())
                        .or_else(|| b.symbol.filter(|s| !s.is_empty()))
                        .unwrap_or_default(),
                    decimals,
                    balance_raw: b.balance_raw.to_string(),
                    balance_display: crate::fetch::chains::evm::format_token_amount(
                        b.balance_raw,
                        decimals,
                    ),
                    is_known: entry.is_some(),
                }
            })
            .collect())
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    pub async fn fetch_token_balances(
        &self,
        chain_id: String,
        address: String,
        tokens: Vec<TokenDescriptor>,
    ) -> Result<Vec<TokenBalanceResult>, SpectraBridgeError> {
        if tokens.is_empty() {
            return Ok(Vec::new());
        }

        let chain = Chain::from_str_id(&chain_id).ok_or_else(|| {
            SpectraBridgeError::from(format!(
                "fetch_token_balances: unsupported chain_id: {chain_id}"
            ))
        })?;
        let endpoints = self.endpoints_for(chain.str_id()).await;

        macro_rules! coin_token_balances {
            ($Client:ty, $endpoints:expr) => {{
                use futures::future::join_all;
                let client = std::sync::Arc::new(<$Client>::new($endpoints));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let address = address.clone();
                        let coin_type = t.contract.clone();
                        let symbol = t.symbol.clone();
                        async move {
                            // Balance and decimals together: the coin's own
                            // count wins over the catalog's, which can only
                            // ever be the one that is wrong.
                            let (raw, own) = tokio::join!(
                                client.fetch_coin_balance(&address, &coin_type),
                                client.fetch_coin_decimals(&coin_type)
                            );
                            let raw = raw?;
                            let decimals = crate::fetch::chains::checked_token_decimals(
                                u128::from(own.ok_or("token decimals unavailable")?),
                            )?;
                            Ok::<_, String>(TokenBalanceResult {
                                contract_address: coin_type,
                                symbol,
                                decimals,
                                balance_raw: raw.to_string(),
                                balance_display: format_decimals(raw as u128, decimals),
                                is_known: true,
                            })
                        }
                    })
                    .collect();
                readable_tokens(join_all(futs).await)
            }};
        }

        let results: Vec<TokenBalanceResult> = match chain {
            Chain::Tron => {
                use futures::future::join_all;
                let client = std::sync::Arc::new(TronClient::with_metadata_cache(
                    endpoints,
                    chain.str_id(),
                    self.trc20_metadata.clone(),
                ));
                let futs: Vec<_> = tokens
                    .iter()
                    .map(|t| {
                        let client = client.clone();
                        let contract = t.contract.clone();
                        let holder = address.clone();
                        let symbol = t.symbol.clone();
                        async move {
                            let b = client.fetch_trc20_balance(&contract, &holder).await?;
                            Ok::<_, String>(TokenBalanceResult {
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
                            Ok::<_, String>(TokenBalanceResult {
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
                        async move {
                            let (raw, meta) = tokio::join!(
                                client.fetch_ft_balance_of(&contract, &holder),
                                client.fetch_ft_metadata(&contract)
                            );
                            let raw = raw?;
                            let decimals = crate::fetch::chains::checked_token_decimals(
                                u128::from(meta?.decimals),
                            )?;
                            let display = format_decimals(raw, decimals);
                            Ok::<_, String>(TokenBalanceResult {
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
            Chain::Sui => coin_token_balances!(SuiClient, endpoints),
            Chain::Aptos => coin_token_balances!(AptosClient, endpoints),
            Chain::Ton => {
                // TON — jetton balances via TonCenter v3 API. The v3 endpoint
                // lives in the chain's Secondary slot (registered as id + 100 = 116).
                let v3_endpoints = self
                    .endpoints_for(&chain.endpoint_str_id(EndpointSlot::Secondary))
                    .await;
                let api_key = self.api_key_for(chain.str_id()).await;
                let client = TonClient::new(endpoints, api_key).with_v3_endpoints(v3_endpoints);
                let jetton_balances = client.fetch_jetton_balances(&address).await?;

                let own_decimals = futures::future::join_all(
                    tokens
                        .iter()
                        .map(|t| client.fetch_jetton_decimals(&t.contract)),
                )
                .await;

                let rows: Vec<Result<TokenBalanceResult, String>> = tokens
                    .iter()
                    .zip(own_decimals)
                    .map(|(t, own)| {
                        let raw = jetton_balances
                            .iter()
                            .find(|j| j.master_address.eq_ignore_ascii_case(&t.contract))
                            .map(|j| j.balance_raw)
                            .unwrap_or(0u128);
                        let decimals = crate::fetch::chains::checked_token_decimals(u128::from(
                            own.ok_or("token decimals unavailable")?,
                        ))?;
                        Ok::<_, String>(TokenBalanceResult {
                            contract_address: t.contract.clone(),
                            symbol: t.symbol.clone(),
                            decimals,
                            balance_raw: raw.to_string(),
                            balance_display: format_decimals(raw, decimals),
                            is_known: true,
                        })
                    })
                    .collect();
                readable_tokens(rows)
            }
            // The EVM family. This was `fetch_evm_token_balances_batch_typed`,
            // a second method with the *same* signature and the complementary
            // set of chains — so a caller holding a chain had to know which
            // family it was in to pick the right one, which is exactly what
            // the chain id already says.
            c if c.is_evm() => {
                let client = EvmClient::new(endpoints, c.evm_chain_id());
                let mut results = Vec::with_capacity(tokens.len());
                for token in &tokens {
                    let contract = token.contract.to_lowercase();
                    if contract.is_empty() {
                        // A row with no contract is a bad row, not a bad
                        // chain: it cannot be read, and failing the request
                        // over it would take every other token with it.
                        tracing::warn!(symbol = %token.symbol, "token row has no contract");
                        continue;
                    }
                    // The contract's own `decimals()`, alongside the balance.
                    // A catalog row that disagrees with the contract can only
                    // be the one that is wrong, and the two numbers together
                    // are what a balance means.
                    let (raw, meta) = tokio::join!(
                        client.fetch_erc20_balance_of(&contract, &address),
                        client.fetch_erc20_metadata(&contract)
                    );
                    let read = raw.and_then(|raw| {
                        let decimals = crate::fetch::chains::checked_token_decimals(u128::from(
                            meta?.decimals,
                        ))?;
                        Ok(TokenBalanceResult {
                            contract_address: contract,
                            symbol: token.symbol.clone(),
                            decimals,
                            balance_raw: raw.to_string(),
                            balance_display: format_decimals(raw, decimals),
                            is_known: true,
                        })
                    });
                    results.push(read);
                }
                readable_tokens(results)
            }
            c => {
                return Err(SpectraBridgeError::from(format!(
                    "fetch_token_balances: unsupported chain: {c:?}"
                )))
            }
        };

        Ok(results)
    }
}

#[cfg(test)]
mod discovery_names_only_what_the_catalog_vouches_for {
    use crate::registry::Chain;

    /// Discovery dispatches on a registry flag, so the flag and the client arms
    /// must agree. Offline the enumerable chains fail on their empty endpoint
    /// list; what matters is that they fail there and not on the gate.
    #[tokio::test]
    async fn the_registry_flag_and_the_client_arms_agree() {
        let service = crate::service::WalletService::new_typed(Vec::new()).expect("service");
        for chain in Chain::all() {
            let err = service
                .discover_token_balances(chain.str_id().into(), "whatever".into())
                .await
                .expect_err("no endpoints are configured, so nothing can succeed")
                .to_string();
            if chain.entry().enumerates_holdings {
                assert!(
                    !err.contains("cannot enumerate"),
                    "{} is marked enumerable but discovery refused it",
                    chain.str_id()
                );
                assert!(
                    !err.contains("no client"),
                    "{} is marked enumerable but has no client arm",
                    chain.str_id()
                );
            } else {
                assert!(
                    err.contains("cannot enumerate"),
                    "{} is not enumerable, so it must say so rather than \
                     return a list that reads as 'holds nothing': {err}",
                    chain.str_id()
                );
            }
        }
    }

    /// Every chain whose node can list holdings is marked, and no chain whose
    /// node cannot is. The EVM family and NEAR are the ones that cannot.
    #[test]
    fn only_the_chains_with_an_enumerating_rpc_are_marked() {
        let marked: Vec<&str> = Chain::all()
            .filter(|c| c.entry().enumerates_holdings)
            .map(|c| c.str_id())
            .collect();
        assert_eq!(
            marked,
            vec![
                "solana",
                "tron",
                "sui",
                "aptos",
                "ton",
                "tron-nile",
                "solana-devnet",
                "sui-testnet",
                "aptos-testnet",
                "ton-testnet",
            ]
        );
        assert!(
            Chain::all()
                .filter(|c| c.is_evm())
                .all(|c| !c.entry().enumerates_holdings),
            "an EVM token contract only answers about a holder you name"
        );
        assert!(!Chain::Near.entry().enumerates_holdings);
    }
}

#[cfg(test)]
mod decimals_come_from_the_chain {
    use crate::registry::Chain;
    use crate::service::{ChainEndpoints, TokenDescriptor, WalletService};
    use serde_json::json;

    /// A balance's decimals are the contract's, not the caller's.
    ///
    /// Tron read its contract and Solana its mint; the EVM family, NEAR, TON,
    /// Sui and Aptos passed the caller's number straight through. Where the
    /// catalog disagreed with the contract, the catalog was the one that could
    /// only be wrong — and `balance_display` was formatted with one number
    /// while `decimals` reported the other, so the two no longer described the
    /// same balance.
    ///
    /// A provider failure must remain an error, not an invented zero balance.
    #[tokio::test]
    async fn an_unreadable_contract_is_left_out_rather_than_reported_as_zero() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        for chain in [
            Chain::Ethereum,
            Chain::Tron,
            Chain::Near,
            Chain::Ton,
            Chain::Sui,
            Chain::Aptos,
            Chain::Solana,
        ] {
            let results = service
                .fetch_token_balances(
                    chain.str_id().into(),
                    "whoever".into(),
                    vec![TokenDescriptor {
                        contract: "0xdeadbeef".into(),
                        symbol: "TEST".into(),
                        decimals: 18,
                        name: None,
                    }],
                )
                .await;
            match chain {
                // TON reads every jetton the address holds in one call, so a
                // failure there is the chain's answer and not one token's.
                Chain::Ton => assert!(results.is_err(), "TON"),
                _ => assert!(
                    results
                        .unwrap_or_else(|e| panic!("{}: {e}", chain.chain_display_name()))
                        .is_empty(),
                    "{} fabricated a balance for a contract it could not read",
                    chain.chain_display_name()
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
        use wiremock::{matchers::any, Mock, MockServer, Request, ResponseTemplate};
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

        let service = WalletService::new_typed(vec![ChainEndpoints {
            chain_id: "ethereum".into(),
            endpoints: vec![server.uri()],
            api_key: None,
        }])
        .unwrap();
        let descriptor = |contract: &str| TokenDescriptor {
            contract: contract.into(),
            symbol: "TEST".into(),
            decimals: 18,
            name: None,
        };
        let rows = service
            .fetch_token_balances(
                "ethereum".into(),
                holder,
                vec![descriptor(&bad), descriptor(&good), descriptor("")],
            )
            .await
            .expect("one bad contract is not a failed request");
        assert_eq!(rows.len(), 1, "only the readable contract answers");
        assert!(rows[0].contract_address.eq_ignore_ascii_case(&good));
    }

    /// An empty list is not a fetch.
    #[tokio::test]
    async fn no_tokens_is_no_round_trip() {
        let service = WalletService::new_typed(Vec::new()).expect("service");
        let out = service
            .fetch_token_balances("ethereum".into(), "whoever".into(), Vec::new())
            .await
            .expect("an empty request cannot fail");
        assert!(out.is_empty());
    }
}
