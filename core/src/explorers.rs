//! Transaction explorers, embedded from `explorers.toml`.
//!
//! An explorer is a page the app opens, never a service it requests, so it has
//! no API, capabilities or health probe. Those belong to `endpoints.toml`.

use crate::registry::Chain;
use serde::Deserialize;
use std::sync::LazyLock;

static EXPLORERS_TOML: &str = include_str!("../data/explorers.toml");

/// Where a transaction's hash goes in `tx_url`.
const HASH_PLACEHOLDER: &str = "{hash}";

/// A network's transaction explorer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct TransactionExplorer {
    pub chain_id: String,
    /// What the explorer calls itself, such as "Etherscan".
    pub name: String,
    /// The transaction page, with `{hash}` where the hash goes.
    pub tx_url: String,
}

/// One transaction's page on its network's explorer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TransactionExplorerLink {
    pub name: String,
    pub url: String,
}

impl TransactionExplorer {
    fn link(&self, transaction_hash: &str) -> Option<TransactionExplorerLink> {
        let hash = transaction_hash.trim();
        (!hash.is_empty()).then(|| TransactionExplorerLink {
            name: self.name.clone(),
            url: self.tx_url.replace(HASH_PLACEHOLDER, hash),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlFile {
    explorers: Vec<TransactionExplorer>,
}

static EXPLORERS: LazyLock<Vec<TransactionExplorer>> = LazyLock::new(|| {
    load(EXPLORERS_TOML).unwrap_or_else(|error| panic!("explorers.toml: {error}"))
});

fn load(text: &str) -> Result<Vec<TransactionExplorer>, String> {
    let explorers = toml::from_str::<TomlFile>(text)
        .map_err(|e| e.to_string())?
        .explorers;
    let mut seen = std::collections::HashSet::new();
    for explorer in &explorers {
        let id = &explorer.chain_id;
        Chain::from_str_id(id).ok_or_else(|| format!("unknown chain_id {id:?}"))?;
        if !seen.insert(id) {
            return Err(format!("{id}: more than one explorer"));
        }
        if explorer.name.trim().is_empty() {
            return Err(format!("{id}: empty name"));
        }
        if !explorer.tx_url.starts_with("https://")
            || explorer.tx_url.matches(HASH_PLACEHOLDER).count() != 1
        {
            return Err(format!(
                "{id}: tx_url must be HTTPS with exactly one {HASH_PLACEHOLDER}"
            ));
        }
    }
    Ok(explorers)
}

impl Chain {
    pub fn transaction_explorer(self) -> Option<&'static TransactionExplorer> {
        EXPLORERS.iter().find(|e| e.chain_id == self.str_id())
    }
}

/// Every network's transaction explorer, in registry order.
#[uniffi::export]
pub fn transaction_explorers() -> Vec<TransactionExplorer> {
    Chain::all()
        .filter_map(Chain::transaction_explorer)
        .cloned()
        .collect()
}

/// The explorer page for one transaction, or `None` when the network has no
/// explorer or there is no hash to show.
#[uniffi::export]
pub fn transaction_explorer_link(
    chain_id: String,
    transaction_hash: String,
) -> Option<TransactionExplorerLink> {
    Chain::from_str_id(&chain_id)?
        .transaction_explorer()?
        .link(&transaction_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_file_loads_one_explorer_per_listed_network() {
        assert_eq!(transaction_explorers().len(), EXPLORERS.len());
        assert!(transaction_explorers().len() > 40);
    }

    #[test]
    fn the_hash_is_placed_where_the_template_says() {
        assert_eq!(
            transaction_explorer_link("ethereum".into(), " 0xabc ".into()),
            Some(TransactionExplorerLink {
                name: "Etherscan".into(),
                url: "https://etherscan.io/tx/0xabc".into()
            })
        );
        assert_eq!(
            transaction_explorer_link("aptos".into(), "0xabc".into())
                .map(|l| l.url)
                .as_deref(),
            Some("https://explorer.aptoslabs.com/txn/0xabc?network=mainnet")
        );
        assert_eq!(
            transaction_explorer_link("ethereum".into(), "  ".into()),
            None
        );
        assert_eq!(
            transaction_explorer_link("nowhere".into(), "0xabc".into()),
            None
        );
        assert_eq!(
            transaction_explorer_link("ethereum-sepolia".into(), "0xabc".into()),
            None
        );
    }

    #[test]
    fn malformed_files_are_refused() {
        let row = |chain: &str, url: &str| {
            format!("[[explorers]]\nchain_id = \"{chain}\"\nname = \"X\"\ntx_url = \"{url}\"\n")
        };
        for text in [
            row("nowhere", "https://x.example/{hash}"),
            row("bitcoin", "https://x.example/tx/"),
            row("bitcoin", "http://x.example/{hash}"),
            row("bitcoin", "https://x.example/{hash}/{hash}"),
            row("bitcoin", "https://x.example/{hash}") + &row("bitcoin", "https://y.example/{hash}"),
            row("bitcoin", "https://x.example/{hash}") + "label = \"Open\"\n",
            "[[explorers]]\nchain_id = \"bitcoin\"\nname = \" \"\ntx_url = \"https://x.example/{hash}\"\n".into(),
        ] {
            assert!(load(&text).is_err(), "accepted {text}");
        }
        assert!(load(&row("bitcoin", "https://x.example/{hash}")).is_ok());
    }
}
