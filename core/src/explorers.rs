//! Explorers, embedded from `explorers.toml`.
//!
//! An explorer is a page the app opens, never a service it requests, so it has
//! no API, capabilities or health probe. Those belong to `endpoints.toml`.

use crate::registry::Chain;
use serde::Deserialize;
use std::sync::LazyLock;

static EXPLORERS_TOML: &str = include_str!("../data/explorers.toml");

/// Where a transaction's hash goes in `tx_url`.
const HASH_PLACEHOLDER: &str = "{hash}";
/// Where an address goes in `address_url`.
const ADDRESS_PLACEHOLDER: &str = "{address}";

/// A network's explorer: the pages its transactions and addresses open in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct Explorer {
    pub chain_id: crate::registry::Chain,
    /// What the explorer calls itself, such as "Etherscan".
    pub name: String,
    /// The transaction page, with `{hash}` where the hash goes.
    pub tx_url: String,
    /// The address page, with `{address}` where the address goes, or `None`
    /// where the explorer has none (Monero's, which cannot see one).
    #[serde(default)]
    pub address_url: Option<String>,
}

/// One page on a network's explorer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ExplorerLink {
    pub name: String,
    pub url: String,
}

impl Explorer {
    fn link(&self, template: &str, placeholder: &str, value: &str) -> Option<ExplorerLink> {
        let value = value.trim();
        (!value.is_empty()).then(|| ExplorerLink {
            name: self.name.clone(),
            url: template.replace(placeholder, value),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFile {
    explorers: Vec<Explorer>,
}

static EXPLORERS: LazyLock<Vec<Explorer>> = LazyLock::new(|| {
    load(EXPLORERS_TOML).unwrap_or_else(|error| panic!("explorers.toml: {error}"))
});

fn load(text: &str) -> Result<Vec<Explorer>, String> {
    let explorers = toml::from_str::<TomlFile>(text)
        .map_err(|e| e.to_string())?
        .explorers;
    let mut seen = std::collections::HashSet::new();
    let template = |url: &str, placeholder: &str| {
        url.starts_with("https://") && url.matches(placeholder).count() == 1
    };
    for explorer in &explorers {
        let id = explorer.chain_id;
        if !seen.insert(id) {
            return Err(format!("{id}: more than one explorer"));
        }
        if explorer.name.trim().is_empty() {
            return Err(format!("{id}: empty name"));
        }
        if !template(&explorer.tx_url, HASH_PLACEHOLDER) {
            return Err(format!(
                "{id}: tx_url must be HTTPS with exactly one {HASH_PLACEHOLDER}"
            ));
        }
        if let Some(url) = &explorer.address_url
            && !template(url, ADDRESS_PLACEHOLDER)
        {
            return Err(format!(
                "{id}: address_url must be HTTPS with exactly one {ADDRESS_PLACEHOLDER}"
            ));
        }
    }
    Ok(explorers)
}

impl Chain {
    pub fn explorer(self) -> Option<&'static Explorer> {
        EXPLORERS.iter().find(|e| e.chain_id == self)
    }
}

/// Every network's explorer, in registry order.
#[uniffi::export]
pub fn explorers() -> Vec<Explorer> {
    Chain::all().filter_map(Chain::explorer).cloned().collect()
}

/// The explorer page for one transaction, or `None` when the network has no
/// explorer or there is no hash to show.
#[uniffi::export]
pub fn transaction_explorer_link(
    chain_id: crate::registry::Chain,
    transaction_hash: String,
) -> Option<ExplorerLink> {
    let explorer = chain_id.explorer()?;
    explorer.link(&explorer.tx_url, HASH_PLACEHOLDER, &transaction_hash)
}

/// The explorer page for one address, or `None` when the network's explorer
/// has no address page or there is no address to show.
#[uniffi::export]
pub fn address_explorer_link(
    chain_id: crate::registry::Chain,
    address: String,
) -> Option<ExplorerLink> {
    let explorer = chain_id.explorer()?;
    explorer.link(
        explorer.address_url.as_deref()?,
        ADDRESS_PLACEHOLDER,
        &address,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_file_loads_one_explorer_per_listed_network() {
        assert_eq!(explorers().len(), EXPLORERS.len());
    }

    /// Every explorer has an address page but Monero's, which cannot see
    /// one: its addresses are not on its chain.
    #[test]
    fn every_explorer_but_moneros_has_an_address_page() {
        for explorer in explorers() {
            assert_eq!(
                explorer.address_url.is_none(),
                explorer.chain_id.mainnet_counterpart() == Chain::Monero,
                "{}",
                explorer.chain_id
            );
        }
    }

    #[test]
    fn the_hash_is_placed_where_the_template_says() {
        assert_eq!(
            transaction_explorer_link(crate::registry::Chain::Ethereum, " 0xabc ".into()),
            Some(ExplorerLink {
                name: "Etherscan".into(),
                url: "https://etherscan.io/tx/0xabc".into()
            })
        );
        assert_eq!(
            transaction_explorer_link(crate::registry::Chain::Aptos, "0xabc".into())
                .map(|l| l.url)
                .as_deref(),
            Some("https://explorer.aptoslabs.com/txn/0xabc?network=mainnet")
        );
        assert_eq!(
            transaction_explorer_link(crate::registry::Chain::Ethereum, "  ".into()),
            None
        );
        assert_eq!(
            transaction_explorer_link(crate::registry::Chain::AptosTestnet, "0xabc".into())
                .map(|l| l.url)
                .as_deref(),
            Some("https://explorer.aptoslabs.com/txn/0xabc?network=testnet")
        );
        assert_eq!(
            transaction_explorer_link(crate::registry::Chain::KaspaTestnet, "abc".into()),
            None
        );
    }

    #[test]
    fn the_address_is_placed_where_the_template_says() {
        assert_eq!(
            address_explorer_link(Chain::Solana, " So1 ".into()).map(|l| l.url),
            Some("https://solscan.io/account/So1".into())
        );
        assert_eq!(
            address_explorer_link(Chain::SolanaDevnet, "So1".into()).map(|l| l.url),
            Some("https://solscan.io/account/So1?cluster=devnet".into())
        );
        assert_eq!(address_explorer_link(Chain::Ethereum, " ".into()), None);
        assert_eq!(address_explorer_link(Chain::Monero, "4A".into()), None);
    }

    #[test]
    fn malformed_files_are_refused() {
        let row = |chain: &str, url: &str| {
            format!("[[explorers]]\nchain_id = \"{chain}\"\nname = \"X\"\ntx_url = \"{url}\"\n")
        };
        let ok = row("bitcoin", "https://x.example/{hash}");
        for text in [
            row("nowhere", "https://x.example/{hash}"),
            row("bitcoin", "https://x.example/tx/"),
            row("bitcoin", "http://x.example/{hash}"),
            row("bitcoin", "https://x.example/{hash}/{hash}"),
            ok.clone() + &row("bitcoin", "https://y.example/{hash}"),
            ok.clone() + "label = \"Open\"\n",
            ok.clone() + "address_url = \"https://x.example/a/\"\n",
            ok.clone() + "address_url = \"http://x.example/{address}\"\n",
            ok.clone() + "address_url = \"https://x.example/{hash}\"\n",
            "[[explorers]]\nchain_id = \"bitcoin\"\nname = \" \"\ntx_url = \"https://x.example/{hash}\"\n".into(),
        ] {
            assert!(load(&text).is_err(), "accepted {text}");
        }
        assert!(load(&ok).is_ok());
        assert!(load(&(ok + "address_url = \"https://x.example/a/{address}\"\n")).is_ok());
    }
}
