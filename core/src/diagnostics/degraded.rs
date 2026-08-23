//! Degraded-chain-sync message pattern matching and normalization.
//!
//! Decides which message template applies to a sync failure and normalizes the
//! detail string. Locale lookup and formatting stay on the platform side.

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

/// Everything core can say about one degraded-sync detail string.
///
/// Three exports asked three questions about the same string — is it really a
/// success report, what does it look like with its suffix stripped, and which
/// localization key does it match. The caller asked one, two or all three
/// depending on the path, which is how the two paths came to disagree about
/// whether to normalise before matching.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DegradedDetail {
    /// The message with its "Last good sync: …" or "No prior successful sync
    /// yet." suffix stripped.
    pub normalized: String,
    /// The `localizedStoreFormat` key this detail matches, if any. Matched
    /// against `normalized`, so a detail carrying a suffix still resolves.
    pub template_key: Option<String>,
    /// The "failure" detail is in fact a partial-success report.
    pub indicates_live_success: bool,
}

#[uniffi::export]
pub fn diagnostics_classify_degraded_detail(detail: String) -> DegradedDetail {
    let normalized = normalize_degraded_detail(&detail);
    DegradedDetail {
        template_key: degraded_detail_template_key(&normalized),
        indicates_live_success: detail_indicates_live_success(&detail),
        normalized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A detail arriving with its suffix still attached resolves to a template.
    ///
    /// It did not, on one of the two paths. `markChainDegraded` matched the raw
    /// string and `localizedDegradedMessage` normalised first, so the same
    /// message localized on one route and fell through to the raw English on
    /// the other. Classification normalises once and matches the normalized
    /// form, so both routes get the same answer.
    #[test]
    fn a_detail_with_its_suffix_still_matches_its_template() {
        let raw = "Ethereum providers are unavailable. Using cached balances and history. \
                   Last good sync: 10:00 AM";
        assert!(
            degraded_detail_template_key(raw).is_none(),
            "the raw form does not match — that is the bug this classification removes"
        );
        let classified = diagnostics_classify_degraded_detail(raw.to_string());
        assert_eq!(
            classified.template_key.as_deref(),
            Some("%@ providers are unavailable. Using cached balances and history.")
        );
        assert!(!classified.indicates_live_success);
    }

    /// "Partially reachable" is a success report wearing a failure's clothes.
    #[test]
    fn a_partial_success_is_reported_as_one() {
        let classified = diagnostics_classify_degraded_detail(
            "Solana providers are partially reachable. Showing the latest available balances."
                .to_string(),
        );
        assert!(classified.indicates_live_success);
        assert!(classified.template_key.is_some());
    }

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
        assert!(detail_indicates_live_success(
            "foo partial provider failures bar"
        ));
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
