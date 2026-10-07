//! Monero block heights for dates, from monthly checkpoints read off two
//! independent public daemons (`core/data/monero-checkpoints.json`, made by
//! `scripts/generate-monero-checkpoints.py`).
//!
//! A restore height is where a Monero wallet's scan starts; outputs received
//! below it are never found. So a date becomes the height of the last
//! checkpoint at or before it — never one after — for a Polyseed's birthday
//! and for a wallet created now.

use std::sync::OnceLock;

use crate::SpectraBridgeError;
use crate::registry::Chain;

/// Monero's block time since its v2 hard fork.
const SECONDS_PER_BLOCK: u64 = 120;
/// How far a typed restore height may run past the estimated tip: 30 days of
/// blocks, for the drift of an estimate extrapolated from the last checkpoint.
const TIP_MARGIN: u64 = 21_600;

struct Checkpoint {
    height: u64,
    timestamp: u64,
}

fn checkpoints(chain: Chain) -> &'static [Checkpoint] {
    static TABLE: OnceLock<(Vec<Checkpoint>, Vec<Checkpoint>)> = OnceLock::new();
    let (mainnet, stagenet) = TABLE.get_or_init(|| {
        let raw: serde_json::Value =
            serde_json::from_str(include_str!("../data/monero-checkpoints.json"))
                .expect("monero-checkpoints.json is valid JSON");
        let read = |key: &str| {
            raw[key]
                .as_array()
                .expect("a checkpoint list per network")
                .iter()
                .map(|point| Checkpoint {
                    height: point[0].as_u64().expect("a height"),
                    timestamp: point[1].as_u64().expect("a timestamp"),
                })
                .collect::<Vec<_>>()
        };
        (read("monero"), read("monero-stagenet"))
    });
    match chain {
        Chain::Monero => mainnet,
        Chain::MoneroStagenet => stagenet,
        _ => &[],
    }
}

/// The height of the last checkpoint at or before `unix`: a scan from there
/// misses nothing received after that moment. Zero before the first one.
pub(crate) fn height_at_or_before(chain: Chain, unix: u64) -> u64 {
    checkpoints(chain)
        .iter()
        .take_while(|point| point.timestamp <= unix)
        .last()
        .map_or(0, |point| point.height)
}

/// The chain height `now`, extrapolated from the last checkpoint at the block
/// target. An estimate: real blocks drift from the target.
pub(crate) fn estimated_tip(chain: Chain, now: u64) -> u64 {
    checkpoints(chain).last().map_or(0, |point| {
        point.height + now.saturating_sub(point.timestamp) / SECONDS_PER_BLOCK
    })
}

/// Refuse a typed restore height past the chain's current height: a scan
/// that starts in the future misses every output received before it, which
/// is the whole of a restored wallet's history.
pub(crate) fn check_restore_height(chain: Chain, height: u64) -> Result<(), SpectraBridgeError> {
    if height > estimated_tip(chain, now()) + TIP_MARGIN {
        return Err(SpectraBridgeError::invalid(
            "That restore height is past the chain's current height.",
        ));
    }
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The restore height for a Monero wallet created now: no earlier output can
/// belong to it, so its scan need not start further back.
#[uniffi::export]
pub fn monero_new_wallet_restore_height(chain: Chain) -> Result<u64, SpectraBridgeError> {
    if chain.mainnet_counterpart() != Chain::Monero {
        return Err(SpectraBridgeError::invalid(
            "Only Monero wallets take a restore height.",
        ));
    }
    Ok(height_at_or_before(chain, now()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_the_last_checkpoint_at_or_before_it() {
        for chain in [Chain::Monero, Chain::MoneroStagenet] {
            let points = checkpoints(chain);
            assert!(points.len() > 50, "{chain}");
            assert!(points.windows(2).all(|pair| {
                pair[0].height < pair[1].height && pair[0].timestamp < pair[1].timestamp
            }));
            let middle = &points[points.len() / 2];
            assert_eq!(height_at_or_before(chain, middle.timestamp), middle.height);
            assert_eq!(
                height_at_or_before(chain, middle.timestamp - 1),
                points[points.len() / 2 - 1].height
            );
            assert_eq!(height_at_or_before(chain, 0), 0);
            // Polyseed's epoch, November 2021, is covered.
            assert!(points[0].timestamp <= 1_635_768_000);
        }
    }

    #[test]
    fn a_new_wallet_starts_at_or_before_now_and_only_monero_has_one() {
        let height = monero_new_wallet_restore_height(Chain::Monero).unwrap();
        assert!(height > 3_000_000);
        assert!(height <= estimated_tip(Chain::Monero, now()));
        assert!(monero_new_wallet_restore_height(Chain::Bitcoin).is_err());
    }

    #[test]
    fn a_height_past_the_tip_is_refused() {
        let tip = estimated_tip(Chain::Monero, now());
        assert!(check_restore_height(Chain::Monero, tip).is_ok());
        assert!(check_restore_height(Chain::Monero, tip * 10).is_err());
    }
}
