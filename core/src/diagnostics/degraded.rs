//! Degraded-chain-sync message pattern matching and normalization.
//!
//! Centralizes the rules that Swift previously owned in
//! `swift/fetch/DiagnosticsState.swift`. The Swift layer still performs
//! locale-specific string lookup and formatting; this module just decides
//! which template key applies and normalizes detail strings before display.

pub fn detail_indicates_live_success(detail: &str) -> bool {
    detail.contains("partially reachable") || detail.contains("partial provider failures")
}

pub fn normalize_degraded_detail(message: &str) -> String {
    if let Some(pos) = message.find(" Last good sync: ") {
        return message[..pos].trim().to_string();
    }
    let no_prior_suffix = " No prior successful sync yet.";
    if message.ends_with(no_prior_suffix) {
        let cut = message.len() - no_prior_suffix.len();
        return message[..cut].trim().to_string();
    }
    message.trim().to_string()
}

/// Known degraded-detail suffixes and their `localizedStoreFormat` keys.
/// Order matches the Swift table exactly.
const DEGRADED_DETAIL_TEMPLATES: &[(&str, &str)] = &[
    (
        " refresh timed out. Using cached balances and history.",
        "%@ refresh timed out. Using cached balances and history.",
    ),
    (
        " providers are partially reachable. Showing the latest available balances.",
        "%@ providers are partially reachable. Showing the latest available balances.",
    ),
    (
        " providers are unavailable. Using cached balances and history.",
        "%@ providers are unavailable. Using cached balances and history.",
    ),
    (
        " history loaded with partial provider failures.",
        "%@ history loaded with partial provider failures.",
    ),
    (
        " history refresh failed. Using cached history.",
        "%@ history refresh failed. Using cached history.",
    ),
];

/// Returns the `localizedStoreFormat` key if `detail` matches a known template,
/// else `None`. Swift applies the chain-name format + localization.
pub fn degraded_detail_template_key(detail: &str) -> Option<String> {
    for (suffix, key) in DEGRADED_DETAIL_TEMPLATES {
        if detail.ends_with(suffix) {
            return Some((*key).to_string());
        }
    }
    None
}

#[uniffi::export]
pub fn diagnostics_detail_indicates_live_success(detail: String) -> bool {
    detail_indicates_live_success(&detail)
}

#[uniffi::export]
pub fn diagnostics_normalize_degraded_detail(message: String) -> String {
    normalize_degraded_detail(&message)
}

#[uniffi::export]
pub fn diagnostics_degraded_detail_template_key(detail: String) -> Option<String> {
    degraded_detail_template_key(&detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_last_good_sync_suffix() {
        let input = "Ethereum providers are unavailable. Using cached balances and history. Last good sync: 10:00 AM";
        assert_eq!(
            normalize_degraded_detail(input),
            "Ethereum providers are unavailable. Using cached balances and history."
        );
    }

    #[test]
    fn normalize_strips_no_prior_sync_suffix() {
        let input = "Ethereum refresh timed out. Using cached balances and history. No prior successful sync yet.";
        assert_eq!(
            normalize_degraded_detail(input),
            "Ethereum refresh timed out. Using cached balances and history."
        );
    }

    #[test]
    fn normalize_trims_plain_message() {
        assert_eq!(normalize_degraded_detail("  hello  "), "hello");
    }

    #[test]
    fn live_success_detects_known_phrases() {
        assert!(detail_indicates_live_success("foo partially reachable bar"));
        assert!(detail_indicates_live_success("foo partial provider failures bar"));
        assert!(!detail_indicates_live_success("foo timed out"));
    }

    #[test]
    fn template_key_matches_timed_out() {
        let detail = "Ethereum refresh timed out. Using cached balances and history.";
        assert_eq!(
            degraded_detail_template_key(detail).as_deref(),
            Some("%@ refresh timed out. Using cached balances and history.")
        );
    }

    #[test]
    fn template_key_returns_none_for_unknown() {
        assert!(degraded_detail_template_key("some unrelated error").is_none());
    }
}
