//! Artwork is catalog metadata, addressed by deployment identity.
pub fn deployment_artwork_name(deployment_id: Option<String>) -> String {
    deployment_id
        .and_then(|id| {
            crate::catalog::tokens::catalog()
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
        for token in crate::catalog::tokens::catalog() {
            assert_eq!(
                deployment_artwork_name(Some(token.holding_template().deployment_id())),
                token.artwork_name,
                "{}",
                token.deployment_id
            );
        }
        let mut impostor = crate::catalog::tokens::catalog()
            .iter()
            .find(|t| t.symbol == "USDC" && !t.contract.is_empty())
            .unwrap()
            .holding_template();
        impostor.contract_address = Some("unknown-contract".into());
        assert_eq!(deployment_artwork_name(Some(impostor.deployment_id())), "");
    }
    #[test]
    fn every_named_mark_ships_a_file() {
        let icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../icons");
        let ships =
            |library: &str, name: &str| icons.join(library).join(format!("{name}.svg")).is_file();
        for token in crate::catalog::tokens::catalog() {
            if !token.artwork_name.is_empty() {
                assert!(
                    ships("crypto", &token.artwork_name),
                    "{}",
                    token.artwork_name
                );
            }
        }
        for chain in crate::catalog::chains::catalog() {
            assert!(!chain.artwork_name.is_empty());
            assert!(
                ships("chain", &chain.artwork_name),
                "{}",
                chain.artwork_name
            );
        }
    }
}
