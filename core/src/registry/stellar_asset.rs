//! Stellar credit assets: a code of one to twelve letters and digits and
//! the account that issues it, written `CODE:ISSUER` as SEP-11 writes it.

use crate::api::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StellarAsset {
    pub code: String,
    pub issuer: String,
}

impl StellarAsset {
    pub fn new(code: &str, issuer: &str) -> Result<Self, ApiError> {
        if code.is_empty() || code.len() > 12 || !code.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ApiError::invalid("Invalid Stellar asset code"));
        }
        // Account IDs are base32, written in capitals; their checksum covers
        // the decoded bytes, so a lowercase spelling is the same account.
        let issuer = issuer.to_ascii_uppercase();
        crate::derivation::stellar::decode_stellar_address(&issuer)
            .map_err(|_| ApiError::invalid("Invalid Stellar asset issuer"))?;
        Ok(Self {
            code: code.to_string(),
            issuer,
        })
    }

    pub fn parse(identifier: &str) -> Result<Self, ApiError> {
        let (code, issuer) = identifier
            .trim()
            .split_once(':')
            .ok_or_else(|| ApiError::invalid("A Stellar asset is written CODE:ISSUER"))?;
        Self::new(code, issuer)
    }

    pub fn identifier(&self) -> String {
        format!("{}:{}", self.code, self.issuer)
    }

    /// `credit_alphanum4` for codes of up to four characters, otherwise
    /// `credit_alphanum12`: the asset type Horizon reports and XDR encodes.
    pub fn asset_type(&self) -> &'static str {
        if self.code.len() <= 4 {
            "credit_alphanum4"
        } else {
            "credit_alphanum12"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CIRCLE: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

    #[test]
    fn assets_are_code_and_issuer() {
        let usdc = StellarAsset::parse(&format!("USDC:{}", CIRCLE.to_lowercase())).unwrap();
        assert_eq!(usdc.identifier(), format!("USDC:{CIRCLE}"));
        assert_eq!(usdc.asset_type(), "credit_alphanum4");
        let long = StellarAsset::new("LONGCODE12AB", CIRCLE).unwrap();
        assert_eq!(long.asset_type(), "credit_alphanum12");
        assert_ne!(
            StellarAsset::new("usdc", CIRCLE).unwrap(),
            usdc,
            "codes are case-sensitive"
        );
        for bad in [
            "USDC".to_string(),
            format!(":{CIRCLE}"),
            format!("THIRTEENCHARS:{CIRCLE}"),
            format!("US-D:{CIRCLE}"),
            "USDC:GNOTANACCOUNT".to_string(),
        ] {
            assert!(StellarAsset::parse(&bad).is_err(), "{bad}");
        }
    }
}
