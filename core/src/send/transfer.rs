use crate::registry::SendRule;

/// Whether a holding can be sent.
///
/// Expressed against the coin core actually holds. It replaced a twin that
/// took an index-and-flags record the caller assembled; that twin and the
/// planner it served are gone.
pub(crate) fn can_send_coin(
    coin: &crate::store::wallet_domain::AssetHolding,
    has_signing_material: bool,
    chain_supports_send: bool,
    is_live_chain: bool,
    token_preferences: &[crate::store::wallet_domain::CoreTokenPreferenceEntry],
) -> bool {
    if !chain_supports_send {
        return false;
    }
    if is_live_chain && !has_signing_material {
        return false;
    }
    let Some(chain) = crate::registry::Chain::from_str_id(&coin.chain_id) else {
        return false;
    };
    let is_known_token = || {
        token_preferences
            .iter()
            .any(|entry| entry.is_enabled && entry.token.matches_holding(coin))
    };
    match chain.send_rule() {
        SendRule::Any => true,
        SendRule::NativeOnly => coin.is_native(),
        SendRule::NativeOrSupportedToken => coin.is_native() || is_known_token(),
        SendRule::SupportedSolanaCoin => coin.is_native() || is_known_token(),
    }
}
// ── FFI surface ─────────────────────────────────────────────────────────────
