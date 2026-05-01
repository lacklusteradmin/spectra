use serde::{Deserialize, Serialize};

const ERROR_AMOUNT_BELOW_DUST: &str = "utxo.amountBelowDustThreshold";
const ERROR_FEE_BELOW_RELAY: &str = "utxo.feeBelowRelayPolicy";
const ERROR_TRANSACTION_TOO_LARGE: &str = "utxo.transactionTooLarge";
const ERROR_INSUFFICIENT_FUNDS: &str = "utxo.insufficientFunds";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoEntry {
    pub index: u64,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoFeePolicy {
    pub chain_name: String,
    pub fee_model: String,
    pub dust_threshold: u64,
    pub minimum_relay_fee_rate: Option<f64>,
    pub minimum_absolute_fee: Option<u64>,
    pub minimum_relay_fee_per_kb: Option<f64>,
    pub base_units_per_coin: Option<f64>,
    pub max_standard_transaction_bytes: u64,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
    pub overhead_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoPreviewRequest {
    pub inputs: Vec<UtxoEntry>,
    pub fee_rate: f64,
    pub fee_policy: UtxoFeePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoPreviewPlan {
    pub estimated_transaction_bytes: u64,
    pub estimated_fee: u64,
    pub spendable_value: u64,
    pub input_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoSpendPlanRequest {
    pub inputs: Vec<UtxoEntry>,
    pub target_value: u64,
    pub fee_rate: f64,
    pub fee_policy: UtxoFeePolicy,
    pub max_input_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct UtxoSpendPlan {
    pub selected_indices: Vec<u64>,
    pub total_input_value: u64,
    pub fee: u64,
    pub change: u64,
    pub uses_change_output: bool,
    pub estimated_transaction_bytes: u64,
}

pub fn plan_utxo_preview(request: UtxoPreviewRequest) -> Result<UtxoPreviewPlan, String> {
    if request.inputs.is_empty() {
        return Err(ERROR_INSUFFICIENT_FUNDS.to_string());
    }

    let input_count = request.inputs.len().max(1);
    let total_input_value = request.inputs.iter().map(|entry| entry.value).sum::<u64>();
    let estimated_transaction_bytes = request
        .fee_policy
        .estimate_transaction_bytes(input_count, 1);
    let estimated_fee = request
        .fee_policy
        .estimated_fee(estimated_transaction_bytes, request.fee_rate)?;
    let spendable_value = total_input_value.saturating_sub(estimated_fee);

    Ok(UtxoPreviewPlan {
        estimated_transaction_bytes: estimated_transaction_bytes as u64,
        estimated_fee,
        spendable_value,
        input_count: request.inputs.len() as u64,
    })
}

pub fn plan_utxo_spend(request: UtxoSpendPlanRequest) -> Result<UtxoSpendPlan, String> {
    let inputs = sort_inputs_descending(request.inputs);
    if inputs.is_empty() {
        return Err(ERROR_INSUFFICIENT_FUNDS.to_string());
    }

    let effective_max_input_count = request
        .max_input_count
        .map(|count| (count.max(1)) as usize);
    let mut candidates: Vec<Vec<&UtxoEntry>> = Vec::with_capacity(inputs.len() * 2);

    let mut prefix: Vec<&UtxoEntry> = Vec::with_capacity(inputs.len());
    for entry in &inputs {
        prefix.push(entry);
        if let Some(limit) = effective_max_input_count {
            if prefix.len() > limit {
                continue;
            }
        }
        candidates.push(prefix.clone());
    }

    for entry in &inputs {
        candidates.push(vec![entry]);
    }

    let mut best_plan: Option<UtxoSpendPlan> = None;
    for candidate in candidates {
        let Some(plan) = evaluate_candidate(
            &candidate,
            request.target_value,
            request.fee_rate,
            &request.fee_policy,
        )?
        else {
            continue;
        };

        match &best_plan {
            Some(current_best) if !is_better_plan(&plan, current_best) => {}
            _ => best_plan = Some(plan),
        }
    }

    best_plan.ok_or_else(|| ERROR_INSUFFICIENT_FUNDS.to_string())
}

fn sort_inputs_descending(mut inputs: Vec<UtxoEntry>) -> Vec<UtxoEntry> {
    inputs.sort_by(|lhs, rhs| {
        rhs.value
            .cmp(&lhs.value)
            .then_with(|| lhs.index.cmp(&rhs.index))
    });
    inputs
}

fn evaluate_candidate(
    inputs: &[&UtxoEntry],
    target_value: u64,
    fee_rate: f64,
    fee_policy: &UtxoFeePolicy,
) -> Result<Option<UtxoSpendPlan>, String> {
    if inputs.is_empty() {
        return Ok(None);
    }

    if target_value < fee_policy.dust_threshold {
        return Err(ERROR_AMOUNT_BELOW_DUST.to_string());
    }
    if !fee_policy.is_fee_rate_acceptable(fee_rate) {
        return Err(ERROR_FEE_BELOW_RELAY.to_string());
    }

    let total_input_value = inputs.iter().map(|entry| entry.value).sum::<u64>();
    let fee_with_change = fee_policy.estimated_fee_for_layout(inputs.len(), 2, fee_rate)?;
    if total_input_value >= target_value.saturating_add(fee_with_change) {
        let change = total_input_value - target_value - fee_with_change;
        if change >= fee_policy.dust_threshold {
            let estimated_transaction_bytes =
                fee_policy.estimate_transaction_bytes(inputs.len(), 2);
            if estimated_transaction_bytes as u64 > fee_policy.max_standard_transaction_bytes {
                return Err(ERROR_TRANSACTION_TOO_LARGE.to_string());
            }
            return Ok(Some(UtxoSpendPlan {
                selected_indices: inputs.iter().map(|entry| entry.index).collect(),
                total_input_value,
                fee: fee_with_change,
                change,
                uses_change_output: true,
                estimated_transaction_bytes: estimated_transaction_bytes as u64,
            }));
        }
    }

    let fee_without_change = fee_policy.estimated_fee_for_layout(inputs.len(), 1, fee_rate)?;
    if total_input_value < target_value.saturating_add(fee_without_change) {
        return Ok(None);
    }

    let estimated_transaction_bytes = fee_policy.estimate_transaction_bytes(inputs.len(), 1);
    if estimated_transaction_bytes as u64 > fee_policy.max_standard_transaction_bytes {
        return Err(ERROR_TRANSACTION_TOO_LARGE.to_string());
    }

    let remainder = total_input_value - target_value - fee_without_change;
    Ok(Some(UtxoSpendPlan {
        selected_indices: inputs.iter().map(|entry| entry.index).collect(),
        total_input_value,
        fee: fee_without_change + remainder,
        change: 0,
        uses_change_output: false,
        estimated_transaction_bytes: estimated_transaction_bytes as u64,
    }))
}

fn is_better_plan(lhs: &UtxoSpendPlan, rhs: &UtxoSpendPlan) -> bool {
    if lhs.uses_change_output != rhs.uses_change_output {
        return lhs.uses_change_output && !rhs.uses_change_output;
    }
    if lhs.selected_indices.len() != rhs.selected_indices.len() {
        return lhs.selected_indices.len() < rhs.selected_indices.len();
    }
    if lhs.fee != rhs.fee {
        return lhs.fee < rhs.fee;
    }
    lhs.change < rhs.change
}

impl UtxoFeePolicy {
    fn estimate_transaction_bytes(&self, input_count: usize, output_count: usize) -> usize {
        let input_bytes = self.input_bytes.unwrap_or(148) as usize;
        let output_bytes = self.output_bytes.unwrap_or(34) as usize;
        let overhead_bytes = self.overhead_bytes.unwrap_or(10) as usize;
        overhead_bytes + (input_bytes * input_count) + (output_bytes * output_count)
    }

    fn estimated_fee_for_layout(
        &self,
        input_count: usize,
        output_count: usize,
        fee_rate: f64,
    ) -> Result<u64, String> {
        let estimated_bytes = self.estimate_transaction_bytes(input_count, output_count);
        self.estimated_fee(estimated_bytes, fee_rate)
    }

    fn estimated_fee(&self, estimated_bytes: usize, fee_rate: f64) -> Result<u64, String> {
        match self.fee_model.as_str() {
            "satVbyte" => {
                let minimum_relay_fee_rate = self.minimum_relay_fee_rate.unwrap_or(0.0);
                let minimum_absolute_fee = self.minimum_absolute_fee.unwrap_or(0);
                let applied_fee_rate = fee_rate.max(minimum_relay_fee_rate);
                let fee = ((estimated_bytes as f64) * applied_fee_rate).ceil() as u64;
                Ok(fee.max(minimum_absolute_fee))
            }
            "kilobyte" => {
                let minimum_relay_fee_per_kb = self.minimum_relay_fee_per_kb.unwrap_or_default();
                let base_units_per_coin = self
                    .base_units_per_coin
                    .ok_or_else(|| "UTXO fee policy missing base units.".to_string())?;
                let kilobytes = ((estimated_bytes as f64) / 1000.0).ceil().max(1.0);
                let applied_fee_rate = fee_rate.max(minimum_relay_fee_per_kb);
                Ok((kilobytes * applied_fee_rate * base_units_per_coin).round() as u64)
            }
            _ => Err("Unsupported UTXO fee model.".to_string()),
        }
    }

    fn is_fee_rate_acceptable(&self, fee_rate: f64) -> bool {
        match self.fee_model.as_str() {
            "satVbyte" => fee_rate >= self.minimum_relay_fee_rate.unwrap_or(0.0),
            "kilobyte" => fee_rate >= self.minimum_relay_fee_per_kb.unwrap_or(0.0),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        plan_utxo_preview, plan_utxo_spend, UtxoEntry, UtxoFeePolicy, UtxoPreviewRequest,
        UtxoSpendPlanRequest, ERROR_AMOUNT_BELOW_DUST, ERROR_INSUFFICIENT_FUNDS,
    };

    fn sat_policy() -> UtxoFeePolicy {
        UtxoFeePolicy {
            chain_name: "Litecoin".to_string(),
            fee_model: "satVbyte".to_string(),
            dust_threshold: 1_000,
            minimum_relay_fee_rate: Some(1.0),
            minimum_absolute_fee: Some(1_000),
            minimum_relay_fee_per_kb: None,
            base_units_per_coin: None,
            max_standard_transaction_bytes: 100_000,
            input_bytes: None,
            output_bytes: None,
            overhead_bytes: None,
        }
    }

    fn kb_policy() -> UtxoFeePolicy {
        UtxoFeePolicy {
            chain_name: "Dogecoin".to_string(),
            fee_model: "kilobyte".to_string(),
            dust_threshold: 1_000_000,
            minimum_relay_fee_rate: None,
            minimum_absolute_fee: None,
            minimum_relay_fee_per_kb: Some(0.01),
            base_units_per_coin: Some(100_000_000.0),
            max_standard_transaction_bytes: 100_000,
            input_bytes: None,
            output_bytes: None,
            overhead_bytes: None,
        }
    }

    #[test]
    fn computes_sat_vbyte_preview_with_absolute_minimum() {
        let preview = plan_utxo_preview(UtxoPreviewRequest {
            inputs: vec![UtxoEntry {
                index: 0,
                value: 1_000_000,
            }],
            fee_rate: 1.0,
            fee_policy: sat_policy(),
        })
        .expect("preview should be planned");

        assert_eq!(preview.estimated_fee, 1_000);
        assert_eq!(preview.spendable_value, 999_000);
    }

    #[test]
    fn computes_kilobyte_preview() {
        let preview = plan_utxo_preview(UtxoPreviewRequest {
            inputs: vec![UtxoEntry {
                index: 0,
                value: 5_000_000,
            }],
            fee_rate: 0.02,
            fee_policy: kb_policy(),
        })
        .expect("dogecoin preview should be planned");

        assert_eq!(preview.estimated_fee, 2_000_000);
        assert_eq!(preview.spendable_value, 3_000_000);
    }

    #[test]
    fn prefers_fewer_inputs_and_change_output_when_possible() {
        let plan = plan_utxo_spend(UtxoSpendPlanRequest {
            inputs: vec![
                UtxoEntry {
                    index: 0,
                    value: 150_000,
                },
                UtxoEntry {
                    index: 1,
                    value: 80_000,
                },
                UtxoEntry {
                    index: 2,
                    value: 70_000,
                },
            ],
            target_value: 90_000,
            fee_rate: 1.0,
            fee_policy: sat_policy(),
            max_input_count: None,
        })
        .expect("spend plan should be selected");

        assert_eq!(plan.selected_indices, vec![0u64]);
        assert!(plan.uses_change_output);
    }

    #[test]
    fn rejects_targets_below_dust_threshold() {
        let error = plan_utxo_spend(UtxoSpendPlanRequest {
            inputs: vec![UtxoEntry {
                index: 0,
                value: 150_000,
            }],
            target_value: 500,
            fee_rate: 1.0,
            fee_policy: sat_policy(),
            max_input_count: None,
        })
        .expect_err("dust targets should be rejected");

        assert_eq!(error, ERROR_AMOUNT_BELOW_DUST);
    }

    #[test]
    fn returns_insufficient_funds_when_no_candidate_satisfies_target() {
        let error = plan_utxo_spend(UtxoSpendPlanRequest {
            inputs: vec![UtxoEntry {
                index: 0,
                value: 10_000,
            }],
            target_value: 9_500,
            fee_rate: 20.0,
            fee_policy: sat_policy(),
            max_input_count: None,
        })
        .expect_err("planner should reject impossible spends");

        assert_eq!(error, ERROR_INSUFFICIENT_FUNDS);
    }
}

// ── FFI surface (relocated from ffi.rs) ──────────────────────────────────

#[uniffi::export]
pub fn core_plan_utxo_preview(
    request: UtxoPreviewRequest,
) -> Result<UtxoPreviewPlan, crate::SpectraBridgeError> {
    Ok(plan_utxo_preview(request)?)
}

#[uniffi::export]
pub fn core_plan_utxo_spend(
    request: UtxoSpendPlanRequest,
) -> Result<UtxoSpendPlan, crate::SpectraBridgeError> {
    Ok(plan_utxo_spend(request)?)
}
