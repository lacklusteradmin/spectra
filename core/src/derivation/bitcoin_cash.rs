//! Bitcoin Cash: CashAddr payloads as Blockbook reads them. Its keys and
//! addresses derive with the Bitcoin family (`bitcoin::derive_from_seed_phrase`).

/// Blockbook accepts CashAddr payloads without their network prefix.
pub(crate) fn normalize_bch_address(address: &str) -> String {
    address
        .strip_prefix("bitcoincash:")
        .or_else(|| address.strip_prefix("bchtest:"))
        .unwrap_or(address)
        .to_string()
}
