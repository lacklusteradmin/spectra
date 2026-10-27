//! Polling a pending transaction for its final status: when each is due,
//! how a poll's answer or failure moves its schedule, what a resolved status
//! changes, and an EVM receipt's cost.

use serde::{Deserialize, Serialize};

use super::wallet_domain;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusTrackerState {
    pub last_checked_at_unix: Option<f64>,
    pub next_check_at_unix: f64,
    pub consecutive_failures: u32,
    pub polling_complete: bool,
}

impl TransactionStatusTrackerState {
    pub(crate) fn initial(now_unix: f64) -> Self {
        Self {
            last_checked_at_unix: None,
            next_check_at_unix: now_unix,
            consecutive_failures: 0,
            polling_complete: false,
        }
    }
}

/// How often a pending send is re-polled, with bounded provider-error backoff.
///
/// Policy, so it lives with the tracker table it schedules rather than crossing
/// the boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusPollConfig {
    pub pending_poll_seconds: f64,
    pub backoff_max_seconds: f64,
}

impl Default for TransactionStatusPollConfig {
    fn default() -> Self {
        Self {
            pending_poll_seconds: 20.0,
            backoff_max_seconds: 600.0,
        }
    }
}

/// Whether this transaction is due for another status query.
pub fn should_poll_transaction_status(
    tracker: Option<TransactionStatusTrackerState>,
    now_unix: f64,
) -> bool {
    let tracker = tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    if tracker.polling_complete {
        return false;
    }
    now_unix >= tracker.next_check_at_unix
}

/// Advance a tracker after a successful status query.
pub fn transaction_status_after_successful_poll(
    tracker: Option<TransactionStatusTrackerState>,
    resolved_status_confirmed: bool,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> TransactionStatusTrackerState {
    let mut tracker = tracker.unwrap_or_else(|| TransactionStatusTrackerState::initial(now_unix));
    tracker.last_checked_at_unix = Some(now_unix);
    tracker.consecutive_failures = 0;
    tracker.polling_complete = resolved_status_confirmed;
    tracker.next_check_at_unix = now_unix + config.pending_poll_seconds;
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

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPendingTransactionInput {
    pub id: String,
    pub old_status: wallet_domain::TransactionStatus,
    pub new_status: wallet_domain::TransactionStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPendingTransactionDecision {
    pub id: String,
    pub new_status: wallet_domain::TransactionStatus,
    pub status_changed: bool,
}

/// One chain's resolved statuses, as the network reported them.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPendingStatus {
    pub id: String,
    pub status: wallet_domain::TransactionStatus,
    pub confirmations: Option<u32>,
    pub receipt_block_number: Option<i64>,
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
    /// Exact decimal gwei.
    pub effective_gas_price_gwei: String,
    /// The complete actual network fee, in the chain's gas token.
    pub network_fee: String,
    /// OP Stack charges, in native units. Both must be known to publish a total.
    pub l1_data_fee: Option<String>,
    pub operator_fee: Option<String>,
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
            effective_gas_price_gwei: crate::amount::decimal::from_units(price, 9),
            network_fee: crate::amount::decimal::from_units(
                gas.checked_mul(price)?,
                u32::from(native_decimals),
            ),
            l1_data_fee: None,
            operator_fee: None,
        })
    }

    pub fn from_rollup_receipt(
        gas_used: Option<&str>,
        effective_gas_price_wei: Option<&str>,
        l1_fee_wei: Option<&str>,
        operator_fee_wei: Option<&str>,
        native_decimals: u8,
    ) -> Option<Self> {
        let mut cost = Self::from_receipt(gas_used, effective_gas_price_wei, native_decimals)?;
        let execution =
            crate::amount::decimal::to_units(&cost.network_fee, u32::from(native_decimals))?;
        let l1 = l1_fee_wei?.parse::<u128>().ok()?;
        let operator = operator_fee_wei?.parse::<u128>().ok()?;
        cost.network_fee = crate::amount::decimal::from_units(
            execution.checked_add(l1)?.checked_add(operator)?,
            u32::from(native_decimals),
        );
        cost.l1_data_fee = Some(crate::amount::decimal::from_units(
            l1,
            u32::from(native_decimals),
        ));
        cost.operator_fee = Some(crate::amount::decimal::from_units(
            operator,
            u32::from(native_decimals),
        ));
        Some(cost)
    }

    /// Recompute a supplied projection's total from exact components.
    pub(crate) fn validated_for_chain(&self, chain: crate::registry::Chain) -> Option<Self> {
        let price =
            crate::amount::decimal::to_units(&self.effective_gas_price_gwei, 9)?.to_string();
        let expected = if chain.evm_rollup_fee_model().is_some() {
            let l1 = crate::amount::decimal::to_units(
                self.l1_data_fee.as_deref()?,
                u32::from(chain.native_decimals()),
            )?
            .to_string();
            let operator = crate::amount::decimal::to_units(
                self.operator_fee.as_deref()?,
                u32::from(chain.native_decimals()),
            )?
            .to_string();
            Self::from_rollup_receipt(
                Some(&self.gas_used),
                Some(&price),
                Some(&l1),
                Some(&operator),
                chain.native_decimals(),
            )?
        } else {
            if self.l1_data_fee.is_some() || self.operator_fee.is_some() {
                return None;
            }
            Self::from_receipt(Some(&self.gas_used), Some(&price), chain.native_decimals())?
        };
        (expected == *self).then_some(expected)
    }
}

/// What changed when resolved statuses were applied — enough for a front end
/// to write an operational event and a notification, and nothing more. The
/// records themselves are already stored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransactionStatusChange {
    pub id: String,
    pub chain_id: crate::registry::Chain,
    pub transaction_hash: Option<String>,
    pub old_status: crate::store::wallet_domain::TransactionStatus,
    pub new_status: crate::store::wallet_domain::TransactionStatus,
    pub status_changed: bool,
    /// Whether to tell the user: the status reached confirmed or failed while
    /// transaction status notifications are on. Core's rule, as for price
    /// alerts and large movements; a front end only delivers it.
    pub notify: bool,
}

pub(crate) fn apply_resolved_pending_transaction_statuses(
    inputs: Vec<ResolvedPendingTransactionInput>,
    trackers: &mut std::collections::HashMap<String, TransactionStatusTrackerState>,
    now_unix: f64,
    config: TransactionStatusPollConfig,
) -> Vec<ResolvedPendingTransactionDecision> {
    inputs
        .into_iter()
        .map(|input| {
            let new_status = input.new_status;
            let status_changed = input.old_status != new_status;
            if new_status != wallet_domain::TransactionStatus::Pending {
                let tracker = trackers
                    .entry(input.id.clone())
                    .or_insert_with(|| TransactionStatusTrackerState::initial(now_unix));
                tracker.polling_complete = true;
                tracker.next_check_at_unix = now_unix + config.backoff_max_seconds;
            }
            ResolvedPendingTransactionDecision {
                id: input.id,
                new_status,
                status_changed,
            }
        })
        .collect()
}

#[cfg(test)]
mod evm_receipt_cost_tests {
    use super::EvmReceiptCost;

    #[test]
    fn a_receipt_cost_needs_both_fields_and_uses_the_gas_token_places() {
        let cost = EvmReceiptCost::from_receipt(Some("21000"), Some("2000000000"), 18).unwrap();
        assert_eq!(cost.gas_used, "21000");
        assert_eq!(cost.effective_gas_price_gwei, "2");
        assert_eq!(cost.network_fee, "0.000042");
        assert_eq!(EvmReceiptCost::from_receipt(Some("21000"), None, 18), None);
        assert_eq!(EvmReceiptCost::from_receipt(None, Some("1"), 18), None);
        assert_eq!(
            EvmReceiptCost::from_receipt(Some("0x5208"), Some("1"), 18),
            None
        );
    }
}
