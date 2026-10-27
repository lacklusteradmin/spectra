pub mod artwork;
pub mod persistence_models;
mod price_alerts;
pub use price_alerts::{
    PriceAlertEvaluation, PriceAlertEvaluationPrice, PriceAlertNotification, PriceAlertRejection,
    PriceAlertRule, PriceAlertTriggerUpdate, evaluate_price_alerts,
};
pub mod state;
mod status_tracker;
pub use status_tracker::*;
pub mod transaction_merge;
pub mod wallet_domain;

pub use artwork::{chain_artwork_name, deployment_artwork_name, token_artwork_name};

/// Seconds since the Unix epoch. The one clock read in this module.
pub fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A random v4 UUID in canonical dashed form. Callers treat it as opaque.
pub fn new_transaction_id() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    // Version 4, variant 1, as RFC 4122 asks.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Random hex event identifier. Callers treat it as opaque.
pub fn new_event_id() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests;
