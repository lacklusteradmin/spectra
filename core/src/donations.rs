//! Donation addresses, embedded from `donations.toml`.
//!
//! These are funds destinations, so they are checked when the file loads
//! rather than trusted: a known mainnet, one address per network, and an
//! address that is already its chain's valid, normalized form.

use serde::Deserialize;
use std::sync::LazyLock;

static DONATIONS_TOML: &str = include_str!("../data/donations.toml");

/// Where a donation on one network goes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct DonationDestination {
    pub chain_id: crate::registry::Chain,
    pub address: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFile {
    donations: Vec<DonationDestination>,
}

static DONATIONS: LazyLock<Vec<DonationDestination>> = LazyLock::new(|| {
    load(DONATIONS_TOML).unwrap_or_else(|error| panic!("donations.toml: {error}"))
});

fn load(text: &str) -> Result<Vec<DonationDestination>, String> {
    let donations = toml::from_str::<TomlFile>(text)
        .map_err(|e| e.to_string())?
        .donations;
    let mut seen = std::collections::HashSet::new();
    for donation in &donations {
        let id = donation.chain_id;
        if id.is_testnet() {
            return Err(format!("{id}: a donation address belongs on a mainnet"));
        }
        if !seen.insert(id) {
            return Err(format!("{id}: more than one donation address"));
        }
        if !crate::send::flow::is_valid_send_address(id, donation.address.clone())
            || crate::send::flow::normalize_address(id, &donation.address) != donation.address
        {
            return Err(format!(
                "{id}: {:?} is not a valid address",
                donation.address
            ));
        }
    }
    Ok(donations)
}

/// The Donate screen's addresses, in file order.
#[uniffi::export]
pub fn donation_destinations() -> Vec<DonationDestination> {
    DONATIONS.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_addresses_load() {
        assert!(!donation_destinations().is_empty());
    }

    #[test]
    fn unsafe_destinations_are_refused() {
        let row = |chain: &str, address: &str| {
            format!("[[donations]]\nchain_id = \"{chain}\"\naddress = \"{address}\"\n")
        };
        let eth = "0xefa039ed09c3fe6aeceb89b365b2740e4050365c";
        for text in [
            row("nowhere", eth),
            row("ethereum-sepolia", eth),
            row("ethereum", "0xefa039ed09c3fe6aeceb89b365b2740e4050365"),
            row("bitcoin", eth),
            row("ethereum", &format!(" {eth}")),
            row("ethereum", eth) + &row("ethereum", eth),
        ] {
            assert!(load(&text).is_err(), "accepted {text}");
        }
        assert!(load(&row("ethereum", eth)).is_ok());
    }
}
