//! Dash: address validation. Its keys and P2PKH (X…) addresses derive with
//! the Bitcoin family (`bitcoin::derive_from_seed_phrase`).

#[cfg(test)]
mod tests {
    use crate::derivation::utxo_address::parse_utxo_address;
    use crate::registry::Chain;

    #[test]
    fn rejects_btc_p2pkh() {
        assert!(parse_utxo_address(Chain::Dash, "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").is_err());
    }
}
