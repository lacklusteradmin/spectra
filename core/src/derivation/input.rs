//! Raw secret editor text retains whitespace. Unknown options are refused.
use crate::store::wallet_domain::CoreWalletDerivationOverrides;

#[derive(Clone, Default, serde::Deserialize, uniffi::Record)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WalletDerivationInput {
    pub passphrase: String,
    pub hmac_key: String,
}

#[uniffi::export]
pub fn core_parse_wallet_derivation_input(
    input: WalletDerivationInput,
) -> CoreWalletDerivationOverrides {
    fn exact(s: String) -> Option<String> {
        (!s.is_empty()).then_some(s)
    }
    CoreWalletDerivationOverrides {
        passphrase: exact(input.passphrase),
        hmac_key: exact(input.hmac_key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_fields_refuse_and_secret_whitespace_is_preserved() {
        for input in [
            r#"{"iterationCount":"abc"}"#,
            r#"{"iterationCount":2048}"#,
            r#"{"curve":"ed25519"}"#,
        ] {
            assert!(serde_json::from_str::<WalletDerivationInput>(input).is_err());
            assert!(serde_json::from_str::<CoreWalletDerivationOverrides>(input).is_err());
        }
        let parsed = core_parse_wallet_derivation_input(WalletDerivationInput {
            passphrase: " secret ".into(),
            hmac_key: " key ".into(),
        });
        assert_eq!(parsed.passphrase.as_deref(), Some(" secret "));
        assert_eq!(parsed.hmac_key.as_deref(), Some(" key "));
    }
}
