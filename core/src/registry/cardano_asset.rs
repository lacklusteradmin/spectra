//! Cardano native assets: a minting policy's 28-byte hash and an asset name
//! of at most 32 bytes, written `policy.name` in hex as cardano-cli writes
//! them (`policy` alone for the empty name).

use crate::api::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CardanoAssetId {
    pub policy: [u8; 28],
    pub name: Vec<u8>,
}

impl CardanoAssetId {
    pub fn new(policy_hex: &str, name_hex: &str) -> Result<Self, ApiError> {
        let invalid = || ApiError::invalid("A Cardano native token is written POLICY.NAME in hex");
        let policy = hex::decode(policy_hex)
            .ok()
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .ok_or_else(invalid)?;
        let name = hex::decode(name_hex).map_err(|_| invalid())?;
        if name.len() > 32 {
            return Err(invalid());
        }
        Ok(Self { policy, name })
    }

    /// `policy.name`, `policy.` or `policy`, in either case.
    pub fn parse(identifier: &str) -> Result<Self, ApiError> {
        let identifier = identifier.trim().to_ascii_lowercase();
        let (policy, name) = identifier.split_once('.').unwrap_or((&identifier, ""));
        Self::new(policy, name)
    }

    /// The one spelling identity uses: lowercase hex, and no dot when the
    /// name is empty.
    pub fn identifier(&self) -> String {
        if self.name.is_empty() {
            hex::encode(self.policy)
        } else {
            format!("{}.{}", hex::encode(self.policy), hex::encode(&self.name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_have_one_spelling() {
        let policy = "aa".repeat(28);
        let asset = CardanoAssetId::parse(&format!("{}.CAFE", policy.to_uppercase())).unwrap();
        assert_eq!(asset.identifier(), format!("{policy}.cafe"));
        assert_eq!(
            CardanoAssetId::parse(&format!("{policy}."))
                .unwrap()
                .identifier(),
            policy
        );
        assert_eq!(CardanoAssetId::parse(&policy).unwrap().identifier(), policy);
        for bad in [
            format!("{policy}.c"),
            format!("{policy}.{}", "00".repeat(33)),
            "aa".repeat(27),
            format!("{policy}0"),
            format!("zz{}", &policy[2..]),
        ] {
            assert!(CardanoAssetId::parse(&bad).is_err(), "{bad}");
        }
    }
}
