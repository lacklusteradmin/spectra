//! Cardano address decoding and validation (Shelley bech32 / Byron base58).

pub fn validate_cardano_address(address: &str) -> bool {
    // Shelley bech32 addresses start with "addr1" (mainnet) or "addr_test1" (testnet).
    if address.starts_with("addr1") || address.starts_with("addr_test1") {
        return bech32::decode(address).is_ok();
    }
    // Byron base58 addresses.
    bs58::decode(address).with_check(None).into_vec().is_ok()
}

/// Decode a Cardano Shelley bech32 or Byron base58 address to raw bytes.
pub(crate) fn decode_cardano_addr_bytes(address: &str) -> Result<Vec<u8>, String> {
    if address.starts_with("addr1") || address.starts_with("addr_test1") {
        bech32::decode(address)
            .map(|(_, data)| data)
            .map_err(|e| format!("cardano bech32 decode: {e}"))
    } else {
        // Byron base58 — strip check bytes (last 4).
        let decoded = bs58::decode(address)
            .with_check(None)
            .into_vec()
            .map_err(|e| format!("cardano base58 decode: {e}"))?;
        Ok(decoded)
    }
}
