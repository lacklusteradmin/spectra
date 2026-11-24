//! Bitcoin Gold: address validation. Its keys and P2PKH (G…) addresses
//! derive with the Bitcoin family (`bitcoin::derive_from_seed_phrase`).

#[cfg(test)]
mod tests {
    use crate::derivation::utxo_address::parse_utxo_address;
    use crate::registry::Chain;

    #[test]
    fn rejects_btc_p2pkh() {
        // BTC P2PKH starts with '1' (version 0x00); BTG must reject.
        assert!(
            parse_utxo_address(Chain::BitcoinGold, "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").is_err()
        );
    }
}
