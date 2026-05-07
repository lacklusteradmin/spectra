// ── Output types ─────────────────────────────────────────────────────────────

/// Returned by every `derive_<chain>` function.
///
/// Request which fields are populated via the `want_*` booleans on the call
/// site. Fields not requested come back as `None`. `account`, `branch`, and
/// `index` are always populated — they are parsed from the derivation path at
/// zero extra cost and are useful as display metadata or for signing bookkeeping.
#[derive(Debug, uniffi::Record)]
pub struct DerivationResult {
    pub address:         Option<String>,
    pub public_key_hex:  Option<String>,
    pub private_key_hex: Option<String>,
    /// Segment 2 of the BIP-32 path (the account level), hardening stripped.
    pub account:         u32,
    /// Second-to-last segment (the change/branch level).
    pub branch:          u32,
    /// Last segment (the address index).
    pub index:           u32,
}

// ── Script type (UTXO chains) ─────────────────────────────────────────────────

/// Address encoding format for UTXO chains. Fully independent of the
/// derivation path — any path can be paired with any script type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BitcoinScriptType {
    /// Legacy P2PKH (pay-to-public-key-hash). Base58Check, prefix `1`.
    P2pkh,
    /// Nested SegWit P2SH-P2WPKH. Base58Check, prefix `3`.
    P2shP2wpkh,
    /// Native SegWit P2WPKH. Bech32, prefix `bc1q`.
    P2wpkh,
    /// Taproot P2TR. Bech32m, prefix `bc1p`.
    P2tr,
}

// ── Path metadata helper ──────────────────────────────────────────────────────

/// Extract `(account, branch, index)` from a BIP-32 path string.
/// Returns `(0, 0, 0)` for paths with too few segments or no `m/` prefix.
pub(crate) fn parse_path_metadata(path: &str) -> (u32, u32, u32) {
    let trimmed = path.trim();
    let Some(stripped) = trimmed
        .strip_prefix("m/")
        .or_else(|| trimmed.strip_prefix("M/"))
    else {
        return (0, 0, 0);
    };
    let segments: Vec<u32> = stripped
        .split('/')
        .filter_map(|s| s.trim_end_matches('\'').parse().ok())
        .collect();
    let account = segments.get(2).copied().unwrap_or(0);
    let branch = segments
        .get(segments.len().saturating_sub(2))
        .copied()
        .unwrap_or(0);
    let index = segments.last().copied().unwrap_or(0);
    (account, branch, index)
}
