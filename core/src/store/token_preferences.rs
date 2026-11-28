//! The tokens a wallet knows: the built-in catalog's and the ones the user
//! added, and the commands that change them.

use serde::{Deserialize, Serialize};

use super::state::{ResidentState, StateCommand, StateEvent};
use super::wallet_domain;

/// Why a token-preference change was refused. Front ends map these to their
/// own wording; the decision itself is core's.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TokenPreferenceRejection {
    /// The chain does not host tokens at all.
    UnknownChain,
    EmptySymbol,
    /// Longer than a symbol is ever spelled; almost always a pasted name.
    SymbolTooLong,
    EmptyName,
    InvalidPriceId,
    EmptyContract,
    /// Not a well-formed contract for the chain that would host it.
    InvalidContract,
    /// The chain already has a row for this contract.
    DuplicateToken,
    /// More places than any token has. A clamp here would silently read a
    /// balance at the wrong scale.
    TooManyDecimals,
    /// The catalog ships it, so it is not the user's to edit or remove.
    BuiltInToken,
    /// No row for that chain and contract.
    UnknownToken,
    /// The network has more than one token protocol and none was named. A
    /// guess from the identifier's shape cannot tell an ERC-20 from a BEP-20.
    StandardRequired,
    /// The named protocol is not one the network supports.
    UnsupportedStandard,
    /// The token's protocol does not fix its decimal places and none were
    /// given.
    DecimalsRequired,
}

/// Most tokens use 18 or fewer; the ceiling exists to stop a typo from
/// producing an unrenderable amount.
pub const MAX_TOKEN_DECIMALS: u32 = 30;

/// Longer than any token symbol is spelled. Past this the field holds a pasted
/// name or a whole contract address.
pub(crate) const MAX_TOKEN_SYMBOL_CHARS: usize = 12;

/// Where a (chain, contract) pair sits in the preference list, if at all.
///
/// Matched on the *normalized* contract, which is the chain's own rule — a TON
/// jetton address is case-significant and an EVM one is not.
fn token_preference_index(
    state: &ResidentState,
    chain: crate::registry::Chain,
    contract: &str,
) -> Option<usize> {
    token_preference_row(state, token_hosting_chain(chain)?, contract)
}

/// The chain a token command names, if it can hold tracked tokens.
fn token_hosting_chain(chain: crate::registry::Chain) -> Option<crate::registry::Chain> {
    chain.hosts_tokens().then_some(chain)
}

fn token_preference_row(
    state: &ResidentState,
    hosting: crate::registry::Chain,
    contract: &str,
) -> Option<usize> {
    let needle =
        crate::catalog::tokens::normalize_token_identifier(Some(contract.to_string()), hosting)?;
    state.token_preferences.iter().position(|entry| {
        entry.hosting_chain() == Some(hosting)
            && crate::catalog::tokens::normalize_token_identifier(
                Some(entry.token.contract.clone()),
                hosting,
            )
            .as_deref()
                == Some(needle.as_str())
    })
}

fn valid_price_id(value: &str) -> bool {
    value
        .trim()
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn token_preference_rejected(reason: TokenPreferenceRejection) -> StateEvent {
    StateEvent::TokenPreferenceRejected { reason }
}

/// The built-in token catalog, as preference entries.
///
/// Built from `tokens.toml` — the same catalog `list_all_builtin_token_deployments`
/// serves.
///
/// `id` is derived from chain and contract rather than minted at random: a
/// built-in's identity *is* its contract.
pub fn built_in_token_preferences() -> Vec<wallet_domain::TokenPreferenceEntry> {
    let mut entries: Vec<_> = crate::catalog::tokens::catalog()
        .iter()
        .filter(|token| !token.is_native())
        .filter_map(|token| {
            // A catalog row on a chain that cannot host tokens is a data
            // mistake, and skipping it is how it stays one.
            Some(token.chain_id).filter(|c| c.hosts_tokens())?;
            Some(wallet_domain::TokenPreferenceEntry {
                is_built_in: true,
                token: token.clone(),
            })
        })
        .collect();
    sort_token_preferences(&mut entries);
    entries
}

/// The built-in catalog with the user's stored tokens: a stored token the
/// catalog now ships is the catalog's row. Each normalized
/// network/identifier appears once, even when its protocol label differs.
pub fn with_custom_tokens(
    custom: Vec<crate::catalog::tokens::TokenDeploymentEntry>,
) -> Vec<wallet_domain::TokenPreferenceEntry> {
    let mut merged = built_in_token_preferences();
    let key = |entry: &wallet_domain::TokenPreferenceEntry| {
        (
            entry.token.chain_id,
            crate::catalog::tokens::normalize_token_identifier(
                Some(entry.token.contract.clone()),
                entry.token.chain_id,
            ),
        )
    };
    let mut seen: std::collections::HashSet<_> = merged.iter().map(key).collect();
    merged.extend(
        custom
            .into_iter()
            .map(|token| wallet_domain::TokenPreferenceEntry {
                is_built_in: false,
                token,
            })
            .filter(|entry| seen.insert(key(entry))),
    );
    sort_token_preferences(&mut merged);
    merged
}

/// Known Tokens: the catalog's tokens, native coins included, then each token
/// the user added as one of its own. `preferences` are the tracked tokens,
/// which hold the user's.
pub(crate) fn known_tokens(
    preferences: &[wallet_domain::TokenPreferenceEntry],
) -> Vec<crate::catalog::tokens::KnownToken> {
    crate::catalog::tokens::built_in_known_tokens()
        .iter()
        .cloned()
        .chain(
            preferences
                .iter()
                .filter(|entry| !entry.is_built_in)
                .map(|entry| crate::catalog::tokens::KnownToken::custom(entry.token.clone())),
        )
        .collect()
}

/// The catalog's own rows before the user's, then symbol, then token, then
/// chain: one token's deployments sit together, so a list grouped by token
/// keeps this order without sorting again. Every writer of the list sorts
/// with this, so an edited list matches the one a reload builds.
pub(crate) fn sort_token_preferences(entries: &mut [wallet_domain::TokenPreferenceEntry]) {
    entries.sort_by(|lhs, rhs| {
        rhs.is_built_in
            .cmp(&lhs.is_built_in)
            .then_with(|| lhs.token.symbol.cmp(&rhs.token.symbol))
            .then_with(|| lhs.token.token_id.cmp(&rhs.token.token_id))
            .then_with(|| lhs.token.chain_id.str_id().cmp(rhs.token.chain_id.str_id()))
    });
}

/// Apply a token preferences command; `reduce_state_in_place` routes only these here.
pub(super) fn reduce(
    state: &mut ResidentState,
    command: StateCommand,
    events: &mut Vec<StateEvent>,
) {
    match command {
        StateCommand::AddCustomToken {
            standard,
            chain_id,
            symbol,
            name,
            contract,
            coingecko_id,
            coinpaprika_id,
            decimals,
        } => {
            let symbol = symbol.trim().to_uppercase();
            let name = name.trim().to_string();
            let mut contract =
                crate::catalog::tokens::normalize_token_identifier(Some(contract), chain_id)
                    .unwrap_or_default();
            let hosting = token_hosting_chain(chain_id);
            // Named, never guessed: the network's only protocol, or the one
            // asked for among several.
            let standard = match (standard, chain_id.token_standards()) {
                (Some(named), _) if chain_id.allows_token_standard(named) => Ok(named),
                (Some(_), _) => Err(TokenPreferenceRejection::UnsupportedStandard),
                (None, [only]) => Ok(*only),
                (None, _) => Err(TokenPreferenceRejection::StandardRequired),
            };
            if let Ok(standard) = standard
                && let Ok(normalized) = crate::catalog::tokens::validate_protocol_identifier(
                    chain_id, standard, &contract,
                )
            {
                contract = normalized;
            }
            // Where the protocol fixes a token's places, they are the
            // protocol's, whatever was typed; otherwise they must be given.
            let decimals = standard
                .ok()
                .and_then(|standard| standard.fixed_decimals())
                .or(decimals);

            let rejection = match (hosting, standard) {
                (None, _) => Some(TokenPreferenceRejection::UnknownChain),
                (Some(_), Err(reason)) => Some(reason),
                (Some(hosting), Ok(standard)) => match () {
                    _ if symbol.is_empty() => Some(TokenPreferenceRejection::EmptySymbol),
                    // Twelve characters is longer than any symbol is spelled;
                    // past it the field has a pasted name or a whole address.
                    _ if symbol.chars().count() > MAX_TOKEN_SYMBOL_CHARS => {
                        Some(TokenPreferenceRejection::SymbolTooLong)
                    }
                    _ if name.is_empty() => Some(TokenPreferenceRejection::EmptyName),
                    _ if !valid_price_id(&coingecko_id) || !valid_price_id(&coinpaprika_id) => {
                        Some(TokenPreferenceRejection::InvalidPriceId)
                    }
                    _ if contract.is_empty() => Some(TokenPreferenceRejection::EmptyContract),
                    _ if decimals.is_none() => Some(TokenPreferenceRejection::DecimalsRequired),
                    _ if decimals > Some(MAX_TOKEN_DECIMALS) => {
                        Some(TokenPreferenceRejection::TooManyDecimals)
                    }
                    _ if crate::catalog::tokens::validate_protocol_identifier(
                        hosting, standard, &contract,
                    )
                    .is_err() =>
                    {
                        Some(TokenPreferenceRejection::InvalidContract)
                    }
                    _ if token_preference_row(state, hosting, &contract).is_some() => {
                        Some(TokenPreferenceRejection::DuplicateToken)
                    }
                    // The coin's own ERC-20 face: its balance is the native
                    // row the network already has, and a second row would
                    // count it twice.
                    _ if hosting
                        .evm_native_coin_contract()
                        .is_some_and(|native| native.eq_ignore_ascii_case(&contract)) =>
                    {
                        Some(TokenPreferenceRejection::DuplicateToken)
                    }
                    _ => None,
                },
            };

            match (rejection, hosting, standard) {
                (Some(reason), ..) => events.push(token_preference_rejected(reason)),
                (None, Some(hosting), Ok(standard)) => {
                    let deployment_id =
                        crate::catalog::tokens::deployment_id(hosting, Some(standard), &contract);
                    state.token_preferences.push(
                        crate::store::wallet_domain::TokenPreferenceEntry {
                            is_built_in: false,
                            token: crate::catalog::tokens::TokenDeploymentEntry {
                                token_id: format!("custom:{deployment_id}"),
                                deployment_id,
                                chain_id: hosting,
                                name,
                                symbol: symbol.clone(),
                                token_standard: Some(standard),
                                contract,
                                coingecko_id: coingecko_id.trim().to_lowercase(),
                                coinpaprika_id: coinpaprika_id.trim().to_lowercase(),
                                decimals: decimals.unwrap_or_default(),
                                tags: Vec::new(),
                                color: None,
                                artwork_name: String::new(),
                            },
                        },
                    );
                    sort_token_preferences(&mut state.token_preferences);
                    events.push(StateEvent::TokenPreferencesChanged {
                        symbol: Some(symbol),
                    });
                }
                (None, ..) => unreachable!("an unknown chain or standard is rejected above"),
            }
        }
        StateCommand::UpdateCustomToken {
            chain_id,
            contract,
            symbol,
            name,
            coingecko_id,
            coinpaprika_id,
            decimals,
        } => {
            let symbol = symbol.trim().to_uppercase();
            let name = name.trim().to_string();
            let index = token_preference_index(state, chain_id, &contract);
            // The token keeps the protocol it was added under; where that
            // fixes its places, they are the protocol's.
            let decimals = index
                .and_then(|i| state.token_preferences[i].token.token_standard)
                .and_then(|standard| standard.fixed_decimals())
                .or(decimals);
            let rejection = match index {
                None => Some(TokenPreferenceRejection::UnknownToken),
                Some(i) if state.token_preferences[i].is_built_in => {
                    Some(TokenPreferenceRejection::BuiltInToken)
                }
                Some(_) if symbol.is_empty() => Some(TokenPreferenceRejection::EmptySymbol),
                Some(_) if symbol.chars().count() > MAX_TOKEN_SYMBOL_CHARS => {
                    Some(TokenPreferenceRejection::SymbolTooLong)
                }
                Some(_) if name.is_empty() => Some(TokenPreferenceRejection::EmptyName),
                Some(_) if !valid_price_id(&coingecko_id) || !valid_price_id(&coinpaprika_id) => {
                    Some(TokenPreferenceRejection::InvalidPriceId)
                }
                Some(_) if decimals.is_none() => Some(TokenPreferenceRejection::DecimalsRequired),
                Some(_) if decimals > Some(MAX_TOKEN_DECIMALS) => {
                    Some(TokenPreferenceRejection::TooManyDecimals)
                }
                Some(_) => None,
            };
            if let Some(reason) = rejection {
                events.push(token_preference_rejected(reason));
            } else if let Some(index) = index {
                let token = &mut state.token_preferences[index].token;
                token.name = name;
                token.symbol = symbol.clone();
                token.coingecko_id = coingecko_id.trim().to_lowercase();
                token.coinpaprika_id = coinpaprika_id.trim().to_lowercase();
                token.decimals = decimals.unwrap_or_default();
                state.quotes.prices.remove(&token.deployment_id);
                state.quotes.prices_attempt_at = None;
                sort_token_preferences(&mut state.token_preferences);
                events.push(StateEvent::TokenPreferencesChanged {
                    symbol: Some(symbol),
                });
            }
        }
        StateCommand::RemoveCustomToken { chain_id, contract } => {
            match token_preference_index(state, chain_id, &contract) {
                None => events.push(token_preference_rejected(
                    TokenPreferenceRejection::UnknownToken,
                )),
                Some(index) if state.token_preferences[index].is_built_in => events.push(
                    token_preference_rejected(TokenPreferenceRejection::BuiltInToken),
                ),
                Some(index) => {
                    let removed = state.token_preferences.remove(index);
                    events.push(StateEvent::TokenPreferencesChanged {
                        symbol: Some(removed.token.symbol),
                    });
                }
            }
        }
        StateCommand::SetCustomTokenDecimals {
            chain_id,
            contract,
            decimals,
        } => match token_preference_index(state, chain_id, &contract) {
            None => events.push(token_preference_rejected(
                TokenPreferenceRejection::UnknownToken,
            )),
            Some(index) if state.token_preferences[index].is_built_in => events.push(
                token_preference_rejected(TokenPreferenceRejection::BuiltInToken),
            ),
            Some(_) if decimals > MAX_TOKEN_DECIMALS => events.push(token_preference_rejected(
                TokenPreferenceRejection::TooManyDecimals,
            )),
            Some(index) => {
                if state.token_preferences[index].token.decimals != decimals {
                    state.token_preferences[index].token.decimals = decimals;
                    events.push(StateEvent::TokenPreferencesChanged {
                        symbol: Some(state.token_preferences[index].token.symbol.clone()),
                    });
                }
            }
        },
        StateCommand::ResetTokenPreferences => {
            let defaults = built_in_token_preferences();
            if defaults != state.token_preferences {
                state.token_preferences = defaults;
                sort_token_preferences(&mut state.token_preferences);
                events.push(StateEvent::TokenPreferencesChanged { symbol: None });
            }
        }
        _ => unreachable!("routed by reduce_state_in_place"),
    }
}
