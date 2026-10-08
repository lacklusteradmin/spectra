//! Where a scanning wallet's scan starts: its restore height.
//!
//! Outputs received below a wallet's restore height are never found, so a
//! date becomes a height at or before it — never one after — for a
//! Polyseed's birthday and for a wallet created now. Monero heights come from
//! monthly checkpoints read off two independent public daemons
//! (`core/data/monero-checkpoints.json`, made by
//! `scripts/generate-monero-checkpoints.py`). Zcash's come from one recent
//! block of each network, read off lightwalletd, and the 75-second target
//! spacing, less a week of blocks: a network that ran faster than its target
//! only moves the estimate earlier, which scans more and misses nothing.

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

/// A recent block of each Zcash network: its height and time, from
/// lightwalletd (`docs/audits/zcash-shielded-endpoints-2026-10-07.json`).
const ZCASH_REFERENCE: [(Chain, Checkpoint); 2] = [
    (
        Chain::Zcash,
        Checkpoint {
            height: 3_510_167,
            timestamp: 1_791_428_961,
        },
    ),
    (
        Chain::ZcashTestnet,
        Checkpoint {
            height: 4_477_002,
            timestamp: 1_791_429_034,
        },
    ),
];
/// Zcash's block target since Blossom.
const ZCASH_SECONDS_PER_BLOCK: u64 = 75;
/// A week of Zcash blocks, taken off every estimate so it lands before the
/// moment it estimates.
const ZCASH_MARGIN: u64 = 8_064;

/// Whether wallets on `chain` scan blocks from a restore height. Zcash's
/// shielded pools are scanned; its transparent address is not.
pub(crate) fn takes_restore_height(chain: Chain) -> bool {
    matches!(chain.mainnet_counterpart(), Chain::Monero | Chain::Zcash)
}

/// The height a restored wallet with no restore height scans from: the
/// chain's start for Monero, and Sapling's activation for Zcash, before
/// which no ZIP-32 key could receive.
pub(crate) fn default_restore_height(chain: Chain) -> u64 {
    use zcash_protocol::consensus::{NetworkUpgrade, Parameters};
    chain
        .zcash_network()
        .ok()
        .and_then(|network| network.activation_height(NetworkUpgrade::Sapling))
        .map_or(0, u64::from)
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

/// A height at or before `unix`: a scan from there misses nothing received
/// after that moment. For Monero the last checkpoint at or before it, zero
/// before the first one; for Zcash the estimate from its reference block,
/// never before Sapling's activation.
pub(crate) fn height_at_or_before(chain: Chain, unix: u64) -> u64 {
    if let Some((_, reference)) = ZCASH_REFERENCE.iter().find(|(c, _)| *c == chain) {
        let estimate = if unix >= reference.timestamp {
            reference.height + (unix - reference.timestamp) / ZCASH_SECONDS_PER_BLOCK
        } else {
            reference
                .height
                .saturating_sub((reference.timestamp - unix).div_ceil(ZCASH_SECONDS_PER_BLOCK))
        };
        return estimate
            .saturating_sub(ZCASH_MARGIN)
            .max(default_restore_height(chain));
    }
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
/// is the whole of a restored wallet's history. A Zcash network's blocks
/// keep no steady pace, so its first sync checks the height against the tip
/// it reads instead; here a Zcash height below Sapling's activation is
/// refused, as no shielded key could receive there.
pub(crate) fn check_restore_height(chain: Chain, height: u64) -> Result<(), SpectraBridgeError> {
    if chain.mainnet_counterpart() == Chain::Zcash {
        if height < default_restore_height(chain) {
            return Err(SpectraBridgeError::refused(
                "A Zcash restore height starts at Sapling's activation, block %@.",
                [default_restore_height(chain)],
            ));
        }
        return Ok(());
    }
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

/// The restore height for a wallet created now on `chain`, where wallets
/// scan from one: no earlier output can belong to it, so its scan need not
/// start further back. `None` on a network that does not scan.
#[uniffi::export]
pub fn new_wallet_restore_height(chain: Chain) -> Option<u64> {
    takes_restore_height(chain).then(|| height_at_or_before(chain, now()))
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
    fn a_new_wallet_starts_at_or_before_now_and_only_scanning_chains_have_one() {
        let height = new_wallet_restore_height(Chain::Monero).unwrap();
        assert!(height > 3_000_000);
        assert!(height <= estimated_tip(Chain::Monero, now()));
        assert_eq!(new_wallet_restore_height(Chain::Bitcoin), None);
        // Zcash: a week before the estimate from the reference block.
        let (_, reference) = &ZCASH_REFERENCE[0];
        assert_eq!(
            height_at_or_before(Chain::Zcash, reference.timestamp),
            reference.height - ZCASH_MARGIN
        );
        assert_eq!(
            height_at_or_before(Chain::Zcash, reference.timestamp + 75 * 10_000),
            reference.height + 10_000 - ZCASH_MARGIN
        );
        // Long before the reference, never before Sapling.
        assert_eq!(height_at_or_before(Chain::Zcash, 0), 419_200);
        assert_eq!(height_at_or_before(Chain::ZcashTestnet, 0), 280_000);
        assert!(new_wallet_restore_height(Chain::ZcashTestnet).unwrap() > 4_000_000);
    }

    #[test]
    fn a_zcash_restore_height_starts_at_sapling() {
        assert!(check_restore_height(Chain::Zcash, 419_200).is_ok());
        assert!(check_restore_height(Chain::Zcash, 419_199).is_err());
        assert!(check_restore_height(Chain::ZcashTestnet, 280_000).is_ok());
        assert_eq!(default_restore_height(Chain::Monero), 0);
        assert!(takes_restore_height(Chain::ZcashTestnet) && !takes_restore_height(Chain::Bitcoin));
    }

    #[test]
    fn a_height_past_the_tip_is_refused() {
        let tip = estimated_tip(Chain::Monero, now());
        assert!(check_restore_height(Chain::Monero, tip).is_ok());
        assert!(check_restore_height(Chain::Monero, tip * 10).is_err());
    }
}
