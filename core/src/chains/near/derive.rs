//! NEAR account-id validation (named + implicit hex accounts).

pub fn validate_near_address(address: &str) -> bool {
    // NEAR accounts: named (alice.near, sub.alice.near) or implicit (64 hex chars).
    if address.len() == 64 && address.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    // Named account: 2-64 chars, alphanumeric, hyphen, underscore, dot.
    !address.is_empty()
        && address.len() <= 64
        && address
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}
