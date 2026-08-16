use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct WalletTransferAvailability {
    pub wallet_id: String,
    pub send_holding_indices: Vec<u64>,
    pub receive_holding_indices: Vec<u64>,
    pub receive_chains: Vec<String>,
}

use crate::registry::SendRule;

/// Whether a holding can be sent.
///
/// Expressed against the coin core actually holds. It replaced a twin that
/// took an index-and-flags record the caller assembled; that twin and the
/// planner it served are gone.
pub(crate) fn can_send_coin(
    coin: &crate::store::wallet_domain::CoreCoin,
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
    let Some(chain) = crate::registry::Chain::from_display_name(&coin.chain_name) else {
        return true;
    };
    let is_tracked_token = || {
        token_preferences.iter().any(|entry| {
            entry.is_enabled
                && entry.symbol == coin.symbol
                && entry.chain.chain_name() == coin.chain_name
                && match coin.contract_address.as_deref() {
                    Some(contract) => entry.contract_address.eq_ignore_ascii_case(contract),
                    None => true,
                }
        })
    };
    match chain.send_rule() {
        SendRule::Any => true,
        SendRule::NativeOnly => coin.symbol == chain.coin_symbol(),
        SendRule::NativeOrSupportedToken => {
            coin.symbol == chain.coin_symbol() || is_tracked_token()
        }
        SendRule::SupportedSolanaCoin => coin.symbol == chain.coin_symbol() || is_tracked_token(),
    }
}
// ── FFI surface ─────────────────────────────────────────────────────────────
