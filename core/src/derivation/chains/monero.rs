//! Monero address validation.
//!
//! Monero derivation proper (view/spend key pair, stealth addresses) is
//! handled by wallet-rpc, not here — this module only validates addresses.

pub fn validate_monero_address(address: &str) -> bool {
    // Monero mainnet addresses start with '4' (standard) or '8' (subaddress)
    // and are 95 characters in base58 (Monero alphabet).
    if address.len() != 95 {
        return false;
    }
    let first = address.chars().next().unwrap_or('0');
    (first == '4' || first == '8')
        && address.chars().all(|c| {
            "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains(c)
        })
}
