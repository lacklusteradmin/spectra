//! Artwork is catalog metadata, addressed by token or network identity.
#[uniffi::export]
pub fn token_artwork_name(token_id: String) -> String {
    crate::tokens::catalog()
        .iter()
        .find(|t| t.token_id == token_id)
        .map(|t| t.artwork_name.clone())
        .unwrap_or_default()
}
#[uniffi::export]
pub fn chain_artwork_name(chain_id: String) -> String {
    crate::chains::chain_by_str_id(&chain_id)
        .map(|c| c.artwork_name.clone())
        .unwrap_or_default()
}
#[uniffi::export]
pub fn holding_artwork_name(holding: crate::store::wallet_domain::AssetHolding) -> String {
    deployment_artwork_name(Some(holding.deployment_id()))
}

#[uniffi::export]
pub fn deployment_artwork_name(deployment_id: Option<String>) -> String {
    deployment_id
        .and_then(|id| {
            crate::tokens::catalog()
                .iter()
                .find(|t| t.deployment_id == id)
        })
        .map(|t| t.artwork_name.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_uses_identity_and_unknown_contracts_cannot_borrow_a_symbol() {
        for token in crate::tokens::catalog() {
            assert_eq!(
                token_artwork_name(token.token_id.clone()),
                token.artwork_name
            );
            assert_eq!(
                holding_artwork_name(token.holding_template()),
                token.artwork_name,
                "{}",
                token.deployment_id
            );
        }
        let mut impostor = crate::tokens::catalog()
            .iter()
            .find(|t| t.symbol == "USDC" && !t.contract.is_empty())
            .unwrap()
            .holding_template();
        impostor.contract_address = Some("unknown-contract".into());
        assert_eq!(holding_artwork_name(impostor), "");
        assert_eq!(token_artwork_name("USDC".into()), "");
        assert_eq!(chain_artwork_name("base".into()), "base");
    }
    #[test]
    fn every_named_mark_ships_a_file() {
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../icons/crypto");
        for token in crate::tokens::catalog() {
            if !token.artwork_name.is_empty() {
                assert!(icons.join(format!("{}.svg", token.artwork_name)).is_file());
            }
        }
        for chain in crate::chains::catalog() {
            assert!(!chain.artwork_name.is_empty());
            assert!(icons.join(format!("{}.svg", chain.artwork_name)).is_file());
        }
    }
}
