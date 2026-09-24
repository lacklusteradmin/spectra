//! The records and enums `WalletService` hands across the FFI.
//!
//! Shared answer snapshots live here. Durable transaction artifacts and their
//! typed protocol content live in [`crate::send::stages`].

use super::*;

/// Everything the wallet list implies, with holdings already resolved.
#[derive(Debug, Clone, serde::Serialize, uniffi::Record)]
pub struct WalletDerivedState {
    pub included_portfolio_holdings: Vec<crate::store::wallet_domain::AssetHolding>,
    pub unique_price_request_coins: Vec<crate::store::wallet_domain::AssetHolding>,
    /// One entry per asset, amounts summed across wallets.
    pub portfolio: Vec<crate::store::wallet_domain::AssetHolding>,
    pub send_coins_by_wallet_id: HashMap<String, Vec<crate::store::wallet_domain::AssetHolding>>,
    pub receive_coins_by_wallet_id: HashMap<String, Vec<crate::store::wallet_domain::AssetHolding>>,
    pub send_enabled_wallet_ids: Vec<String>,
    pub receive_enabled_wallet_ids: Vec<String>,
}

/// Token descriptor passed across UniFFI without JSON-shuttle marshalling.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenDescriptor {
    pub contract: String,
    pub symbol: String,
    pub decimals: u8,
    pub name: Option<String>,
}

/// Typed token-balance result returned via UniFFI.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenBalanceResult {
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

/// Unified per-chain native balance projection used by `fetch_native_balance_summary`.
/// `smallest_unit` is a base-10 integer string (sats, lamports, wei, yocto-NEAR, …);
/// `amount_display` is the chain's human-readable native amount.
#[derive(serde::Serialize, Debug, Clone, uniffi::Record)]
pub struct NativeBalanceSummary {
    pub smallest_unit: String,
    pub amount_display: String,
    pub utxo_count: u32,
}

/// What a send destination looks like, for the composer's recipient warning.
///
/// Core classifies activity; front ends supply localized wording.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum SendDestinationActivity {
    Unused,
    EmptyPreviouslyUsed,
    Funded,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SendDestinationRisk {
    pub balance_is_zero: bool,
    pub has_history: bool,
    pub activity: SendDestinationActivity,
}

impl SendDestinationRisk {
    pub(crate) fn from_probe(balance_is_zero: bool, has_history: bool) -> Self {
        let activity = if !balance_is_zero {
            SendDestinationActivity::Funded
        } else if has_history {
            SendDestinationActivity::EmptyPreviouslyUsed
        } else {
            SendDestinationActivity::Unused
        };
        Self {
            balance_is_zero,
            has_history,
            activity,
        }
    }
}

/// The address a send is actually going to, from what the user typed.
///
/// `used_ens` is not decoration: it is one of the high-risk signals, and it is
/// the difference between showing the user the string they typed and showing
/// them the address it stood for. A caller that resolved the name itself had
/// to remember to say so; one that asks for a destination is told.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SendDestinationResolution {
    /// Normalized for the chain, so it compares equal to a stored address.
    pub address: String,
    /// A name lookup, rather than the user, produced `address`.
    pub used_ens: bool,
}

/// The answer to a seed-phrase reveal.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SeedPhraseReveal {
    Phrase {
        phrase: String,
    },
    /// The wallet stores no phrase: watch-only, or a private key.
    NotStored,
    /// The phrase is sealed and no password was given.
    PasswordRequired,
    IncorrectPassword,
    /// A password was given for a phrase that has none.
    PasswordNotRequired,
}

/// One endpoint and whether it answered.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EndpointProbe {
    pub api: Option<crate::EndpointApi>,
    pub chain_id: String,
    pub endpoint: String,
    /// Operations the endpoint declares.
    pub capabilities: Vec<String>,
    /// False when nothing knows how to probe this endpoint. Not a pass.
    pub checked: bool,
    pub reachable: bool,
    pub detail: String,
}

/// What a [`TransactionCommand`] changed, by id.
///
/// Deliberately not the resulting list: history is unbounded, so a command that
/// returned it would make every write cost the size of the whole store.
#[derive(Debug, Clone, Default, uniffi::Record)]
pub struct TransactionChange {
    pub added: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
}

impl TransactionChange {
    /// True when the store is unchanged, so a caller can skip re-reading.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }
}

/// Mutations of the transaction store.
///
/// `Upsert` covers recording a send, merging a fetched history page, and
/// updating a status — in every case the caller supplies the record it wants
/// stored, and core works out whether that is an addition or an update.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum TransactionCommand {
    Upsert {
        records: Vec<crate::store::persistence_models::CorePersistedTransactionRecord>,
    },
    /// Merge a freshly fetched page into what is already stored.
    ///
    /// Core reads its own records to merge against, so only the incoming page
    /// crosses the FFI. Previously a front end shipped its entire history over,
    /// received the merged list back, and diffed it — the whole history moved
    /// three times per refresh.
    /// Merge freshly fetched history for a chain into what is stored.
    ///
    /// The merge strategy is not a parameter: it is a property of the chain and
    /// comes from `registry::Chain`. Callers used to pass it, which is how a
    /// chain could be wired to the wrong one.
    Merge {
        incoming: Vec<crate::fetch::transactions::CoreTransactionRecord>,
        chain_id: String,
        preserve_created_at_sentinel_unix: Option<f64>,
    },
    Remove {
        ids: Vec<String>,
    },
    RemoveForWallet {
        wallet_id: String,
    },
    Clear,
}

/// Endpoint configuration passed in from Swift at construction time and
/// rebuilt via `update_endpoints`.
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
pub struct ChainEndpoints {
    /// Declared operations for explicit URLs absent from the directory.
    pub capabilities: Vec<String>,
    pub chain_id: String,
    pub endpoints: Vec<String>,
}

// Durable transaction lifecycle records live in `crate::send::stages`.

#[cfg(test)]
mod destination_activity_tests {
    use super::*;
    #[test]
    fn activity_distinguishes_an_unused_address_from_a_previously_emptied_one() {
        for (balance, history, expected) in [
            (true, false, SendDestinationActivity::Unused),
            (true, true, SendDestinationActivity::EmptyPreviouslyUsed),
            (false, false, SendDestinationActivity::Funded),
            (false, true, SendDestinationActivity::Funded),
        ] {
            assert_eq!(
                SendDestinationRisk::from_probe(balance, history).activity,
                expected
            );
        }
    }
}
