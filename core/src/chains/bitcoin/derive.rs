//! Bitcoin address validation (P2PKH / P2WPKH / P2TR / nested-SegWit) using
//! the `bitcoin` crate's parser + per-network check.

use std::str::FromStr;

use bitcoin::Address;

use super::bitcoin_network_for_mode;

/// Validate a Bitcoin address string against the given network mode.
/// Returns the canonical form if valid.
pub fn validate_bitcoin_address(address: &str, network_mode: &str) -> Option<String> {
    let network = bitcoin_network_for_mode(network_mode);
    Address::from_str(address)
        .ok()
        .and_then(|a| a.require_network(network).ok())
        .map(|a| a.to_string())
}
