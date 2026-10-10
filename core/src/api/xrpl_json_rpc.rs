//! The XRP Ledger JSON-RPC adapter (rippled / Clio): account info and
//! sequence, fees, transaction history and blob submission.

use crate::api::error::{ApiError, OrDecode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::http::HttpClient;

// ── Public result types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpBalance {
    /// XRP drops (1 XRP = 1_000_000 drops).
    pub drops: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpHistoryEntry {
    pub txid: String,
    pub ledger_index: u64,
    pub timestamp: u64,
    pub from: String,
    pub to: String,
    pub amount_drops: u64,
    pub fee_drops: u64,
    pub is_incoming: bool,
    /// An issued currency's `CODE.rIssuer`; absent for XRP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// An issued currency's amount, an exact decimal; absent for XRP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XrpSendResult {
    pub txid: String,
    /// Signed tx blob hex — stored for rebroadcast.
    pub tx_blob_hex: String,
}

// ── Client

pub struct XrplClient {
    endpoints: std::sync::Arc<Vec<String>>,
    client: std::sync::Arc<HttpClient>,
}

/// An XRP account's reserve, in drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpReserveState {
    pub exists: bool,
    pub balance_drops: u64,
    pub owner_count: u64,
    pub reserve_base: u64,
    pub reserve_increment: u64,
}

/// `lsfRequireDestTag`: payments to the account need a destination tag.
pub(crate) const LSF_REQUIRE_DEST_TAG: u32 = 0x0002_0000;

/// An account root's `Flags`.
fn flags_of(root: &Value) -> Result<u32, ApiError> {
    root.get("Flags")
        .and_then(Value::as_u64)
        .and_then(|flags| u32::try_from(flags).ok())
        .or_decode("account_info: missing Flags")
}

/// What deleting an account depends on, from one node's validated ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpDeletionState {
    /// `None` when the account does not exist.
    pub source: Option<XrpDeletableAccount>,
    /// The destination's flags; `None` when it does not exist.
    pub destination_flags: Option<u32>,
    pub ledger_index: u32,
    pub reserve_base: u64,
    pub reserve_increment: u64,
}

/// The account being deleted, as the validated ledger holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpDeletableAccount {
    pub balance_drops: u64,
    pub sequence: u32,
    pub owner_count: u64,
    /// `FirstNFTokenSequence + MintedNFTokens`, where the account has minted.
    pub minted_nft_sequence: Option<u64>,
    /// Objects in its owner directory the network will not delete with it:
    /// trust lines, escrows, payment channels, checks and the like.
    pub blockers: u64,
}

/// One of an account's trust lines, as `account_lines` reports it from the
/// account's own side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrplTrustLine {
    pub issue: crate::api::xrpl_amount::XrplIssue,
    /// What the account holds; negative where it owes the other side.
    pub balance: crate::api::xrpl_amount::IouValue,
    /// The most the account accepts.
    pub limit: crate::api::xrpl_amount::IouValue,
    /// The most the other side accepts from the account.
    pub limit_peer: crate::api::xrpl_amount::IouValue,
    /// The other side does not let payments ripple through this line.
    pub no_ripple_peer: bool,
    /// The issuer has authorized the account to hold its currency.
    pub peer_authorized: bool,
    /// The issuer has frozen the line: the account can send only back to it.
    pub frozen: bool,
    /// The issuer has deep-frozen the line: it can neither send nor receive.
    pub deep_frozen: bool,
}

/// An issuer's account flags and transfer rate, from its ledger root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct XrplIssuer {
    pub flags: u32,
    /// Billionths: 1 000 000 000 charges nothing, 1 002 000 000 takes 0.2%.
    pub transfer_rate: u32,
}

impl XrplIssuer {
    pub const REQUIRE_AUTH: u32 = 0x0004_0000;
    pub const GLOBAL_FREEZE: u32 = 0x0040_0000;
    pub const DISALLOW_INCOMING_TRUSTLINE: u32 = 0x2000_0000;
}

/// An account's `lsfDepositAuth`: payments need its preauthorization.
pub(crate) const DEPOSIT_AUTH: u32 = 0x0100_0000;

/// Everything a payment of an issued currency, or a trust line for one,
/// depends on, from one verified node's validated ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrplIssueState {
    /// The holder's balance and owner count; `None` when it does not exist.
    pub holder: Option<(u64, u64)>,
    pub holder_line: Option<XrplTrustLine>,
    /// `None` when the issuer does not exist.
    pub issuer: Option<XrplIssuer>,
    /// The destination's flags and line, when a destination was asked
    /// about; `None` inside when it does not exist.
    pub destination: Option<Option<(u32, Option<XrplTrustLine>)>>,
    /// Whether the destination accepts the holder's payments, when it
    /// requires authorization for deposits.
    pub deposit_authorized: Option<bool>,
    pub reserve_base: u64,
    pub reserve_increment: u64,
}

fn parse_trust_line(line: &Value) -> Result<XrplTrustLine, ApiError> {
    use crate::api::xrpl_amount::{IouValue, XrplCurrency, XrplIssue};
    let text = |field: &str| {
        line.get(field)
            .and_then(Value::as_str)
            .or_decode("account_lines: incomplete line")
    };
    let value =
        |field: &str| IouValue::parse(text(field)?).or_decode("account_lines: invalid amount");
    let flag = |field: &str| line.get(field).and_then(Value::as_bool).unwrap_or(false);
    let issuer = text("account")?;
    crate::derivation::xrp::decode_xrp_address(issuer)
        .map_err(|_| ApiError::decode("account_lines: invalid counterparty"))?;
    Ok(XrplTrustLine {
        issue: XrplIssue {
            currency: XrplCurrency::parse(text("currency")?)
                .map_err(|_| ApiError::decode("account_lines: invalid currency"))?,
            issuer: issuer.to_string(),
        },
        balance: value("balance")?,
        limit: value("limit")?,
        limit_peer: value("limit_peer")?,
        no_ripple_peer: flag("no_ripple_peer"),
        peer_authorized: flag("peer_authorized"),
        frozen: flag("freeze_peer"),
        deep_frozen: flag("deep_freeze_peer"),
    })
}

impl XrplClient {
    pub fn new(endpoints: std::sync::Arc<Vec<String>>) -> Self {
        Self {
            endpoints,
            client: HttpClient::shared(),
        }
    }

    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, ApiError> {
        crate::api::json_rpc::call(
            crate::EndpointApi::XrplJsonRpc,
            &self.client,
            &self.endpoints,
            method,
            params,
        )
        .await
    }
}

/// What an account's policy read finds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XrpAccountPolicy {
    pub exists: bool,
    pub master_disabled: bool,
    pub regular_key: Option<String>,
    pub signer_list: Option<crate::send::xrp_multisig::XrpSignerList>,
    /// The account's next sequence and its balance, when the node gave them
    /// as numbers in range.
    pub sequence: Option<u32>,
    pub balance_drops: Option<u64>,
    /// The open ledger the account was read at, when the node said.
    pub ledger: Option<u32>,
    /// The open ledger's fee for one signature, in drops, when it was read.
    pub fee_drops: Option<u64>,
}

/// `account_info`'s answer with `signer_lists` as an account's policy:
/// the list under `account_data` (API v1) or beside it (API v2).
pub(crate) fn account_policy(
    response: &Value,
    address: &str,
    fee_drops: Option<u64>,
) -> Result<XrpAccountPolicy, ApiError> {
    let result = response
        .get("result")
        .or_decode("account_info: missing result")?;
    let ledger = result
        .get("ledger_current_index")
        .or_else(|| result.get("ledger_index"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    match result.get("error").and_then(Value::as_str) {
        Some("actNotFound") => {
            return Ok(XrpAccountPolicy {
                exists: false,
                master_disabled: false,
                regular_key: None,
                signer_list: None,
                sequence: None,
                balance_drops: None,
                ledger,
                fee_drops,
            });
        }
        Some(error) => return Err(ApiError::rejected(format!("account_info: {error}"))),
        None => {}
    }
    let data = result
        .get("account_data")
        .or_decode("account_info: missing account_data")?;
    if data
        .get("Account")
        .and_then(Value::as_str)
        .is_some_and(|account| account != address)
    {
        return Err(ApiError::decode("account_info names another account"));
    }
    let lists = data
        .get("signer_lists")
        .or_else(|| result.get("signer_lists"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let signer_list = match lists.as_slice() {
        [] => None,
        [list] => {
            let entries = list
                .get("SignerEntries")
                .and_then(Value::as_array)
                .or_decode("signer list: missing entries")?
                .iter()
                .map(|entry| {
                    let entry = entry.get("SignerEntry").or_decode("signer list: entry")?;
                    let account = entry
                        .get("Account")
                        .and_then(Value::as_str)
                        .or_decode("signer list: entry account")?;
                    crate::derivation::xrp::decode_xrp_address(account)
                        .map_err(ApiError::decode)?;
                    Ok((
                        account.to_string(),
                        entry
                            .get("SignerWeight")
                            .and_then(Value::as_u64)
                            .filter(|weight| *weight > 0)
                            .or_decode("signer list: entry weight")?,
                    ))
                })
                .collect::<Result<Vec<_>, ApiError>>()?;
            let quorum = list
                .get("SignerQuorum")
                .and_then(Value::as_u64)
                .filter(|quorum| *quorum > 0)
                .or_decode("signer list: missing quorum")?;
            if entries.is_empty()
                || entries.len() > crate::send::xrp_multisig::MAX_SIGNERS
                || entries.iter().map(|(_, weight)| weight).sum::<u64>() < quorum
            {
                return Err(ApiError::decode("signer list cannot meet its quorum"));
            }
            Some(crate::send::xrp_multisig::XrpSignerList { quorum, entries })
        }
        _ => return Err(ApiError::decode("account_info: more than one signer list")),
    };
    let flags = data.get("Flags").and_then(Value::as_u64).unwrap_or(0) as u32;
    Ok(XrpAccountPolicy {
        exists: true,
        master_disabled: flags & crate::send::xrp_multisig::LSF_DISABLE_MASTER != 0,
        regular_key: data
            .get("RegularKey")
            .and_then(Value::as_str)
            .map(str::to_string),
        signer_list,
        sequence: data
            .get("Sequence")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok()),
        balance_drops: data
            .get("Balance")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok()),
        ledger,
        fee_drops,
    })
}

// XRP fetch paths: balance, sequence, fee, history.

impl XrplClient {
    pub(crate) async fn verify_network(
        &self,
        chain: crate::registry::Chain,
    ) -> Result<(), ApiError> {
        let expected = chain
            .xrp_network_id()
            .or_decode("Missing XRP network identity")?;
        let result = self.call("server_info", json!({})).await?;
        if result.pointer("/info/network_id").and_then(Value::as_u64) != Some(expected) {
            return Err(ApiError::invalid("XRP endpoint is on the wrong network"));
        }
        Ok(())
    }

    /// A provisional result is not final. Validated `tec` results are committed failures.
    pub(crate) async fn fetch_transaction_status(
        &self,
        hash: &str,
    ) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
        use crate::api::http::{RetryProfile, race};
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ApiError::invalid("Invalid XRP transaction hash"));
        }
        // Inspect the machine-readable error before generic RPC decoding loses it.
        let body = json!({"method":"tx","params":[{"transaction":hash,"binary":false}]});
        race(&self.endpoints, |endpoint| {
            let body = &body;
            async move {
                let response: Value = self
                    .client
                    .post_json(&endpoint, body, RetryProfile::ChainRead)
                    .await?;
                xrp_transaction_status(response, hash)
            }
        })
        .await
    }

    pub async fn fetch_balance(&self, address: &str) -> Result<XrpBalance, ApiError> {
        let result = self
            .call(
                "account_info",
                json!({"account": address, "ledger_index": "validated"}),
            )
            .await?;
        let drops: u64 = result
            .pointer("/account_data/Balance")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("account_info: missing Balance")?;
        Ok(XrpBalance { drops })
    }

    pub async fn fetch_sequence(&self, address: &str) -> Result<u32, ApiError> {
        let result = self
            .call(
                "account_info",
                json!({"account": address, "ledger_index": "current"}),
            )
            .await?;
        result
            .pointer("/account_data/Sequence")
            .and_then(|v| v.as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .or_decode("account_info: missing or invalid Sequence")
    }

    /// The validated ledger's base reserve, in drops: what an account must
    /// hold for the ledger to create it.
    pub(crate) async fn fetch_base_reserve(&self) -> Result<u64, ApiError> {
        let result = self.call("server_state", json!({})).await?;
        result
            .pointer("/state/validated_ledger/reserve_base")
            .and_then(Value::as_u64)
            .or_decode("server_state: missing reserve_base")
    }

    /// An account's reserve as the validated ledger holds it, from one
    /// verified node: whether the account exists, its balance, how many
    /// objects it owns, and the base and per-object reserves, in drops.
    pub(crate) async fn fetch_reserve_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<XrpReserveState, ApiError> {
        use crate::api::http::race;
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            let state = node.call("server_state", json!({})).await?;
            let reserve = |field: &str| {
                state
                    .pointer(&format!("/state/validated_ledger/{field}"))
                    .and_then(Value::as_u64)
                    .or_decode("server_state: missing reserve")
            };
            let (exists, balance_drops, owner_count) =
                match node.account_root(&endpoint, address).await? {
                    None => (false, 0, 0),
                    Some(root) => (true, balance_of(&root)?, owner_count_of(&root)?),
                };
            Ok(XrpReserveState {
                exists,
                balance_drops,
                owner_count,
                reserve_base: reserve("reserve_base")?,
                reserve_increment: reserve("reserve_inc")?,
            })
        })
        .await
    }

    /// What deleting `address` into `destination` depends on, from one
    /// verified node's validated ledger: both accounts, the objects that
    /// would block the deletion, the ledger index and the reserves.
    pub(crate) async fn fetch_deletion_state(
        &self,
        chain: crate::registry::Chain,
        address: &str,
        destination: &str,
    ) -> Result<XrpDeletionState, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            let state = node.call("server_state", json!({})).await?;
            let validated = |field: &str| {
                state
                    .pointer(&format!("/state/validated_ledger/{field}"))
                    .and_then(Value::as_u64)
                    .or_decode("server_state: missing validated ledger")
            };
            let source = match node.account_root(&endpoint, address).await? {
                None => None,
                Some(root) => {
                    let field = |name: &str| root.get(name).and_then(Value::as_u64);
                    let blockers = node
                        .call(
                            "account_objects",
                            json!({
                                "account": address,
                                "ledger_index": "validated",
                                "deletion_blockers_only": true,
                                "limit": 10,
                            }),
                        )
                        .await?
                        .get("account_objects")
                        .and_then(Value::as_array)
                        .or_decode("account_objects: missing list")?
                        .len();
                    Some(XrpDeletableAccount {
                        balance_drops: balance_of(&root)?,
                        sequence: field("Sequence")
                            .and_then(|sequence| u32::try_from(sequence).ok())
                            .or_decode("account_info: missing Sequence")?,
                        owner_count: owner_count_of(&root)?,
                        minted_nft_sequence: field("MintedNFTokens")
                            .map(|minted| minted + field("FirstNFTokenSequence").unwrap_or(0)),
                        blockers: blockers as u64,
                    })
                }
            };
            let destination_flags = node
                .account_root(&endpoint, destination)
                .await?
                .as_ref()
                .map(flags_of)
                .transpose()?;
            Ok(XrpDeletionState {
                source,
                destination_flags,
                ledger_index: u32::try_from(validated("seq")?)
                    .map_err(|_| ApiError::decode("server_state: ledger index out of range"))?,
                reserve_base: validated("reserve_base")?,
                reserve_increment: validated("reserve_inc")?,
            })
        })
        .await
    }

    /// Whether payments to `address` must carry a destination tag
    /// (`lsfRequireDestTag`), from a node verified to be on `chain`. An
    /// account the ledger does not hold asks for none.
    pub(crate) async fn requires_destination_tag(
        &self,
        chain: crate::registry::Chain,
        address: &str,
    ) -> Result<bool, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            Ok(match node.account_root(&endpoint, address).await? {
                Some(root) => flags_of(&root)? & LSF_REQUIRE_DEST_TAG != 0,
                None => false,
            })
        })
        .await
    }

    /// An account's signing policy in the current ledger, read from one node
    /// on `chain`: whether its master key still signs, its regular key, its
    /// signer list, its sequence and balance, with the ledger it was read
    /// at, and with `fee` the open ledger's fee. An account not on the
    /// ledger is its master key alone.
    pub(crate) async fn fetch_account_policy(
        &self,
        chain: crate::registry::Chain,
        address: &str,
        fee: bool,
    ) -> Result<XrpAccountPolicy, ApiError> {
        use crate::api::http::{RetryProfile, race};
        race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            let body = json!({"method": "account_info", "params": [{
                "account": address, "ledger_index": "current", "signer_lists": true}]});
            let response: Value = self
                .client
                .post_json(&endpoint, &body, RetryProfile::ChainRead)
                .await?;
            let fee_drops = if fee {
                Some(node.fetch_fee().await?)
            } else {
                None
            };
            account_policy(&response, address, fee_drops)
        })
        .await
    }

    /// An account's root as the validated ledger holds it on `endpoint`, or
    /// `None` when the ledger has no such account. The unfunded case is an
    /// `actNotFound` error, read here before generic decoding turns it into
    /// a refusal.
    async fn account_root(&self, endpoint: &str, address: &str) -> Result<Option<Value>, ApiError> {
        let body = json!({"method": "account_info", "params": [{"account": address, "ledger_index": "validated"}]});
        let response: Value = self
            .client
            .post_json(endpoint, &body, crate::api::http::RetryProfile::ChainRead)
            .await?;
        let result = response
            .get("result")
            .or_decode("account_info: missing result")?;
        match result.get("error").and_then(Value::as_str) {
            Some("actNotFound") => Ok(None),
            Some(error) => Err(ApiError::rejected(format!("account_info: {error}"))),
            None => Ok(Some(
                result
                    .get("account_data")
                    .cloned()
                    .or_decode("account_info: missing account_data")?,
            )),
        }
    }

    /// Every trust line of `address`, or of `address` with `peer` only, in
    /// the validated ledger. An account that does not exist has none.
    pub(crate) async fn fetch_trust_lines(
        &self,
        address: &str,
        peer: Option<&str>,
    ) -> Result<Vec<XrplTrustLine>, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            Self::new(std::sync::Arc::new(vec![endpoint.clone()]))
                .lines_at(&endpoint, address, peer)
                .await
        })
        .await
    }

    /// `account_lines` on one node, every page of it.
    async fn lines_at(
        &self,
        endpoint: &str,
        address: &str,
        peer: Option<&str>,
    ) -> Result<Vec<XrplTrustLine>, ApiError> {
        let mut lines = Vec::new();
        let mut marker: Option<Value> = None;
        // Four hundred lines a page; a hundred pages is more trust lines than
        // any wallet keeps, and a node that never stops is not answering.
        for _ in 0..100 {
            let mut params = json!({"account": address, "ledger_index": "validated", "limit": 400});
            if let Some(peer) = peer {
                params["peer"] = json!(peer);
            }
            if let Some(marker) = marker.take() {
                params["marker"] = marker;
            }
            let body = json!({"method": "account_lines", "params": [params]});
            let response: Value = self
                .client
                .post_json(endpoint, &body, crate::api::http::RetryProfile::ChainRead)
                .await?;
            let result = response
                .get("result")
                .or_decode("account_lines: missing result")?;
            match result.get("error").and_then(Value::as_str) {
                Some("actNotFound") => return Ok(Vec::new()),
                Some(error) => return Err(ApiError::rejected(format!("account_lines: {error}"))),
                None => {}
            }
            for line in result
                .get("lines")
                .and_then(Value::as_array)
                .or_decode("account_lines: missing lines")?
            {
                lines.push(parse_trust_line(line)?);
            }
            match result.get("marker").filter(|marker| !marker.is_null()) {
                Some(next) => marker = Some(next.clone()),
                None => return Ok(lines),
            }
        }
        Err(ApiError::decode("account_lines: pages never end"))
    }

    /// What paying `issue` from `holder`, or trusting it, depends on: the
    /// holder, its line, the issuer, and the destination and its line, all
    /// from one verified node's validated ledger.
    pub(crate) async fn fetch_issue_state(
        &self,
        chain: crate::registry::Chain,
        holder: &str,
        issue: &crate::api::xrpl_amount::XrplIssue,
        destination: Option<&str>,
    ) -> Result<XrplIssueState, ApiError> {
        crate::api::http::race(&self.endpoints, |endpoint| async move {
            let node = Self::new(std::sync::Arc::new(vec![endpoint.clone()]));
            node.verify_network(chain).await?;
            let state = node.call("server_state", json!({})).await?;
            let reserve = |field: &str| {
                state
                    .pointer(&format!("/state/validated_ledger/{field}"))
                    .and_then(Value::as_u64)
                    .or_decode("server_state: missing reserve")
            };
            let line_of = |lines: Vec<XrplTrustLine>| {
                lines.into_iter().find(|line| line.issue == *issue)
            };
            let flags_of = |root: &Value| {
                root.get("Flags")
                    .and_then(Value::as_u64)
                    .and_then(|flags| u32::try_from(flags).ok())
                    .or_decode("account_info: missing Flags")
            };
            let holder_root = node.account_root(&endpoint, holder).await?;
            let holder_line = match holder_root {
                Some(_) => line_of(node.lines_at(&endpoint, holder, Some(&issue.issuer)).await?),
                None => None,
            };
            let issuer = match node.account_root(&endpoint, &issue.issuer).await? {
                None => None,
                Some(root) => Some(XrplIssuer {
                    flags: flags_of(&root)?,
                    transfer_rate: match root.get("TransferRate") {
                        None => 1_000_000_000,
                        Some(rate) => rate
                            .as_u64()
                            .and_then(|rate| u32::try_from(rate).ok())
                            // Zero is how the ledger spells "no fee".
                            .map(|rate| if rate == 0 { 1_000_000_000 } else { rate })
                            .filter(|rate| (1_000_000_000..=2_000_000_000).contains(rate))
                            .or_decode("account_info: invalid TransferRate")?,
                    },
                }),
            };
            let mut deposit_authorized = None;
            let destination = match destination {
                None => None,
                Some(destination) => Some(match node.account_root(&endpoint, destination).await? {
                    None => None,
                    Some(root) => {
                        let flags = flags_of(&root)?;
                        if flags & DEPOSIT_AUTH != 0 {
                            let answer = node
                                .call(
                                    "deposit_authorized",
                                    json!({"source_account": holder, "destination_account": destination,
                                           "ledger_index": "validated"}),
                                )
                                .await?;
                            deposit_authorized = Some(
                                answer
                                    .get("deposit_authorized")
                                    .and_then(Value::as_bool)
                                    .or_decode("deposit_authorized: missing answer")?,
                            );
                        }
                        let line = if destination == issue.issuer {
                            None
                        } else {
                            line_of(node.lines_at(&endpoint, destination, Some(&issue.issuer)).await?)
                        };
                        Some((flags, line))
                    }
                }),
            };
            Ok(XrplIssueState {
                holder: match &holder_root {
                    None => None,
                    Some(root) => Some((balance_of(root)?, owner_count_of(root)?)),
                },
                holder_line,
                issuer,
                destination,
                deposit_authorized,
                reserve_base: reserve("reserve_base")?,
                reserve_increment: reserve("reserve_inc")?,
            })
        })
        .await
    }

    pub async fn fetch_fee(&self) -> Result<u64, ApiError> {
        let result = self.call("fee", json!({})).await?;
        result
            .pointer("/drops/open_ledger_fee")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_decode("fee: missing open_ledger_fee")
    }

    pub async fn fetch_history_page(
        &self,
        address: &str,
        cursor: Option<&str>,
    ) -> Result<crate::api::HistoryPage<XrpHistoryEntry>, ApiError> {
        let mut params = json!({"account": address, "limit": 50, "ledger_index_min": -1, "ledger_index_max": -1, "forward": false});
        if let Some(cursor) = cursor {
            params["marker"] = serde_json::from_str(cursor)?;
        }
        let result = self.call("account_tx", params).await?;
        let txs = result
            .get("transactions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let next_cursor = result
            .get("marker")
            .filter(|value| !value.is_null())
            .map(Value::to_string);
        Ok(crate::api::HistoryPage {
            items: xrp_history_from_transactions(&txs, address)?,
            next_cursor,
        })
    }
}

fn xrp_transaction_status(
    response: Value,
    hash: &str,
) -> Result<crate::api::transaction_status::TransactionStatus, ApiError> {
    use crate::api::transaction_status::TransactionStatus;
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return Err(ApiError::rejected(format!("XRP transaction: {error}")));
    }
    let result = response
        .get("result")
        .or_decode("XRP transaction: missing result")?;
    if result.get("status").and_then(Value::as_str) == Some("error") {
        return match result.get("error").and_then(Value::as_str) {
            Some("txnNotFound") => Ok(TransactionStatus::Pending),
            _ => Err(ApiError::rejected(format!("XRP transaction: {result}"))),
        };
    }
    if result.get("validated").and_then(Value::as_bool) != Some(true) {
        return Ok(TransactionStatus::Pending);
    }
    let actual = result
        .get("hash")
        .or_else(|| result.pointer("/tx_json/hash"))
        .and_then(Value::as_str)
        .or_decode("XRP transaction: missing hash")?;
    if !actual.eq_ignore_ascii_case(hash) {
        return Err(ApiError::decode("XRP returned a different transaction"));
    }
    let ledger = result
        .get("ledger_index")
        .or_else(|| result.pointer("/tx_json/ledger_index"))
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .or_decode("XRP transaction: missing validated ledger")?;
    let outcome = result
        .pointer("/meta/TransactionResult")
        .and_then(Value::as_str)
        .or_decode("XRP transaction: missing execution result")?;
    let succeeded = if outcome == "tesSUCCESS" {
        true
    } else if outcome.starts_with("tec") {
        false
    } else {
        return Err(ApiError::decode(
            "XRP transaction: invalid validated result code",
        ));
    };
    Ok(TransactionStatus::Confirmed {
        succeeded,
        block: Some(ledger),
    })
}

/// Each issued currency a transaction moved in or out of `address`: the
/// change in every trust line of the account its metadata touched, with the
/// line's balance read from the account's side (the ledger stores it from
/// the low account's).
fn trust_line_changes(meta: &Value, address: &str) -> Vec<(String, bool, String)> {
    use crate::api::xrpl_amount::{IouValue, XrplCurrency, XrplIssue};
    let Some(nodes) = meta.get("AffectedNodes").and_then(Value::as_array) else {
        return Vec::new();
    };
    nodes
        .iter()
        .filter_map(|node| {
            let (kind, node) = node.as_object()?.iter().next()?;
            if node.get("LedgerEntryType").and_then(Value::as_str) != Some("RippleState") {
                return None;
            }
            let fields = if kind == "CreatedNode" {
                node.get("NewFields")?
            } else {
                node.get("FinalFields")?
            };
            let side = |limit: &str| fields.pointer(&format!("/{limit}/issuer"))?.as_str();
            let (low, high) = (side("LowLimit")?, side("HighLimit")?);
            let (ours_is_low, counterparty) = if low == address {
                (true, high)
            } else if high == address {
                (false, low)
            } else {
                return None;
            };
            let read = |balance: Option<&Value>| {
                let value = IouValue::parse(balance?.get("value")?.as_str()?)?;
                Some(if ours_is_low {
                    value
                } else {
                    IouValue {
                        negative: !value.negative && !value.is_zero(),
                        ..value
                    }
                })
            };
            let after = read(fields.get("Balance"))?;
            let before = match kind.as_str() {
                "CreatedNode" => IouValue::ZERO,
                _ => read(node.pointer("/PreviousFields/Balance")).unwrap_or(after),
            };
            let (negative, change) = after.minus(&before);
            if crate::decimal::is_zero(&change) {
                return None;
            }
            let currency = fields.pointer("/Balance/currency")?.as_str()?;
            let issue = XrplIssue {
                currency: XrplCurrency::parse(currency).ok()?,
                issuer: counterparty.to_string(),
            };
            Some((issue.identifier(), !negative, change))
        })
        .collect()
}

/// What each successful payment moved in or out of `address`: XRP, fee
/// excluded, and each issued currency.
///
/// Every amount is the account's own balance change from the transaction's
/// metadata, not the payment's `Amount` field, which for a partial payment
/// is an upper bound larger than what was delivered. An issued currency's
/// change comes from the account's trust line for it and is its own row,
/// beside the XRP row when a payment moved both. A failed payment only
/// burned its fee and yields nothing.
fn xrp_history_from_transactions(
    txs: &[Value],
    address: &str,
) -> Result<Vec<XrpHistoryEntry>, ApiError> {
    let drops = |value: Option<&Value>| -> Option<i128> {
        value.and_then(Value::as_str).and_then(|s| s.parse().ok())
    };
    let mut entries = Vec::new();
    for item in txs {
        let tx = item.get("tx").unwrap_or(&Value::Null);
        let meta = item.get("meta").unwrap_or(&Value::Null);
        if tx.get("TransactionType").and_then(Value::as_str) != Some("Payment")
            || meta.get("TransactionResult").and_then(Value::as_str) != Some("tesSUCCESS")
        {
            continue;
        }
        let from = tx
            .get("Account")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let to = tx
            .get("Destination")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let fee_drops = drops(tx.get("Fee")).unwrap_or(0);
        let balance_change: i128 = meta
            .get("AffectedNodes")
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|node| {
                        let (kind, node) = node.as_object()?.iter().next()?;
                        if node.get("LedgerEntryType").and_then(Value::as_str)
                            != Some("AccountRoot")
                        {
                            return None;
                        }
                        let fields = if kind == "CreatedNode" {
                            node.get("NewFields")?
                        } else {
                            node.get("FinalFields")?
                        };
                        if fields.get("Account").and_then(Value::as_str) != Some(address) {
                            return None;
                        }
                        let after = drops(fields.get("Balance"))?;
                        let before = match kind.as_str() {
                            "CreatedNode" => 0,
                            _ => drops(node.pointer("/PreviousFields/Balance")).unwrap_or(after),
                        };
                        Some(after - before)
                    })
                    .sum()
            })
            .unwrap_or(0);
        let transfer = if from == address {
            balance_change + fee_drops
        } else {
            balance_change
        };
        let issued = trust_line_changes(meta, address);
        if transfer == 0 && issued.is_empty() {
            continue;
        }
        let txid = tx
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // `account_tx` up to the validated ledger lists only validated
        // transactions. XRP epoch: 2000-01-01, Unix epoch difference = 946684800.
        let timestamp = crate::api::time::confirmed_history_time(
            tx.get("date")
                .and_then(Value::as_u64)
                .map(|d| d + 946_684_800),
            &txid,
        )?;
        let row = |amount_drops: u64, is_incoming: bool| XrpHistoryEntry {
            txid: txid.clone(),
            ledger_index: tx.get("ledger_index").and_then(Value::as_u64).unwrap_or(0),
            timestamp,
            from: from.clone(),
            to: to.clone(),
            amount_drops,
            fee_drops: u64::try_from(fee_drops).unwrap_or(0),
            is_incoming,
            contract: None,
            amount_display: None,
        };
        if let Ok(amount_drops) = u64::try_from(transfer.unsigned_abs())
            && transfer != 0
        {
            entries.push(row(amount_drops, transfer > 0));
        }
        for (contract, is_incoming, amount) in issued {
            entries.push(XrpHistoryEntry {
                contract: Some(contract),
                amount_display: Some(amount),
                ..row(0, is_incoming)
            });
        }
    }
    Ok(entries)
}

impl XrplClient {
    /// Submit a pre-signed transaction blob (for rebroadcast).
    pub async fn submit_signed_blob(&self, tx_blob_hex: &str) -> Result<XrpSendResult, ApiError> {
        let result = self.call("submit", json!({"tx_blob": tx_blob_hex})).await?;
        let engine_result = result
            .get("engine_result")
            .and_then(Value::as_str)
            .or_decode("submit: missing engine_result")?;
        let accepted = result
            .get("accepted")
            .and_then(Value::as_bool)
            .or_decode("submit: missing accepted")?;
        if !accepted || !matches!(engine_result, "tesSUCCESS" | "terQUEUED") {
            let message = result
                .get("engine_result_message")
                .and_then(Value::as_str)
                .unwrap_or("transaction was not accepted");
            return Err(ApiError::rejected(format!(
                "submit: {engine_result}: {message}"
            )));
        }
        let txid = result
            .get("tx_json")
            .and_then(|t| t.get("hash"))
            .and_then(|v| v.as_str())
            .filter(|hash| !hash.is_empty())
            .or_decode("submit: missing transaction hash")?
            .to_string();
        Ok(XrpSendResult {
            txid,
            tx_blob_hex: tx_blob_hex.to_string(),
        })
    }
}

fn balance_of(root: &Value) -> Result<u64, ApiError> {
    root.get("Balance")
        .and_then(Value::as_str)
        .and_then(|drops| drops.parse().ok())
        .or_decode("account_info: missing Balance")
}

fn owner_count_of(root: &Value) -> Result<u64, ApiError> {
    root.get("OwnerCount")
        .and_then(Value::as_u64)
        .or_decode("account_info: missing OwnerCount")
}

#[cfg(test)]
#[path = "tests/xrpl_json_rpc.rs"]
mod tests;

#[cfg(test)]
mod transaction_status_tests {
    use super::*;
    use crate::api::transaction_status::TransactionStatus;
    use std::sync::Arc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method},
    };

    #[tokio::test]
    async fn hash_lookup_requires_validation_and_retains_committed_failure() {
        let hash = "AB".repeat(32);
        for (result, expected) in [
            (
                json!({"status":"error","error":"txnNotFound"}),
                TransactionStatus::Pending,
            ),
            (
                json!({"validated":false,"hash":hash,"meta":{"TransactionResult":"tesSUCCESS"}}),
                TransactionStatus::Pending,
            ),
            (
                json!({"validated":true,"hash":hash,"ledger_index":120,"meta":{"TransactionResult":"tesSUCCESS"}}),
                TransactionStatus::Confirmed {
                    succeeded: true,
                    block: Some(120),
                },
            ),
            (
                json!({"validated":true,"hash":hash.to_lowercase(),"ledger_index":121,"meta":{"TransactionResult":"tecUNFUNDED_PAYMENT"}}),
                TransactionStatus::Confirmed {
                    succeeded: false,
                    block: Some(121),
                },
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(
                    json!({"method":"tx","params":[{"transaction":hash,"binary":false}]}),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"result":result})))
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                XrplClient::new(Arc::new(vec![server.uri()]))
                    .fetch_transaction_status(&hash)
                    .await
                    .unwrap(),
                expected
            );
        }
        for result in [
            json!({"validated":true,"hash":"CD".repeat(32),"ledger_index":120,"meta":{"TransactionResult":"tesSUCCESS"}}),
            json!({"validated":true,"hash":hash,"ledger_index":120}),
            json!({"validated":true,"hash":hash,"meta":{"TransactionResult":"tesSUCCESS"}}),
            json!({"validated":true,"hash":hash,"ledger_index":120,"meta":{"TransactionResult":"tefPAST_SEQ"}}),
            json!({"status":"error","error":"invalidParams"}),
        ] {
            assert!(xrp_transaction_status(json!({"result":result}), &hash).is_err());
        }
    }

    #[tokio::test]
    async fn server_network_identity_refuses_wrong_and_unknown_networks() {
        use crate::registry::Chain;
        for (actual, selected, expected_ok) in [
            (json!(0), Chain::Xrp, true),
            (json!(1), Chain::XrpTestnet, true),
            (json!(1), Chain::Xrp, false),
            (Value::Null, Chain::Xrp, false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_json(json!({"method":"server_info","params":[{}]})))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"result":{"info":{"network_id":actual}}})),
                )
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                XrplClient::new(Arc::new(vec![server.uri()]))
                    .verify_network(selected)
                    .await
                    .is_ok(),
                expected_ok
            );
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    const ME: &str = "rMeMeMeMeMeMeMeMeMeMeMeMeMeMeMeMe1";
    const THEM: &str = "rThemThemThemThemThemThemThemThem2";
    const ISSUER: &str = "rhub8VRN55s94qWKDv6jmDy1pUykJzF3wq";

    fn account_root(account: &str, before: &str, after: &str) -> Value {
        json!({"ModifiedNode": {
            "LedgerEntryType": "AccountRoot",
            "FinalFields": {"Account": account, "Balance": after},
            "PreviousFields": {"Balance": before}
        }})
    }

    fn payment(
        hash: &str,
        from: &str,
        to: &str,
        amount: Value,
        result: &str,
        nodes: Value,
    ) -> Value {
        json!({
            "tx": {
                "TransactionType": "Payment", "hash": hash, "Account": from,
                "Destination": to, "Amount": amount, "Fee": "12", "date": 800_000_000u64
            },
            "meta": {"TransactionResult": result, "AffectedNodes": nodes}
        })
    }

    #[test]
    fn amounts_are_the_accounts_own_balance_change() {
        let txs = [
            // An outgoing payment: the fee is not part of the transfer.
            payment(
                "sent",
                ME,
                THEM,
                json!("5000000"),
                "tesSUCCESS",
                json!([
                    account_root(ME, "20000000", "14999988"),
                    account_root(THEM, "0", "5000000")
                ]),
            ),
            // A partial payment claims 1,000 XRP and delivers 1.
            payment(
                "partial",
                THEM,
                ME,
                json!("1000000000"),
                "tesSUCCESS",
                json!([account_root(ME, "14999988", "15999988")]),
            ),
            // An issued currency moves no XRP for this account: its trust
            // line, where it is the high side, gains 5 USD.
            payment(
                "iou",
                THEM,
                ME,
                json!({"currency": "USD", "issuer": ISSUER, "value": "5"}),
                "tesSUCCESS",
                json!([
                    account_root(THEM, "100", "88"),
                    ripple_state(ISSUER, ME, "USD", "-1.5", "-6.5"),
                    ripple_state(THEM, ISSUER, "USD", "10", "5")
                ]),
            ),
            // A line created by the payment, where this account is the low side.
            payment(
                "first",
                THEM,
                ME,
                json!({"currency": "534F4C4F00000000000000000000000000000000", "issuer": ISSUER, "value": "1e-20"}),
                "tesSUCCESS",
                json!([{"CreatedNode": {"LedgerEntryType": "RippleState", "NewFields": {
                    "Balance": {"currency": "534F4C4F00000000000000000000000000000000", "issuer": "rrrrrrrrrrrrrrrrrrrrBZbvji", "value": "1e-20"},
                    "LowLimit": {"issuer": ME}, "HighLimit": {"issuer": ISSUER}}}}]),
            ),
            // A failed payment only burned its fee.
            payment(
                "failed",
                ME,
                THEM,
                json!("5000000"),
                "tecUNFUNDED_PAYMENT",
                json!([account_root(ME, "15999988", "15999976")]),
            ),
        ];
        let entries = xrp_history_from_transactions(&txs, ME).unwrap();
        assert_eq!(entries.len(), 4, "{entries:?}");
        assert_eq!(entries[0].txid, "sent");
        assert!(!entries[0].is_incoming);
        assert_eq!(entries[0].amount_drops, 5_000_000);
        assert_eq!(entries[1].txid, "partial");
        assert!(entries[1].is_incoming);
        assert_eq!(entries[1].amount_drops, 1_000_000);
        // Only this account's line counts, read from its own side.
        assert_eq!(entries[2].txid, "iou");
        assert!(entries[2].is_incoming);
        assert_eq!(
            entries[2].contract.as_deref(),
            Some(format!("USD.{ISSUER}").as_str())
        );
        assert_eq!(entries[2].amount_display.as_deref(), Some("5"));
        assert_eq!(
            entries[3].contract.as_deref(),
            Some(format!("534F4C4F00000000000000000000000000000000.{ISSUER}").as_str())
        );
        assert_eq!(
            entries[3].amount_display.as_deref(),
            Some("0.00000000000000000001")
        );
        // XRP rows name no contract when serialized, as the normalizer reads them.
        let json = serde_json::to_value(&entries[0]).unwrap();
        assert!(json.get("contract").is_none() && json.get("amount_display").is_none());
    }

    fn ripple_state(low: &str, high: &str, currency: &str, before: &str, after: &str) -> Value {
        let balance = |value: &str| json!({"currency": currency, "issuer": "rrrrrrrrrrrrrrrrrrrrBZbvji", "value": value});
        json!({"ModifiedNode": {
            "LedgerEntryType": "RippleState",
            "FinalFields": {"Balance": balance(after), "LowLimit": {"issuer": low}, "HighLimit": {"issuer": high}},
            "PreviousFields": {"Balance": balance(before)}
        }})
    }
}
