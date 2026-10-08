//! The Stellar Horizon adapter: accounts, payments history, base fee and
//! envelope submission.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};

use crate::api::http::{HttpClient, RetryProfile, race};

/// SEP-29: an account that needs a memo says so in its `config.memo_required`
/// data entry, whose value is "1" (base64 `MQ==`).
fn memo_required(data: &std::collections::HashMap<String, String>) -> bool {
    data.get("config.memo_required")
        .is_some_and(|value| value == "MQ==")
}

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarBalance {
    /// Stroops (1 XLM = 10_000_000 stroops).
    pub stroops: i64,
    pub xlm_display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarHistoryEntry {
    pub txid: String,
    pub ledger: u64,
    /// Unix seconds, from Horizon's RFC 3339 `created_at`.
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_stroops: i64,
    pub fee_charged: u64,
    pub is_incoming: bool,
    /// A credit asset's `CODE:ISSUER`; absent for XLM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// A credit asset's amount, an exact decimal; absent for XLM.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StellarSendResult {
    pub txid: String,
    /// Base64-encoded signed XDR envelope — stored for rebroadcast.
    pub signed_xdr_b64: String,
}

// ── Horizon API response types

/// A Stellar account's reserve inputs, in stroops and counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarReserveState {
    pub exists: bool,
    pub balance_stroops: u64,
    pub subentries: u64,
    pub sponsoring: u64,
    pub sponsored: u64,
    pub base_reserve: u64,
}

/// What merging an account depends on, from one node's latest ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarMergeState {
    /// `None` when the account does not exist.
    pub source: Option<StellarMergeableAccount>,
    /// Whether the destination asks for a memo (SEP-29); `None` when it
    /// does not exist.
    pub destination_memo_required: Option<bool>,
    pub ledger: u32,
    pub base_reserve: u64,
}

/// The account being merged, as the latest ledger holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarMergeableAccount {
    pub balance_stroops: u64,
    pub sequence: u64,
    pub subentries: u64,
    pub sponsoring: u64,
    pub sponsored: u64,
    pub auth_immutable: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonAccount {
    pub(crate) balances: Vec<HorizonBalance>,
    pub(crate) sequence: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonBalance {
    pub(crate) balance: String,
    pub(crate) asset_type: String,
    #[serde(default)]
    pub(crate) asset_code: String,
    #[serde(default)]
    pub(crate) asset_issuer: String,
    #[serde(default)]
    pub(crate) limit: String,
    #[serde(default)]
    pub(crate) buying_liabilities: String,
    #[serde(default)]
    pub(crate) selling_liabilities: String,
    #[serde(default)]
    pub(crate) is_authorized: bool,
}

/// A Stellar account's trustline to one credit asset, in stroops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarTrustline {
    pub asset: crate::api::stellar_asset::StellarAsset,
    pub balance: i64,
    pub limit: i64,
    /// Committed to open offers buying the asset: room the line keeps.
    pub buying_liabilities: i64,
    /// Committed to open offers selling it: not spendable.
    pub selling_liabilities: i64,
    /// The issuer lets the account hold and move the asset.
    pub authorized: bool,
}

/// An account as a credit-asset payment or trustline depends on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarAccountState {
    pub native_stroops: i64,
    pub native_selling_liabilities: i64,
    pub sequence: u64,
    pub subentries: u64,
    pub sponsoring: u64,
    pub sponsored: u64,
    pub trustlines: Vec<StellarTrustline>,
}

impl StellarAccountState {
    pub fn trustline(
        &self,
        asset: &crate::api::stellar_asset::StellarAsset,
    ) -> Option<&StellarTrustline> {
        self.trustlines.iter().find(|line| line.asset == *asset)
    }

    /// The XLM the account must keep: two base reserves, and one for each
    /// subentry and sponsorship it pays for.
    pub fn minimum_balance(&self, base_reserve: u64) -> u64 {
        (2 + self.subentries + self.sponsoring).saturating_sub(self.sponsored) * base_reserve
    }
}

/// What a credit-asset payment or trustline depends on, from one verified
/// node's latest ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StellarAssetState {
    /// `None` when the account does not exist.
    pub holder: Option<StellarAccountState>,
    /// The destination, when one was asked about; `None` inside when it does
    /// not exist.
    pub destination: Option<Option<StellarAccountState>>,
    pub issuer_exists: bool,
    pub base_reserve: u64,
}

#[derive(Debug, Deserialize)]
struct HorizonAccountState {
    balances: Vec<HorizonBalance>,
    sequence: String,
    subentry_count: u64,
    #[serde(default)]
    num_sponsoring: u64,
    #[serde(default)]
    num_sponsored: u64,
}

impl HorizonAccountState {
    fn state(self) -> Result<StellarAccountState, ApiError> {
        let native = self
            .balances
            .iter()
            .find(|balance| balance.asset_type == "native")
            .or_decode("no native balance")?;
        let liabilities = |text: &str| {
            if text.is_empty() {
                Ok(0)
            } else {
                parse_stellar_amount(text)
            }
        };
        let mut trustlines = Vec::new();
        for balance in &self.balances {
            if !matches!(
                balance.asset_type.as_str(),
                "credit_alphanum4" | "credit_alphanum12"
            ) {
                continue;
            }
            let asset = crate::api::stellar_asset::StellarAsset::new(
                &balance.asset_code,
                &balance.asset_issuer,
            )
            .map_err(|_| ApiError::decode("Horizon: invalid credit asset"))?;
            if asset.asset_type() != balance.asset_type {
                return Err(ApiError::decode(
                    "Horizon: asset type does not match its code",
                ));
            }
            trustlines.push(StellarTrustline {
                asset,
                balance: parse_stellar_amount(&balance.balance)?,
                limit: parse_stellar_amount(&balance.limit)?,
                buying_liabilities: liabilities(&balance.buying_liabilities)?,
                selling_liabilities: liabilities(&balance.selling_liabilities)?,
                authorized: balance.is_authorized,
            });
        }
        Ok(StellarAccountState {
            native_stroops: parse_stellar_amount(&native.balance)?,
            native_selling_liabilities: liabilities(&native.selling_liabilities)?,
            sequence: self
                .sequence
                .parse()
                .map_err(|e| ApiError::Decode(format!("sequence parse: {e}")))?,
            subentries: self.subentry_count,
            sponsoring: self.num_sponsoring,
            sponsored: self.num_sponsored,
            trustlines,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonFeeStats {
    pub(crate) fee_charged: HorizonFeeCharged,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonFeeCharged {
    pub(crate) mode: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonPayments {
    #[serde(rename = "_embedded")]
    pub(crate) embedded: HorizonPaymentsEmbedded,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonPaymentsEmbedded {
    pub(crate) records: Vec<HorizonPaymentRecord>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HorizonPaymentRecord {
    #[serde(default)]
    pub(crate) paging_token: String,
    #[serde(rename = "type")]
    pub(crate) op_type: String,
    #[serde(default)]
    pub(crate) from: String,
    #[serde(default)]
    pub(crate) to: String,
    #[serde(default)]
    pub(crate) amount: String,
    /// `native` for XLM; a payment of an issued asset names its own.
    #[serde(default)]
    pub(crate) asset_type: String,
    #[serde(default)]
    pub(crate) asset_code: String,
    #[serde(default)]
    pub(crate) asset_issuer: String,
    /// `create_account` names its ends and amount differently.
    #[serde(default)]
    pub(crate) funder: String,
    #[serde(default)]
    pub(crate) account: String,
    #[serde(default)]
    pub(crate) starting_balance: String,
    pub(crate) created_at: String,
    pub(crate) transaction_hash: String,
}

// ── Client

pub struct HorizonClient {
    pub(crate) endpoints: std::sync::Arc<Vec<String>>,
    pub(crate) client: std::sync::Arc<HttpClient>,
}

impl HorizonClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ApiError> {
        self.client.get_path(&self.endpoints, path).await
    }
}
// Stellar fetch paths (Horizon): native balance, per-asset balance, sequence,
// base fee, and payments history.

impl HorizonClient {
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let root: serde_json::Value = self.get("/").await?;
        if root
            .get("network_passphrase")
            .and_then(serde_json::Value::as_str)
            != Some(
                chain
                    .stellar_network_passphrase()
                    .map_err(ApiError::invalid)?,
            )
        {
            return Err(ApiError::invalid(
                "Horizon endpoint is on the wrong network",
            ));
        }
        Ok(())
    }

    /// Horizon's hash resource includes failed transactions, unlike payments history.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        use crate::api::transaction_status::TransactionStatus;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::invalid("Invalid Stellar transaction hash"));
        }
        let result: serde_json::Value = match self.get(&format!("/transactions/{hash}")).await {
            Ok(result) => result,
            Err(ApiError::Status { status: 404, .. }) => return Ok(TransactionStatus::Pending),
            Err(error) => return Err(error),
        };
        let actual = result
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .or_decode("Horizon transaction: missing hash")?;
        if !actual.eq_ignore_ascii_case(hash) {
            return Err(ApiError::decode("Horizon returned a different transaction"));
        }
        let ledger = result
            .get("ledger")
            .and_then(serde_json::Value::as_u64)
            .filter(|number| *number > 0)
            .or_decode("Horizon transaction: missing ledger")?;
        let succeeded = result
            .get("successful")
            .and_then(serde_json::Value::as_bool)
            .or_decode("Horizon transaction: missing execution result")?;
        Ok(TransactionStatus::Confirmed {
            succeeded,
            block: Some(ledger),
        })
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<StellarBalance, ApiError> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        let native = account
            .balances
            .iter()
            .find(|b| b.asset_type == "native")
            .or_decode("no native balance")?;
        // Stellar balances are decimal strings (e.g. "100.0000000")
        let stroops = parse_stellar_amount(&native.balance)?;
        Ok(StellarBalance {
            stroops,
            xlm_display: native.balance.clone(),
        })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u64, ApiError> {
        let account: HorizonAccount = self.get(&format!("/accounts/{address}")).await?;
        account
            .sequence
            .parse::<u64>()
            .map_err(|e| ApiError::Decode(format!("sequence parse: {e}")))
    }

    /// The latest ledger's base reserve, in stroops. A new account holds two
    /// of them: one for itself and one more, which is the network minimum.
    pub(crate) async fn fetch_base_reserve(&self) -> Result<u64, ApiError> {
        let page: serde_json::Value = self.get("/ledgers?order=desc&limit=1").await?;
        page.pointer("/_embedded/records/0/base_reserve_in_stroops")
            .and_then(serde_json::Value::as_u64)
            .or_decode("ledgers: missing base_reserve_in_stroops")
    }

    /// An account's reserve as the latest ledger holds it, from one verified
    /// node: whether it exists, its native balance, the subentries and
    /// sponsorships its minimum balance counts, and the base reserve.
    pub(crate) async fn fetch_reserve_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<StellarReserveState, ApiError> {
        #[derive(Deserialize)]
        struct Account {
            balances: Vec<HorizonBalance>,
            subentry_count: u64,
            #[serde(default)]
            num_sponsoring: u64,
            #[serde(default)]
            num_sponsored: u64,
        }
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let base_reserve = node.fetch_base_reserve().await?;
            let account = match node.get::<Account>(&format!("/accounts/{address}")).await {
                Err(ApiError::Status { status: 404, .. }) => None,
                read => Some(read?),
            };
            Ok(match account {
                None => StellarReserveState {
                    exists: false,
                    balance_stroops: 0,
                    subentries: 0,
                    sponsoring: 0,
                    sponsored: 0,
                    base_reserve,
                },
                Some(account) => StellarReserveState {
                    exists: true,
                    balance_stroops: u64::try_from(parse_stellar_amount(
                        &account
                            .balances
                            .iter()
                            .find(|balance| balance.asset_type == "native")
                            .or_decode("no native balance")?
                            .balance,
                    )?)
                    .map_err(|_| ApiError::decode("negative native balance"))?,
                    subentries: account.subentry_count,
                    sponsoring: account.num_sponsoring,
                    sponsored: account.num_sponsored,
                    base_reserve,
                },
            })
        })
        .await
    }

    /// What merging `address` into `destination` depends on, from one
    /// verified node: both accounts, the latest ledger and its base reserve.
    pub(crate) async fn fetch_merge_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
        destination: &str,
    ) -> Result<StellarMergeState, ApiError> {
        #[derive(Deserialize)]
        struct Flags {
            #[serde(default)]
            auth_immutable: bool,
        }
        #[derive(Deserialize)]
        struct Account {
            balances: Vec<HorizonBalance>,
            sequence: String,
            subentry_count: u64,
            #[serde(default)]
            num_sponsoring: u64,
            #[serde(default)]
            num_sponsored: u64,
            flags: Flags,
            #[serde(default)]
            data: std::collections::HashMap<String, String>,
        }
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let account = |address: String| {
                let node = &node;
                async move {
                    match node.get::<Account>(&format!("/accounts/{address}")).await {
                        Err(ApiError::Status { status: 404, .. }) => Ok(None),
                        read => read.map(Some),
                    }
                }
            };
            let ledger: serde_json::Value = node.get("/ledgers?order=desc&limit=1").await?;
            let latest = |field: &str| {
                ledger
                    .pointer(&format!("/_embedded/records/0/{field}"))
                    .and_then(serde_json::Value::as_u64)
                    .or_decode("ledgers: missing latest ledger")
            };
            let source = match account(address.to_string()).await? {
                None => None,
                Some(account) => Some(StellarMergeableAccount {
                    balance_stroops: u64::try_from(parse_stellar_amount(
                        &account
                            .balances
                            .iter()
                            .find(|balance| balance.asset_type == "native")
                            .or_decode("no native balance")?
                            .balance,
                    )?)
                    .map_err(|_| ApiError::decode("negative native balance"))?,
                    sequence: account
                        .sequence
                        .parse()
                        .map_err(|e| ApiError::Decode(format!("sequence parse: {e}")))?,
                    subentries: account.subentry_count,
                    sponsoring: account.num_sponsoring,
                    sponsored: account.num_sponsored,
                    auth_immutable: account.flags.auth_immutable,
                }),
            };
            let destination_memo_required = account(destination.to_string())
                .await?
                .map(|account| memo_required(&account.data));
            Ok(StellarMergeState {
                source,
                destination_memo_required,
                ledger: u32::try_from(latest("sequence")?)
                    .map_err(|_| ApiError::decode("ledgers: sequence out of range"))?,
                base_reserve: latest("base_reserve_in_stroops")?,
            })
        })
        .await
    }

    /// Whether payments to `address` must carry a memo (SEP-29), from a
    /// node verified to be on `chain`. An account that does not exist asks
    /// for none.
    pub(crate) async fn fetch_memo_required(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<bool, ApiError> {
        #[derive(Deserialize)]
        struct Account {
            #[serde(default)]
            data: std::collections::HashMap<String, String>,
        }
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            match node.get::<Account>(&format!("/accounts/{address}")).await {
                Err(ApiError::Status { status: 404, .. }) => Ok(false),
                read => Ok(memo_required(&read?.data)),
            }
        })
        .await
    }

    /// An account's balances, trustlines and reserve counts, or `None` when
    /// it does not exist.
    pub(crate) async fn fetch_account_state(
        &self,
        address: &str,
    ) -> Result<Option<StellarAccountState>, ApiError> {
        match self
            .get::<HorizonAccountState>(&format!("/accounts/{address}"))
            .await
        {
            Err(ApiError::Status { status: 404, .. }) => Ok(None),
            read => read?.state().map(Some),
        }
    }

    /// What paying `asset` from `holder`, or trusting it, depends on: the
    /// holder, the destination, whether the issuer exists, and the base
    /// reserve, all from one verified node.
    pub(crate) async fn fetch_asset_state(
        &self,
        chain: crate::registry::Chain,
        holder: &str,
        asset: &crate::api::stellar_asset::StellarAsset,
        destination: Option<&str>,
    ) -> Result<StellarAssetState, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint]));
            node.verify_network(chain).await?;
            let base_reserve = node.fetch_base_reserve().await?;
            let destination = match destination {
                None => None,
                Some(destination) => Some(node.fetch_account_state(destination).await?),
            };
            Ok(StellarAssetState {
                holder: node.fetch_account_state(holder).await?,
                destination,
                issuer_exists: node.fetch_account_state(&asset.issuer).await?.is_some(),
                base_reserve,
            })
        })
        .await
    }

    pub async fn fetch_base_fee(&self) -> Result<u64, ApiError> {
        let stats: HorizonFeeStats = self.get("/fee_stats").await?;
        Ok(stats.fee_charged.mode.parse::<u64>().unwrap_or(100))
    }

    pub async fn fetch_history(&self, address: &str) -> Result<Vec<StellarHistoryEntry>, ApiError> {
        Ok(self.fetch_history_page(address, None).await?.items)
    }

    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<StellarHistoryEntry>, ApiError> {
        let continuation = cursor
            .map(|value| format!("&cursor={}", crate::api::history_page::query_value(value)))
            .unwrap_or_default();
        let payments: HorizonPayments = self
            .get(&format!(
                "/accounts/{address}/payments?limit=50&order=desc&include_failed=false{continuation}"
            ))
            .await?;
        let records = payments.embedded.records;
        let next_cursor = if records.len() == 50 {
            Some(
                records
                    .last()
                    .map(|row| row.paging_token.clone())
                    .filter(|token| !token.is_empty())
                    .or_decode("Horizon: full history page has no paging token")?,
            )
        } else {
            None
        };
        Ok(crate::api::HistoryPage {
            items: stellar_history_from_payments(records, address)?,
            next_cursor,
        })
    }
}

/// What each payment record moved for `address`: XLM, or a credit asset
/// named by its `CODE:ISSUER`.
///
/// A `create_account` record carries its ends and amount as `funder`,
/// `account` and `starting_balance`. A payment of a credit asset is that
/// asset's row, never XLM's.
fn stellar_history_from_payments(
    records: Vec<HorizonPaymentRecord>,
    address: &str,
) -> Result<Vec<StellarHistoryEntry>, ApiError> {
    let entries: Result<Vec<Option<StellarHistoryEntry>>, ApiError> = records
        .into_iter()
        .map(|r| {
            let contract = match r.asset_type.as_str() {
                "credit_alphanum4" | "credit_alphanum12" if r.op_type == "payment" => Some(
                    crate::api::stellar_asset::StellarAsset::new(&r.asset_code, &r.asset_issuer)
                        .map_err(|_| ApiError::decode("Horizon payment: invalid credit asset"))?
                        .identifier(),
                ),
                _ => None,
            };
            let (from, to, amount) = match r.op_type.as_str() {
                "payment" if r.asset_type == "native" || contract.is_some() => {
                    (r.from, r.to, r.amount)
                }
                "create_account" => (r.funder, r.account, r.starting_balance),
                _ => return Ok(None),
            };
            let amount_stroops = parse_stellar_amount(&amount)?;
            let amount_display = contract
                .is_some()
                .then(|| crate::decimal::from_units(u128::from(amount_stroops.unsigned_abs()), 7));
            // Horizon lists only operations already in a ledger.
            let timestamp = crate::api::time::confirmed_history_time(
                crate::api::time::parse_iso8601_timestamp(&r.created_at)
                    .filter(|t| *t > 0.0)
                    .map(|t| t as u64),
                &r.transaction_hash,
            )?;
            Ok(Some(StellarHistoryEntry {
                txid: r.transaction_hash,
                ledger: 0,
                timestamp,
                is_incoming: to == address,
                from,
                to,
                amount_stroops: if contract.is_some() {
                    0
                } else {
                    amount_stroops
                },
                fee_charged: 0,
                contract,
                amount_display,
            }))
        })
        .collect();
    Ok(entries?.into_iter().flatten().collect())
}

/// Horizon's seven-place amount ("100.0000000") in stroops, exactly: a
/// value with more places, outside an `i64`, or not a decimal is refused.
pub(crate) fn parse_stellar_amount(s: &str) -> Result<i64, ApiError> {
    let invalid = || ApiError::Decode(format!("amount parse: {s:?}"));
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let stroops = crate::decimal::to_units(digits, 7).ok_or_else(invalid)?;
    let stroops = i64::try_from(stroops).map_err(|_| invalid())?;
    Ok(if negative { -stroops } else { stroops })
}

impl HorizonClient {
    /// Submit a pre-signed XDR envelope (for rebroadcast).
    pub async fn submit_envelope_b64(&self, tx_b64: &str) -> Result<StellarSendResult, ApiError> {
        let tx_b64 = tx_b64.to_string();
        race(&self.endpoints, |base| {
            let client = self.client.clone();
            let tx_b64 = tx_b64.clone();
            let url = format!("{}/transactions", base.trim_end_matches('/'));
            async move {
                let resp: serde_json::Value = client
                    .post_json(
                        &url,
                        &serde_json::json!({"tx": tx_b64}),
                        RetryProfile::ChainWrite,
                    )
                    .await?;
                let hash = resp
                    .get("hash")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(StellarSendResult {
                    txid: hash,
                    signed_xdr_b64: tx_b64.clone(),
                })
            }
        })
        .await
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn amounts_are_exact_stroops_or_refused() {
        assert_eq!(parse_stellar_amount("100.0000000").unwrap(), 1_000_000_000);
        assert_eq!(parse_stellar_amount("0.0000001").unwrap(), 1);
        assert_eq!(parse_stellar_amount("-1.5").unwrap(), -15_000_000);
        assert_eq!(
            parse_stellar_amount("922337203685.4775807").unwrap(),
            i64::MAX
        );
        for bad in [
            "",
            "1.00000001",
            "922337203685.4775808",
            "1e5",
            "abc",
            "--1",
        ] {
            assert!(parse_stellar_amount(bad).is_err(), "{bad}");
        }
    }

    const ME: &str = "GA5XIGA5C7QTPTWXQHY6MCJRMTRZDOSHR6EFIBNDQTCQHG262N4GGKTM";
    const THEM: &str = "GBUXQE5RNV267EEVS6COJSHRKIE52GFVVA66TMM7UNAYLAOZP36PZ7YX";
    const CIRCLE: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

    /// Record shapes as Horizon's `/accounts/{id}/payments` returns them.
    #[test]
    fn create_account_and_issued_assets_are_read_by_their_own_fields() {
        let records: HorizonPaymentsEmbedded = serde_json::from_value(serde_json::json!({
            "records": [
                {"type": "create_account", "created_at": "2025-08-06T02:05:01Z",
                 "transaction_hash": "created", "starting_balance": "241.5703630",
                 "funder": THEM, "account": ME},
                {"type": "payment", "created_at": "2025-08-06T01:17:03Z",
                 "transaction_hash": "xlm", "asset_type": "native",
                 "from": ME, "to": THEM, "amount": "82.1781600"},
                {"type": "payment", "created_at": "2025-08-06T01:18:03Z",
                 "transaction_hash": "usdc", "asset_type": "credit_alphanum4",
                 "asset_code": "USDC", "asset_issuer": CIRCLE,
                 "from": THEM, "to": ME, "amount": "5000.0000000"}
            ]
        }))
        .unwrap();
        let entries = stellar_history_from_payments(records.records, ME).unwrap();
        assert_eq!(entries[0].timestamp, 1_754_445_901, "2025-08-06T02:05:01Z");
        assert_eq!(entries.len(), 3);
        // A credit asset is its own row, named by code and issuer.
        assert_eq!(entries[2].txid, "usdc");
        assert!(entries[2].is_incoming);
        assert_eq!(
            entries[2].contract.as_deref(),
            Some(format!("USDC:{CIRCLE}").as_str())
        );
        assert_eq!(entries[2].amount_display.as_deref(), Some("5000"));
        assert_eq!(entries[2].amount_stroops, 0);
        assert_eq!(entries[0].txid, "created");
        assert!(entries[0].is_incoming);
        assert_eq!(entries[0].from, THEM);
        assert_eq!(entries[0].amount_stroops, 2_415_703_630);
        assert_eq!(entries[1].txid, "xlm");
        assert!(!entries[1].is_incoming);
        assert_eq!(entries[1].amount_stroops, 821_781_600);
    }
}

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::{api::transaction_status::TransactionStatus, registry::Chain};
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn hash_resource_closes_success_and_failed_transactions_without_payment_history() {
        let hash = "ab".repeat(32);
        for succeeded in [true, false] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!("/transactions/{hash}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"hash":hash,"ledger":450,"successful":succeeded})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                HorizonClient::new(Arc::new(vec![server.uri()]))
                    .fetch_transaction_status(&hash)
                    .await
                    .unwrap(),
                TransactionStatus::Confirmed {
                    succeeded,
                    block: Some(450)
                }
            );
        }
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/transactions/{hash}")))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            HorizonClient::new(Arc::new(vec![server.uri()]))
                .fetch_transaction_status(&hash)
                .await
                .unwrap(),
            TransactionStatus::Pending
        );
        for bad in [
            json!({"hash":hash,"successful":true}),
            json!({"hash":hash,"ledger":450}),
            json!({"hash":"cd".repeat(32),"ledger":450,"successful":true}),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(bad))
                .mount(&server)
                .await;
            assert!(
                HorizonClient::new(Arc::new(vec![server.uri()]))
                    .fetch_transaction_status(&hash)
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn horizon_identity_matches_the_signing_network_passphrase() {
        for selected in [Chain::Stellar, Chain::StellarTestnet] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"network_passphrase":selected.stellar_network_passphrase().unwrap()}),
                ))
                .mount(&server)
                .await;
            let client = HorizonClient::new(Arc::new(vec![server.uri()]));
            client.verify_network(selected).await.unwrap();
            assert!(
                client
                    .verify_network(if selected == Chain::Stellar {
                        Chain::StellarTestnet
                    } else {
                        Chain::Stellar
                    })
                    .await
                    .is_err()
            );
        }
    }
}
