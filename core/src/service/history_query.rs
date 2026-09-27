//! Bounded history queries and the small summary used outside the history screen.
use super::*;
use crate::store::persistence_models::CorePersistedTransactionRecord;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum HistoryQueryFilter {
    #[default]
    All,
    Send,
    Receive,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    pub wallet_id: Option<String>,
    pub filter: HistoryQueryFilter,
    pub search: String,
    pub oldest_first: bool,
    pub cursor: Option<String>,
    pub limit: u32,
}
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            wallet_id: None,
            filter: HistoryQueryFilter::All,
            search: String::new(),
            oldest_first: false,
            cursor: None,
            limit: 20,
        }
    }
}
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub records: Vec<CorePersistedTransactionRecord>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}
#[derive(Debug, Clone, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSnapshot {
    pub revision: u64,
    pub recent_and_pending: Vec<CorePersistedTransactionRecord>,
    pub replaceable: Vec<super::history_derived::ReplaceableSend>,
    pub earliest: Vec<crate::store::WalletEarliestTransactionDate>,
    pub total_count: u64,
    /// Wallets whose chain history has pages not yet fetched. A front end
    /// offers "load more" for these and asks nothing per wallet.
    pub wallets_with_more_history: Vec<String>,
}

/// One end of a stored transfer, whether it is the transacting wallet's own
/// address, and who holds it when Spectra knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEndpoint {
    pub address: String,
    /// The transacting wallet's own address. Another wallet of the user's is
    /// not "mine" here; it is named by `holder`.
    pub is_mine: bool,
    pub holder: Option<EndpointHolder>,
}

/// Who holds an address, as far as Spectra knows: one of the user's wallets
/// on the same network, or a saved contact for that network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, uniffi::Enum)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum EndpointHolder {
    Wallet { name: String },
    Contact { name: String },
}

/// A holder and the addresses that name it, as `holder_of` looks them up.
/// Earlier entries win.
pub(crate) struct KnownHolder {
    pub holder: EndpointHolder,
    pub addresses: Vec<String>,
}

/// The two ends of a stored transfer as the detail sheet shows them. An end
/// the record does not name, or that would repeat the other end, is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionEndpoints {
    pub from: Option<TransactionEndpoint>,
    pub to: Option<TransactionEndpoint>,
}

/// Which ends of `record` to show, judged against the addresses the wallet is
/// known to hold, and who holds each end — the first of `known` to list it.
/// Addresses compare in the chain's own normal form.
pub(crate) fn transaction_endpoints_for(
    record: &CorePersistedTransactionRecord,
    owned: &[String],
    known: &[KnownHolder],
) -> TransactionEndpoints {
    let normalize = |value: &str| crate::send::flow::normalize_address(&record.chain_id, value);
    let owned: std::collections::HashSet<String> = owned.iter().map(|a| normalize(a)).collect();
    let holder_of = |value: &str| holder_of(&record.chain_id, value, known);
    let named = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let is_mine = |value: &str| owned.contains(&normalize(value));
    let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(a), Some(b)) => normalize(a) == normalize(b),
        (None, None) => true,
        _ => false,
    };
    let source = named(&record.source_address);
    let counterparty = named(&Some(record.address.clone()));
    let wallet_side = source
        .clone()
        .filter(|a| is_mine(a))
        .or_else(|| counterparty.clone().filter(|a| is_mine(a)));
    let (from, to) = match record.kind {
        crate::store::wallet_domain::CoreTransactionKind::Send => {
            let to = counterparty.filter(|c| !same(&Some(c.clone()), &source));
            (source, to)
        }
        crate::store::wallet_domain::CoreTransactionKind::Receive => {
            let from = counterparty.filter(|c| !same(&Some(c.clone()), &wallet_side));
            (from, wallet_side)
        }
    };
    let endpoint = |address: String| TransactionEndpoint {
        is_mine: is_mine(&address),
        holder: holder_of(&address),
        address,
    };
    TransactionEndpoints {
        from: from.map(endpoint),
        to: to.map(endpoint),
    }
}

/// The first of `known` to list `address` on `chain_id`, compared in the
/// chain's own normal form.
pub(crate) fn holder_of(
    chain_id: &str,
    address: &str,
    known: &[KnownHolder],
) -> Option<EndpointHolder> {
    let normalize = |value: &str| crate::send::flow::normalize_address(chain_id, value);
    let address = normalize(address);
    known
        .iter()
        .find(|k| k.addresses.iter().any(|a| normalize(a) == address))
        .map(|k| k.holder.clone())
}

#[uniffi::export(async_runtime = "tokio")]
impl WalletService {
    /// Who holds `address` on `chain_id`, as seen from the wallet `wallet_id`:
    /// that wallet, another of the user's wallets on the network, or a saved
    /// contact — the same answer a stored transfer's ends get, for an address
    /// that has not been sent to yet.
    pub async fn address_holder(
        &self,
        wallet_id: String,
        chain_id: String,
        address: String,
    ) -> Result<Option<EndpointHolder>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let owned = this.known_wallet_addresses(wallet_id.clone()).await?;
            let known = this
                .known_holders(Some(&wallet_id), None, &chain_id, &owned)
                .await;
            Ok(holder_of(&chain_id, &address, &known))
        })
        .await
    }

    /// The ends of a stored transfer and which of them belong to the wallet.
    pub async fn transaction_endpoints(
        &self,
        transaction_id: String,
    ) -> Result<Option<TransactionEndpoints>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let Some(record) = this.transaction(transaction_id).await? else {
                return Ok(None);
            };
            let owned = match &record.wallet_id {
                Some(wallet_id) => this.known_wallet_addresses(wallet_id.clone()).await?,
                None => Vec::new(),
            };
            let known = this
                .known_holders(
                    record.wallet_id.as_deref(),
                    Some(&record.wallet_name),
                    &record.chain_id,
                    &owned,
                )
                .await;
            Ok(Some(transaction_endpoints_for(&record, &owned, &known)))
        })
        .await
    }

    pub async fn history_page(
        &self,
        query: HistoryQuery,
    ) -> Result<HistoryPage, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            if query.limit == 0 || query.limit > 200 {
                return Err("history query limit must be 1...200".into());
            }
            let database = this.bound_database().await?;
            tokio::task::spawn_blocking(move || crate::wallet_db::history_page(&database, &query))
                .await
                .map_err(|e| SpectraBridgeError::from(e.to_string()))?
                .map(|mut page| {
                    page.records = page
                        .records
                        .into_iter()
                        .map(CorePersistedTransactionRecord::with_actions)
                        .collect();
                    page
                })
                .map_err(Into::into)
        })
        .await
    }
    pub async fn transaction_snapshot(&self) -> Result<TransactionSnapshot, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let database = this.bound_database().await?;
            let more = this.wallets_with_more_history_now().await;
            let sequence = this.projection_sequence.clone();
            tokio::task::spawn_blocking(move || {
                crate::wallet_db::history_snapshot(&database, &sequence)
            })
            .await
            .map_err(|e| SpectraBridgeError::from(e.to_string()))?
            .map(|mut snapshot| {
                snapshot.recent_and_pending = snapshot
                    .recent_and_pending
                    .into_iter()
                    .map(CorePersistedTransactionRecord::with_actions)
                    .collect();
                snapshot
            })
            .map_err(SpectraBridgeError::from)
            .map(|mut snapshot| {
                snapshot.wallets_with_more_history = more;
                snapshot
            })
        })
        .await
    }
    pub async fn transaction(
        &self,
        id: String,
    ) -> Result<Option<CorePersistedTransactionRecord>, SpectraBridgeError> {
        let this = self.clone();
        crate::worker::run(async move {
            let this = &this;
            let database = this.bound_database().await?;
            tokio::task::spawn_blocking(move || crate::wallet_db::history_find(&database, &id))
                .await
                .map_err(|e| SpectraBridgeError::from(e.to_string()))?
                .map(|record| record.map(CorePersistedTransactionRecord::with_actions))
                .map_err(Into::into)
        })
        .await
    }
}

// Not exported: the lookup behind `transaction_endpoints` and `address_holder`.
impl WalletService {
    /// Who could hold an address on `chain_id`, most specific first: the
    /// wallet `wallet_id` (holding `owned`), the user's other wallets on that
    /// network, then the address book's contacts for it. Another network's
    /// wallet is left out even when it shares the address, because the
    /// question is about this network. A wallet since deleted is still named
    /// by `recorded_name`, the name a stored transfer kept.
    async fn known_holders(
        &self,
        wallet_id: Option<&str>,
        recorded_name: Option<&str>,
        chain_id: &str,
        owned: &[String],
    ) -> Vec<KnownHolder> {
        let (own_name, others, contacts) = {
            let state = self.wallet_state.read().await;
            let own = wallet_id.and_then(|id| {
                state
                    .wallets
                    .iter()
                    .find(|w| w.id == id)
                    .map(|w| w.name.clone())
                    .or_else(|| recorded_name.map(str::to_string))
            });
            let others: Vec<(String, String, Vec<String>)> = state
                .wallets
                .iter()
                .filter(|w| Some(w.id.as_str()) != wallet_id && w.chain_id == chain_id)
                .map(|w| {
                    let addresses = w.addresses.iter().map(|a| a.address.clone()).collect();
                    (w.id.clone(), w.name.clone(), addresses)
                })
                .collect();
            let contacts: Vec<KnownHolder> = state
                .address_book
                .iter()
                .filter(|entry| entry.chain_id == chain_id)
                .map(|entry| KnownHolder {
                    holder: EndpointHolder::Contact {
                        name: entry.name.clone(),
                    },
                    addresses: vec![entry.address.clone()],
                })
                .collect();
            (own, others, contacts)
        };
        let mut known = Vec::new();
        if let Some(name) = own_name {
            known.push(KnownHolder {
                holder: EndpointHolder::Wallet { name },
                addresses: owned.to_vec(),
            });
        }
        for (id, name, mut addresses) in others {
            addresses.extend(
                self.owned_addresses_for_wallet(id, Some(chain_id.to_string()))
                    .await,
            );
            known.push(KnownHolder {
                holder: EndpointHolder::Wallet { name },
                addresses,
            });
        }
        known.extend(contacts);
        known
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    fn record(kind: &str, address: &str, source: Option<&str>) -> CorePersistedTransactionRecord {
        serde_json::from_value(serde_json::json!({
            "id": "t", "walletId": "w", "kind": kind, "status": "confirmed",
            "walletName": "W", "assetDisplayName": "Ether", "symbol": "ETH",
            "chainId": "ethereum", "amount": "1", "address": address,
            "sourceAddress": source, "createdAtUnix": 1.0
        }))
        .unwrap()
    }

    const MINE: &str = "0x1111111111111111111111111111111111111111";
    const THEIRS: &str = "0x2222222222222222222222222222222222222222";

    fn wallet(name: &str, addresses: &[&str]) -> KnownHolder {
        KnownHolder {
            holder: EndpointHolder::Wallet { name: name.into() },
            addresses: addresses.iter().map(|a| a.to_string()).collect(),
        }
    }
    fn contact(name: &str, address: &str) -> KnownHolder {
        KnownHolder {
            holder: EndpointHolder::Contact { name: name.into() },
            addresses: vec![address.into()],
        }
    }

    #[test]
    fn a_send_runs_from_the_source_to_the_counterparty() {
        let ends =
            transaction_endpoints_for(&record("send", THEIRS, Some(MINE)), &[MINE.into()], &[]);
        assert_eq!(
            ends.from,
            Some(TransactionEndpoint {
                address: MINE.into(),
                is_mine: true,
                holder: None,
            })
        );
        assert_eq!(
            ends.to,
            Some(TransactionEndpoint {
                address: THEIRS.into(),
                is_mine: false,
                holder: None,
            })
        );
    }

    /// Ownership compares normal forms: a checksummed EVM address is the same
    /// address as its lowercase spelling.
    #[test]
    fn a_receive_ends_at_the_wallet_in_any_case() {
        let ends = transaction_endpoints_for(
            &record(
                "receive",
                &MINE.to_uppercase().replacen("0X", "0x", 1),
                None,
            ),
            &[MINE.into()],
            &[],
        );
        assert_eq!(ends.from, None, "the counterparty is the wallet itself");
        assert!(ends.to.is_some_and(|to| to.is_mine));
    }

    #[test]
    fn a_self_send_names_its_address_once() {
        let ends =
            transaction_endpoints_for(&record("send", MINE, Some(MINE)), &[MINE.into()], &[]);
        assert!(ends.from.is_some());
        assert_eq!(ends.to, None);
    }

    /// Each end is named by the first holder that lists it, in normal form:
    /// the transacting wallet, another of the user's wallets, a contact.
    #[test]
    fn each_end_is_named_by_its_holder() {
        let known = [
            wallet("Main", &[MINE]),
            wallet("Savings", &[&THEIRS.to_uppercase().replacen("0X", "0x", 1)]),
            contact("Alice", THEIRS),
        ];
        let ends =
            transaction_endpoints_for(&record("send", THEIRS, Some(MINE)), &[MINE.into()], &known);
        let from = ends.from.expect("from");
        let to = ends.to.expect("to");
        assert_eq!(
            from.holder,
            Some(EndpointHolder::Wallet {
                name: "Main".into()
            })
        );
        assert_eq!(
            to.holder,
            Some(EndpointHolder::Wallet {
                name: "Savings".into()
            }),
            "a wallet outranks a contact"
        );
        assert!(
            !to.is_mine,
            "another wallet of the user's is not the transacting wallet"
        );
    }

    #[test]
    fn a_contact_names_an_address_no_wallet_holds() {
        let known = [wallet("Main", &[MINE]), contact("Alice", THEIRS)];
        let ends =
            transaction_endpoints_for(&record("send", THEIRS, Some(MINE)), &[MINE.into()], &known);
        assert_eq!(
            ends.to.and_then(|to| to.holder),
            Some(EndpointHolder::Contact {
                name: "Alice".into()
            })
        );
    }

    #[test]
    fn an_unknown_address_has_no_holder() {
        let known = [wallet("Main", &[MINE])];
        let ends =
            transaction_endpoints_for(&record("send", THEIRS, Some(MINE)), &[MINE.into()], &known);
        assert_eq!(ends.to.and_then(|to| to.holder), None);
    }

    /// Serialized for the CLI and JSON front ends with its kind spelled out.
    #[test]
    fn a_holder_serializes_with_its_kind() {
        let json = serde_json::to_value(EndpointHolder::Contact {
            name: "Alice".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "contact", "name": "Alice"})
        );
    }
}
