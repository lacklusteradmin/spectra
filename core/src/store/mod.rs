pub mod artwork;
pub mod password_verifier;
pub mod persistence_models;
mod price_alerts;
pub use price_alerts::PriceAlertRejection;
pub mod secret_backends;
pub mod secret_store;
pub mod seed_envelope;
pub mod state;
pub mod wallet_db;
pub mod wallet_domain;
pub mod wallet_secrets;

pub use artwork::{
    chain_artwork_name, deployment_artwork_name, holding_artwork_name, token_artwork_name,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SecretMaterialDescriptor {
    pub wallet_id: String,
    pub secret_kind: String,
    pub has_seed_phrase: bool,
    pub has_private_key: bool,
    pub has_password: bool,
    pub has_signing_material: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletHoldingRef {
    pub wallet_id: String,
    pub holding_index: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct GroupedPortfolioHolding {
    pub asset_identity_key: String,
    pub wallet_id: String,
    pub holding_index: u64,
    pub total_amount: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PendingSelfSendConfirmationInput {
    pub wallet_id: String,
    pub chain_name: String,
    pub symbol: String,
    pub destination_address_lowercased: String,
    pub amount: f64,
    pub created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SelfSendConfirmationRequest {
    pub pending_confirmation: Option<PendingSelfSendConfirmationInput>,
    pub wallet_id: String,
    pub chain_name: String,
    pub symbol: String,
    pub destination_address: String,
    pub amount: f64,
    pub now_unix: f64,
    pub window_seconds: f64,
    pub owned_addresses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SelfSendConfirmationPlan {
    pub requires_confirmation: bool,
    pub consume_existing_confirmation: bool,
    pub clear_pending_confirmation: bool,
}

/// Trimmed, blanks dropped, and each address once — compared case-folded, the
/// first spelling kept.
pub fn aggregate_owned_addresses(candidates: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = std::collections::BTreeSet::<String>::new();

    for candidate in candidates {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = trimmed.to_lowercase();
        if seen.insert(normalized) {
            ordered.push(trimmed.to_string());
        }
    }

    ordered
}

pub fn self_send_confirmation(request: SelfSendConfirmationRequest) -> SelfSendConfirmationPlan {
    // Folded to lowercase, deliberately, and unlike the `new_address` check in
    // `send::flow` — which compares in the chain's own normal form because a
    // false match there *suppresses* a warning about a swapped destination.
    //
    // Here a wrong answer goes the other way. A false match adds a
    // "you are sending to yourself" prompt the user dismisses; a miss removes
    // one they should have seen. Bech32 is case-insensitive by definition, so
    // an own address typed in caps is the same address, and `Bitcoin`'s
    // `AddressNormalization::None` — correct for its base58 forms — cannot say
    // that. Folding takes the side where being wrong costs a tap.
    let destination = request.destination_address.trim().to_lowercase();
    let owned_addresses = request
        .owned_addresses
        .iter()
        .map(|address| address.trim().to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();

    if !owned_addresses.contains(&destination) {
        return SelfSendConfirmationPlan {
            requires_confirmation: false,
            consume_existing_confirmation: false,
            clear_pending_confirmation: false,
        };
    }

    let Some(pending) = request.pending_confirmation else {
        return SelfSendConfirmationPlan {
            requires_confirmation: true,
            consume_existing_confirmation: false,
            clear_pending_confirmation: false,
        };
    };

    let is_expired = request.now_unix - pending.created_at_unix > request.window_seconds;
    if is_expired {
        return SelfSendConfirmationPlan {
            requires_confirmation: true,
            consume_existing_confirmation: false,
            clear_pending_confirmation: true,
        };
    }

    let same_wallet = pending.wallet_id == request.wallet_id;
    let same_chain = pending.chain_name == request.chain_name;
    let same_symbol = pending.symbol == request.symbol;
    let same_destination = pending.destination_address_lowercased == destination;
    let same_amount = (pending.amount - request.amount).abs() < 0.00000001;

    if same_wallet && same_chain && same_symbol && same_destination && same_amount {
        return SelfSendConfirmationPlan {
            requires_confirmation: false,
            consume_existing_confirmation: true,
            clear_pending_confirmation: true,
        };
    }

    SelfSendConfirmationPlan {
        requires_confirmation: true,
        consume_existing_confirmation: false,
        clear_pending_confirmation: true,
    }
}

/// Normalize a token contract address for identity matching.
fn normalize_known_token_identifier(
    chain: wallet_domain::CoreTokenHostingChain,
    contract_address: &str,
) -> String {
    crate::tokens::normalize_token_identifier(
        Some(contract_address.to_string()),
        chain.chain_name().to_string(),
    )
    .unwrap_or_default()
}

/// The built-in token catalog, as preference entries.
///
/// Built from `tokens.toml` — the same catalog `list_all_builtin_token_deployments`
/// serves. A caller used to fetch that list, reshape each row into a
/// preference entry, and hand it back for merging; the reshaping is here now,
/// where the catalog already is.
///
/// `id` is derived from chain and contract rather than minted at random: a
/// built-in's identity *is* its contract, and the old ids were regenerated on
/// every launch anyway.
pub fn built_in_token_preferences() -> Vec<wallet_domain::CoreTokenPreferenceEntry> {
    crate::tokens::catalog()
        .iter()
        .filter(|token| !token.is_native())
        .filter_map(|token| {
            // A catalog row on a chain that cannot host tokens is a data
            // mistake, and skipping it is how it stays one.
            wallet_domain::CoreTokenHostingChain::from_chain_name(
                crate::registry::Chain::from_str_id(&token.chain_id)?.chain_display_name(),
            )?;
            Some(wallet_domain::CoreTokenPreferenceEntry {
                category: wallet_domain::CoreTokenPreferenceEntry::category_from_tags(&token.tags),
                is_built_in: true,
                is_enabled: token.enabled,
                token: token.clone(),
            })
        })
        .collect()
}

/// Merge built-in token registry entries with persisted user preferences:
/// copies `is_enabled` from matching persisted built-ins,
/// appends all non-built-in (custom) persisted entries, and returns the list
/// sorted by (chain-label, built-in first, symbol).
pub fn merge_built_in_token_preferences(
    built_ins: Vec<wallet_domain::CoreTokenPreferenceEntry>,
    persisted: Vec<wallet_domain::CoreTokenPreferenceEntry>,
) -> Vec<wallet_domain::CoreTokenPreferenceEntry> {
    let mut merged: Vec<wallet_domain::CoreTokenPreferenceEntry> = Vec::new();
    for built_in in built_ins.into_iter() {
        let Some(built_in_chain) = built_in.hosting_chain() else {
            continue;
        };
        let built_in_key =
            normalize_known_token_identifier(built_in_chain, &built_in.token.contract);
        let existing = persisted.iter().find(|entry| {
            entry.is_built_in
                && entry.token.chain_id == built_in.token.chain_id
                && entry.hosting_chain().is_some_and(|c| {
                    normalize_known_token_identifier(c, &entry.token.contract) == built_in_key
                })
        });
        let mut updated = built_in;
        if let Some(existing) = existing {
            updated.is_enabled = existing.is_enabled;
        }
        merged.push(updated);
    }
    merged.extend(persisted.into_iter().filter(|entry| !entry.is_built_in));
    merged.sort_by(|lhs, rhs| {
        lhs.token
            .chain_id
            .cmp(&rhs.token.chain_id)
            .then_with(|| rhs.is_built_in.cmp(&lhs.is_built_in))
            .then_with(|| lhs.token.symbol.cmp(&rhs.token.symbol))
    });
    merged
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletEarliestTransactionDate {
    pub wallet_id: String,
    pub earliest_created_at_unix: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CoreResetPlan {
    pub reset_wallets_and_secrets: bool,
    pub reset_history_and_cache: bool,
    pub reset_alerts_and_contacts: bool,
    pub reset_settings_and_endpoints: bool,
    pub reset_dashboard_customization: bool,
    pub reset_provider_state: bool,
    pub clear_network_and_transport_caches: bool,
}

/// Which sub-resets a set of user-chosen scopes implies.
///
/// Core applies the domain resets; the returned scope expansion also tells
/// the platform which local caches and preferences to clear.
pub fn reset_dispatch(scopes: Vec<state::ResetScope>) -> CoreResetPlan {
    use state::ResetScope;
    let has = |scope: ResetScope| scopes.contains(&scope);
    let wallets_and_secrets = has(ResetScope::WalletsAndSecrets);
    let history_and_cache_direct = has(ResetScope::HistoryAndCache);
    let history_and_cache = wallets_and_secrets || history_and_cache_direct;
    CoreResetPlan {
        reset_wallets_and_secrets: wallets_and_secrets,
        reset_history_and_cache: history_and_cache,
        reset_alerts_and_contacts: has(ResetScope::AlertsAndContacts),
        reset_settings_and_endpoints: has(ResetScope::SettingsAndEndpoints),
        reset_dashboard_customization: has(ResetScope::DashboardCustomization),
        reset_provider_state: has(ResetScope::ProviderState),
        clear_network_and_transport_caches: wallets_and_secrets || history_and_cache_direct,
    }
}

/// Input per price alert — ids/metadata needed to produce notifications;
/// Swift formats the user-facing text itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertEvaluationAlert {
    pub id: String,
    pub holding_key: String,
    pub asset_display_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub target_price: f64,
    pub condition: wallet_domain::CorePriceAlertCondition,
    pub is_enabled: bool,
    pub has_triggered: bool,
}

/// Live price lookup for one holding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertEvaluationPrice {
    pub holding_key: String,
    pub live_price: f64,
}

/// Alert `has_triggered` state changes produced by the evaluator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertTriggerUpdate {
    pub id: String,
    pub has_triggered: bool,
}

/// A single firing — Swift formats the notification body using this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertNotification {
    pub id: String,
    pub asset_display_name: String,
    pub symbol: String,
    pub chain_name: String,
    pub target_price: f64,
    pub live_price: f64,
    pub condition: wallet_domain::CorePriceAlertCondition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertEvaluation {
    pub updates: Vec<PriceAlertTriggerUpdate>,
    pub notifications: Vec<PriceAlertNotification>,
}

pub fn evaluate_price_alerts(
    alerts: Vec<PriceAlertEvaluationAlert>,
    prices: Vec<PriceAlertEvaluationPrice>,
) -> PriceAlertEvaluation {
    let price_by_key: HashMap<String, f64> = prices
        .into_iter()
        .map(|p| (p.holding_key, p.live_price))
        .collect();
    let mut updates = Vec::new();
    let mut notifications = Vec::new();
    for alert in alerts.into_iter() {
        if !alert.is_enabled {
            continue;
        }
        let Some(live_price) = price_by_key.get(&alert.holding_key).copied() else {
            continue;
        };
        let meets_target = match alert.condition {
            wallet_domain::CorePriceAlertCondition::Above => live_price >= alert.target_price,
            wallet_domain::CorePriceAlertCondition::Below => live_price <= alert.target_price,
        };
        if meets_target && !alert.has_triggered {
            updates.push(PriceAlertTriggerUpdate {
                id: alert.id.clone(),
                has_triggered: true,
            });
            notifications.push(PriceAlertNotification {
                id: alert.id,
                asset_display_name: alert.asset_display_name,
                symbol: alert.symbol,
                chain_name: alert.chain_name,
                target_price: alert.target_price,
                live_price,
                condition: alert.condition,
            });
        } else if !meets_target && alert.has_triggered {
            updates.push(PriceAlertTriggerUpdate {
                id: alert.id,
                has_triggered: false,
            });
        }
    }
    PriceAlertEvaluation {
        updates,
        notifications,
    }
}

/// Seconds since the Unix epoch. The one clock read in this module.
pub fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A random v4 UUID in canonical dashed form. Callers treat it as opaque.
pub fn new_transaction_id() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    // Version 4, variant 1, as RFC 4122 asks.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Random hex event identifier. Callers treat it as opaque.
pub fn new_event_id() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EvmRecipientPreflightRequest {
    pub chain_name: String,
    pub holding_symbol: String,
    pub token_symbol: Option<String>,
    pub recipient_has_code: Option<bool>,
    pub token_has_code: Option<bool>,
}

/// A reason an EVM send's recipient or token contract looks wrong. Front ends
/// word each one.
///
/// A record with a free-string `code` before, which the app switched on with a
/// `default` that dropped anything it did not know; see
/// [`crate::send::flow::HighRiskSendWarning`] for why that is the wrong shape
/// for a warning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum EvmRecipientPreflightWarning {
    /// The recipient has contract code, so it may not be able to receive
    /// `symbol`.
    RecipientIsContract { chain_name: String, symbol: String },
    /// The recipient's code could not be read.
    RecipientCodeUnknown { chain_name: String },
    /// The token contract has no code on this chain.
    TokenContractMissing {
        chain_name: String,
        token_symbol: String,
    },
    /// The token contract's code could not be read.
    TokenCodeUnknown {
        chain_name: String,
        token_symbol: String,
    },
}

/// Build warning codes for an EVM send's recipient + token contract checks.
/// Swift localizes the codes into user-facing strings.
/// Not exported: `WalletService::evm_recipient_preflight` is the entry point,
/// because the two contract-code probes it needs are core's own network calls.
pub fn evm_recipient_preflight_warnings(
    request: EvmRecipientPreflightRequest,
) -> Vec<EvmRecipientPreflightWarning> {
    let mut warnings = Vec::new();
    let chain_name = request.chain_name;
    match request.recipient_has_code {
        Some(true) => warnings.push(EvmRecipientPreflightWarning::RecipientIsContract {
            chain_name: chain_name.clone(),
            symbol: request.holding_symbol,
        }),
        Some(false) => {}
        None => warnings.push(EvmRecipientPreflightWarning::RecipientCodeUnknown {
            chain_name: chain_name.clone(),
        }),
    }
    if let Some(token_symbol) = request.token_symbol {
        match request.token_has_code {
            Some(false) => warnings.push(EvmRecipientPreflightWarning::TokenContractMissing {
                chain_name,
                token_symbol,
            }),
            None => warnings.push(EvmRecipientPreflightWarning::TokenCodeUnknown {
                chain_name,
                token_symbol,
            }),
            Some(true) => {}
        }
    }
    warnings
}

// ─── Transaction status polling state machine (J+K+L) ───────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusTrackerState {
    pub last_checked_at_unix: Option<f64>,
    pub next_check_at_unix: f64,
    pub consecutive_failures: u32,
    pub reached_finality: bool,
}

impl TransactionStatusTrackerState {
    pub(crate) fn initial(now_unix: f64) -> Self {
        Self {
            last_checked_at_unix: None,
            next_check_at_unix: now_unix,
            consecutive_failures: 0,
            reached_finality: false,
        }
    }
}

/// How often a pending send is re-polled, and when it is given up on.
///
/// Policy, so it lives with the tracker table it schedules rather than crossing
/// the boundary. These were six constants on the iOS side, packed into this
/// record and handed to core on every one of five calls — so the schedule core
/// applied was whatever the caller last said it was, and a second front end
/// would have had to know the same six numbers to get the same behaviour.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusPollConfig {
    pub pending_poll_seconds: f64,
    pub confirmed_poll_seconds: f64,
    pub backoff_max_seconds: f64,
    pub finality_confirmations: u32,
    pub pending_failure_timeout_seconds: f64,
    pub pending_failure_min_failures: u32,
}

impl Default for TransactionStatusPollConfig {
    fn default() -> Self {
        Self {
            pending_poll_seconds: 20.0,
            confirmed_poll_seconds: 300.0,
            backoff_max_seconds: 600.0,
            finality_confirmations: 12,
            pending_failure_timeout_seconds: 60.0 * 60.0,
            pending_failure_min_failures: 6,
        }
    }
}

/// Whether this transaction is due for another status query.
pub fn should_poll_transaction_status(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
) -> bool {
    let tracker = tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    if tracker.reached_finality {
        return false;
    }
    now_unix >= tracker.next_check_at_unix
}

/// Advance a tracker after a successful status query.
pub fn transaction_status_after_successful_poll(
    tracker: Option<TransactionStatusTrackerState>,
    resolved_status_confirmed: bool,
    resolved_status_pending: bool,
    reported_confirmations: Option<u32>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    let mut tracker = tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    tracker.last_checked_at_unix = Some(now_unix);
    tracker.consecutive_failures = 0;
    let reached_finality = if resolved_status_pending {
        false
    } else {
        reported_confirmations.unwrap_or(config.finality_confirmations)
            >= config.finality_confirmations
    };
    if reached_finality {
        tracker.reached_finality = true;
        tracker.next_check_at_unix = now_unix + config.backoff_max_seconds;
    } else if resolved_status_confirmed {
        tracker.next_check_at_unix = now_unix + config.confirmed_poll_seconds;
    } else {
        tracker.next_check_at_unix = now_unix + config.pending_poll_seconds;
    }
    tracker
}

/// Advance a tracker after a failed query. Exponential backoff is capped at
/// `config.backoff_max_seconds`.
pub fn transaction_status_after_failed_poll(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    let mut tracker = tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    tracker.last_checked_at_unix = Some(now_unix);
    tracker.consecutive_failures = tracker.consecutive_failures.saturating_add(1);
    let exponent = tracker.consecutive_failures.saturating_sub(1) as i32;
    let backoff =
        (config.pending_poll_seconds * 2f64.powi(exponent)).min(config.backoff_max_seconds);
    tracker.next_check_at_unix = now_unix + backoff;
    tracker
}

/// Core reads its own transactions to build these; the caller hands over
/// nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StalePendingFailureTransactionInput {
    pub id: String,
    pub created_at_unix: f64,
    pub status_is_pending: bool,
}

/// Pending transactions that have been pending too long *and* have failed to
/// resolve often enough to call it. `failures` is the caller's tracker table;
/// a transaction missing from it has never failed a poll.
pub(crate) fn stale_pending_failure_ids(
    transactions: Vec<StalePendingFailureTransactionInput>,
    failures: &std::collections::HashMap<String, u32>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<String> {
    transactions
        .into_iter()
        .filter(|transaction| {
            if !transaction.status_is_pending {
                return false;
            }
            let age = now_unix - transaction.created_at_unix;
            if age < config.pending_failure_timeout_seconds {
                return false;
            }
            failures.get(&transaction.id).copied().unwrap_or(0)
                >= config.pending_failure_min_failures
        })
        .map(|transaction| transaction.id)
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingStatusInput {
    pub status: String,
    pub confirmations: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingTransactionInput {
    pub id: String,
    pub old_status: String,
    pub old_failure_reason: Option<String>,
    pub old_confirmations: Option<u32>,
    pub resolution: Option<ResolvedPendingStatusInput>,
    pub is_stale_failure: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FailureReasonDisposition {
    None,
    Preserve,
    LocalizedFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingTransactionDecision {
    pub id: String,
    pub new_status: String,
    pub status_changed: bool,
    pub failure_reason_disposition: FailureReasonDisposition,
    /// When set, emit a chain-event indicating the transaction newly reached the
    /// finality threshold this poll cycle. Independent of `status_changed` so it
    /// fires when `confirmed→confirmed` but confirmations crossed the threshold.
    pub reached_finality_confirmations: Option<u32>,
}

/// Stored in `failure_reason` when a pending transaction is given up on.
///
/// A code rather than a sentence: the front end localizes it at render, so a
/// user who changes language does not keep the old one on old records.
pub const FAILURE_REASON_STUCK: &str = "stuckAfterRetries";

/// One chain's resolved statuses, as the network reported them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPendingStatus {
    pub id: String,
    /// `"pending"` / `"confirmed"` / `"failed"`.
    pub status: String,
    pub confirmations: Option<u32>,
    pub receipt_block_number: Option<i64>,
    pub confirmed_network_fee: Option<f64>,
    /// What the EVM receipt says the transaction cost.
    pub evm_receipt_cost: Option<EvmReceiptCost>,
}

/// An EVM receipt's cost, in the units the record stores.
///
/// The receipt reader decoded `gasUsed` and `effectiveGasPrice` and the poll
/// dropped both, so the record's three receipt columns — and the transaction
/// sheet's "Gas Used", "Effective Gas Price" and "Network Fee" rows — were
/// cleared on every pending pass and written by nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EvmReceiptCost {
    /// Gas consumed, as a decimal integer string.
    pub gas_used: String,
    pub effective_gas_price_gwei: f64,
    /// `gas_used × effective_gas_price`, in the chain's gas token.
    pub network_fee: f64,
}

impl EvmReceiptCost {
    /// Both receipt fields as decimal strings, as the EVM client returns them,
    /// on a chain whose gas token has `native_decimals`. `None` when either is
    /// absent or unparseable: a partial cost is not a cost.
    pub fn from_receipt(
        gas_used: Option<&str>,
        effective_gas_price_wei: Option<&str>,
        native_decimals: u8,
    ) -> Option<Self> {
        let gas = gas_used?.parse::<u128>().ok()?;
        let price = effective_gas_price_wei?.parse::<u128>().ok()?;
        Some(Self {
            gas_used: gas.to_string(),
            effective_gas_price_gwei: price as f64 / 1e9,
            network_fee: gas.saturating_mul(price) as f64 / 10f64.powi(i32::from(native_decimals)),
        })
    }
}

/// What changed when resolved statuses were applied — enough for a front end
/// to write an operational event and a notification, and nothing more. The
/// records themselves are already stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
///
/// Statuses are the typed enum. They were strings, with an `emit_event_code`
/// string beside them that restated "the status changed to confirmed or
/// failed" and a `send_status_notification` flag that restated
/// `status_changed`, so a front end parsed the status back out of a string and
/// had three fields to disagree about one fact.
pub struct TransactionStatusChange {
    pub id: String,
    pub chain_name: String,
    pub transaction_hash: Option<String>,
    pub old_status: crate::store::wallet_domain::CoreTransactionStatus,
    pub new_status: crate::store::wallet_domain::CoreTransactionStatus,
    pub status_changed: bool,
    pub reached_finality_confirmations: Option<u32>,
}

pub(crate) fn apply_resolved_pending_transaction_statuses(
    inputs: Vec<ResolvedPendingTransactionInput>,
    trackers: &mut std::collections::HashMap<String, TransactionStatusTrackerState>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<ResolvedPendingTransactionDecision> {
    let mut decisions = Vec::new();
    for input in inputs {
        let decision = if let Some(resolution) = input.resolution {
            let new_status = resolution.status.clone();
            let status_changed = input.old_status != new_status;
            let new_confirmations = resolution.confirmations;
            if new_status != "pending" {
                let tracker = trackers
                    .entry(input.id.clone())
                    .or_insert_with(|| TransactionStatusTrackerState::initial(now_unix));
                tracker.reached_finality = new_confirmations
                    .unwrap_or(config.finality_confirmations)
                    >= config.finality_confirmations;
                tracker.next_check_at_unix = now_unix + config.backoff_max_seconds;
            }
            let failure_reason_disposition = if new_status == "failed" {
                if input.old_failure_reason.is_some() {
                    FailureReasonDisposition::Preserve
                } else {
                    FailureReasonDisposition::LocalizedFallback
                }
            } else {
                FailureReasonDisposition::None
            };
            let reached_finality_confirmations = match (new_confirmations, input.old_confirmations)
            {
                (Some(new_count), old)
                    if new_status == "confirmed"
                        && new_count >= config.finality_confirmations
                        && old.unwrap_or(0) < config.finality_confirmations =>
                {
                    Some(new_count)
                }
                _ => None,
            };
            Some(ResolvedPendingTransactionDecision {
                id: input.id,
                new_status,
                status_changed,
                failure_reason_disposition,
                reached_finality_confirmations,
            })
        } else if input.is_stale_failure {
            let new_status = "failed".to_string();
            let status_changed = input.old_status != new_status;
            let failure_reason_disposition = if input.old_failure_reason.is_some() {
                FailureReasonDisposition::Preserve
            } else {
                FailureReasonDisposition::LocalizedFallback
            };
            Some(ResolvedPendingTransactionDecision {
                id: input.id,
                new_status,
                status_changed,
                failure_reason_disposition,
                reached_finality_confirmations: None,
            })
        } else {
            None
        };
        decisions.extend(decision);
    }
    decisions
}

// ─── N: Chain keypool state (baseline + merge with existing) ──────────────────
//
// The owning keypool service supplies maxima from persisted transactions and
// addresses. These calculations keep allocation indices monotonic.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainKeypoolStateRecord {
    pub next_external_index: i32,
    pub next_change_index: i32,
    pub reserved_receive_index: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainKeypoolBaselineInput {
    pub supports_deep_utxo_discovery: bool,
    pub max_transaction_external_index: Option<i32>,
    pub max_transaction_change_index: Option<i32>,
    pub max_owned_external_index: Option<i32>,
    pub max_owned_change_index: Option<i32>,
    pub has_resolved_address: bool,
}

pub fn derive_chain_keypool_baseline(input: ChainKeypoolBaselineInput) -> ChainKeypoolStateRecord {
    if input.supports_deep_utxo_discovery {
        let max_external = input.max_transaction_external_index.unwrap_or(-1);
        let max_change = input.max_transaction_change_index.unwrap_or(-1);
        let max_owned_external = input.max_owned_external_index.unwrap_or(0);
        let max_owned_change = input.max_owned_change_index.unwrap_or(-1);
        return ChainKeypoolStateRecord {
            next_external_index: std::cmp::max(
                std::cmp::max(max_external, max_owned_external) + 1,
                1,
            ),
            next_change_index: std::cmp::max(std::cmp::max(max_change, max_owned_change) + 1, 0),
            reserved_receive_index: None,
        };
    }
    let next_external_index = if input.has_resolved_address { 1 } else { 0 };
    ChainKeypoolStateRecord {
        next_external_index,
        next_change_index: 0,
        reserved_receive_index: if input.has_resolved_address {
            Some(0)
        } else {
            None
        },
    }
}

pub fn merge_chain_keypool_state(
    baseline: ChainKeypoolStateRecord,
    existing: Option<ChainKeypoolStateRecord>,
) -> ChainKeypoolStateRecord {
    let Some(mut state) = existing else {
        return baseline;
    };
    state.next_external_index =
        std::cmp::max(state.next_external_index, baseline.next_external_index);
    state.next_change_index = std::cmp::max(state.next_change_index, baseline.next_change_index);
    if state.reserved_receive_index.is_none() {
        state.reserved_receive_index = baseline.reserved_receive_index;
    }
    if let Some(reserved) = state.reserved_receive_index {
        state.next_external_index = std::cmp::max(state.next_external_index, reserved + 1);
    }
    state
}

// ─── O: Wallet holdings merge from balance summary ────────────────────────────
//
// Core merges balance summaries by holding identity while retaining price
// metadata and initializing newly discovered holdings.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HoldingMergeExistingInput {
    pub symbol: String,
    pub chain_name: String,
    pub contract_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HoldingMergeIncomingInput {
    pub name: String,
    pub symbol: String,
    pub coingecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct HoldingMergeAppendPayload {
    pub name: String,
    pub symbol: String,
    pub coingecko_id: String,
    pub chain_name: String,
    pub token_standard: String,
    pub contract_address: Option<String>,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum HoldingMergeAction {
    UpdateAmount { existing_index: u32, amount: f64 },
    Append { coin: HoldingMergeAppendPayload },
}

#[cfg(test)]
mod tests;

// ── FFI surface ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod evm_receipt_cost_tests {
    use super::EvmReceiptCost;

    #[test]
    fn a_receipt_cost_needs_both_fields_and_uses_the_gas_token_places() {
        let cost = EvmReceiptCost::from_receipt(Some("21000"), Some("2000000000"), 18).unwrap();
        assert_eq!(cost.gas_used, "21000");
        assert_eq!(cost.effective_gas_price_gwei, 2.0);
        assert!((cost.network_fee - 0.000042).abs() < 1e-15);
        assert_eq!(EvmReceiptCost::from_receipt(Some("21000"), None, 18), None);
        assert_eq!(EvmReceiptCost::from_receipt(None, Some("1"), 18), None);
        assert_eq!(
            EvmReceiptCost::from_receipt(Some("0x5208"), Some("1"), 18),
            None
        );
    }
}
