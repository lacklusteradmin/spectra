//! Checked input/output accounting shared by fixed-fee UTXO signers.
pub(super) fn checked_change(
    values: impl IntoIterator<Item = u64>,
    amount: u64,
    fee: u64,
) -> Result<u64, String> {
    if amount == 0 {
        return Err("send amount must be positive".into());
    }
    let total = values.into_iter().try_fold(0u64, |total, value| {
        total.checked_add(value).ok_or("input total overflow")
    })?;
    let needed = amount.checked_add(fee).ok_or("amount plus fee overflow")?;
    total
        .checked_sub(needed)
        .ok_or_else(|| "insufficient inputs for amount plus fee".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accounting_refuses_invalid_totals() {
        assert_eq!(checked_change([40, 60], 70, 10).unwrap(), 20);
        assert_eq!(checked_change([100], 90, 10).unwrap(), 0);
        assert!(checked_change([], 1, 0).is_err());
        assert!(checked_change([100], 91, 10).is_err());
        assert!(checked_change([u64::MAX, 1], 1, 0).is_err());
        assert!(checked_change([u64::MAX], u64::MAX, 1).is_err());
        assert!(checked_change([100], 0, 1).is_err());
    }
}
