// Pure token-identifier / endpoint normalization helpers lifted from Swift's
// AppState. Previously these lived as instance methods (string munging, URL
// validation, CSV parsing) that didn't touch any mutable state. Moving them
// here shrinks AppState and makes the logic testable without a MainActor.

/// Canonicalize a `0x…` hex string: strip leading zeroes, keep at least one.
/// Unchanged if the prefix is not `0x`.
#[uniffi::export]
pub fn canonical_aptos_hex_address(value: String) -> String {
    if !value.starts_with("0x") {
        return value;
    }
    let hex_portion = &value[2..];
    let trimmed: String = hex_portion.chars().skip_while(|c| *c == '0').collect();
    let canonical = if trimmed.is_empty() { "0".to_string() } else { trimmed };
    format!("0x{canonical}")
}

/// Normalize an Aptos coin-type / identifier string: lowercase, then rewrite
/// every `0x…` hex run in place with [`canonical_aptos_hex_address`].
#[uniffi::export]
pub fn normalize_aptos_token_identifier(value: String) -> String {
    let lowercased = value.trim().to_lowercase();
    if lowercased.is_empty() {
        return String::new();
    }
    let bytes = lowercased.as_bytes();
    let mut out = String::with_capacity(lowercased.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && &bytes[i..i + 2] == b"0x" {
            let start = i;
            let mut end = i + 2;
            while end < bytes.len() && (bytes[end] as char).is_ascii_hexdigit() {
                end += 1;
            }
            out.push_str(&canonical_aptos_hex_address(lowercased[start..end].to_string()));
            i = end;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Canonicalize just a Sui package identifier: `0x…` with trimmed zeroes.
#[uniffi::export]
pub fn normalize_sui_package_component(value: String) -> String {
    if !value.starts_with("0x") {
        return value;
    }
    let hex_portion = &value[2..];
    let trimmed: String = hex_portion.chars().skip_while(|c| *c == '0').collect();
    let canonical = if trimmed.is_empty() { "0".to_string() } else { trimmed };
    format!("0x{canonical}")
}

/// Normalize a Sui token identifier: lowercase, split on `::`, canonicalize
/// the first (package) component, rejoin.
#[uniffi::export]
pub fn normalize_sui_token_identifier(value: String) -> String {
    let trimmed = value.trim().to_lowercase();
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split("::").collect();
    let first = match parts.first() {
        Some(p) => *p,
        None => return trimmed,
    };
    let normalized_package = normalize_sui_package_component(first.to_string());
    if parts.len() <= 1 {
        return normalized_package;
    }
    let mut out = normalized_package;
    for rest in &parts[1..] {
        out.push_str("::");
        out.push_str(rest);
    }
    out
}

/// Normalize a dashboard asset's contract address for grouping/equality.
/// Returns `None` for empty/whitespace input. For Sui/Aptos uses the chain's
/// canonical identifier form; otherwise lowercases the trimmed value.
#[uniffi::export]
pub fn normalize_dashboard_contract_address(
    contract_address: Option<String>,
    chain_name: String,
    _token_standard: String,
) -> Option<String> {
    let raw = contract_address?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    match chain_name.as_str() {
        "Sui" => Some(normalize_sui_token_identifier(trimmed.to_string())),
        "Aptos" => Some(normalize_aptos_token_identifier(trimmed.to_string())),
        _ => Some(trimmed.to_lowercase()),
    }
}

/// Return the package portion of a (possibly-normalized) aptos identifier.
#[uniffi::export]
pub fn aptos_package_identifier(value: Option<String>) -> String {
    let normalized = normalize_aptos_token_identifier(value.unwrap_or_default());
    match normalized.split_once(':') {
        Some((head, _)) => head.to_string(),
        None => normalized,
    }
}

// ---- Bitcoin Esplora endpoint parsing / validation ----

#[uniffi::export]
pub fn parse_bitcoin_esplora_endpoints(raw: String) -> Vec<String> {
    raw.split([',', '\n', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Returns validation error message if any endpoint is malformed, else `None`.
#[uniffi::export]
pub fn bitcoin_esplora_endpoints_validation_error(raw: String) -> Option<String> {
    for endpoint in parse_bitcoin_esplora_endpoints(raw) {
        if !is_valid_http_url(&endpoint) {
            return Some(
                "Bitcoin Esplora endpoints must be valid http(s) URLs separated by commas."
                    .to_string(),
            );
        }
    }
    None
}

#[uniffi::export]
pub fn ethereum_rpc_endpoint_validation_error(endpoint: String) -> Option<String> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_valid_http_url(trimmed) {
        None
    } else {
        Some("Enter a valid http or https RPC URL.".to_string())
    }
}

#[uniffi::export]
pub fn monero_backend_base_url_validation_error(endpoint: String) -> Option<String> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_valid_http_url(trimmed) {
        None
    } else {
        Some("Enter a valid http or https Monero backend URL.".to_string())
    }
}

fn is_valid_http_url(s: &str) -> bool {
    // Minimal-but-correct parser matching the semantics the Swift code needed:
    // scheme in {http, https} and a non-empty host.
    let Some(scheme_end) = s.find("://") else {
        return false;
    };
    let scheme = &s[..scheme_end].to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return false;
    }
    let after = &s[scheme_end + 3..];
    if after.is_empty() {
        return false;
    }
    // Host ends at '/', '?', '#', or end. Strip any userinfo ('@').
    let host_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = &after[..host_end];
    let host_part = match authority.rsplit_once('@') {
        Some((_, h)) => h,
        None => authority,
    };
    // Strip port if present.
    let host = match host_part.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => h,
        Some(_) => return false,
        None => host_part,
    };
    !host.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hex_strips_leading_zeros() {
        assert_eq!(canonical_aptos_hex_address("0x0000abcd".into()), "0xabcd");
        assert_eq!(canonical_aptos_hex_address("0x0".into()), "0x0");
        assert_eq!(canonical_aptos_hex_address("0x00000".into()), "0x0");
        assert_eq!(canonical_aptos_hex_address("nohex".into()), "nohex");
    }

    #[test]
    fn normalize_aptos_rewrites_embedded_hex() {
        assert_eq!(
            normalize_aptos_token_identifier("0x001::coin::USDC".into()),
            "0x1::coin::usdc"
        );
        assert_eq!(normalize_aptos_token_identifier("   ".into()), "");
    }

    #[test]
    fn normalize_sui_roundtrip() {
        assert_eq!(
            normalize_sui_token_identifier("0x0002::Foo::bar".into()),
            "0x2::foo::bar"
        );
        assert_eq!(normalize_sui_token_identifier("plaintext".into()), "plaintext");
    }

    #[test]
    fn parse_endpoints_splits_and_trims() {
        assert_eq!(
            parse_bitcoin_esplora_endpoints("a, b ;c\nd,,".into()),
            vec!["a", "b", "c", "d"]
        );
    }

    #[test]
    fn endpoint_validation() {
        assert_eq!(
            bitcoin_esplora_endpoints_validation_error("https://x.example,https://y.example".into()),
            None
        );
        assert!(
            bitcoin_esplora_endpoints_validation_error("notaurl".into()).is_some()
        );
        assert_eq!(ethereum_rpc_endpoint_validation_error("".into()), None);
        assert!(ethereum_rpc_endpoint_validation_error("ftp://x".into()).is_some());
        assert_eq!(
            ethereum_rpc_endpoint_validation_error("https://rpc.example/abc".into()),
            None
        );
    }

    #[test]
    fn dashboard_contract_dispatch() {
        assert_eq!(
            normalize_dashboard_contract_address(Some("  ".into()), "Ethereum".into(), "ERC-20".into()),
            None
        );
        assert_eq!(
            normalize_dashboard_contract_address(None, "Ethereum".into(), "ERC-20".into()),
            None
        );
        assert_eq!(
            normalize_dashboard_contract_address(Some("0xABCDEF".into()), "Ethereum".into(), "ERC-20".into()),
            Some("0xabcdef".into())
        );
        assert_eq!(
            normalize_dashboard_contract_address(Some("0x0002::Foo::bar".into()), "Sui".into(), "Native".into()),
            Some("0x2::foo::bar".into())
        );
        assert_eq!(
            normalize_dashboard_contract_address(Some("0x001::coin::USDC".into()), "Aptos".into(), "Native".into()),
            Some("0x1::coin::usdc".into())
        );
    }

    #[test]
    fn aptos_package_id_splits_first_colon() {
        assert_eq!(
            aptos_package_identifier(Some("0x1:coin".into())),
            "0x1"
        );
        assert_eq!(aptos_package_identifier(None), "");
    }
}
