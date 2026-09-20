//! Whether a refresh is due. Pure — no I/O, no clock of its own.
//!
//! Five exports used to live here and in `send/flow.rs`, each taking the piece
//! of state it needed as an argument because core held none of it. The state
//! is `RefreshClock` now, on `WalletService`, and the intervals are settings
//! and policy core owns; what is left here is the arithmetic, which is what a
//! policy module should be.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::store::state::AppSettings;

/// Identity of one independently refreshable wallet history.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct HistoryRefreshKey {
    pub wallet_id: String,
    pub network_id: String,
}
impl HistoryRefreshKey {
    pub(crate) fn new(wallet_id: &str, network_id: &str) -> Self {
        Self {
            wallet_id: wallet_id.to_lowercase(),
            network_id: network_id.to_string(),
        }
    }
}

/// When each kind of refresh last ran, in unix seconds. Absent means never,
/// which is why a fresh clock plans everything.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefreshClock {
    pub full_refresh_at: Option<f64>,
    pub pending_transactions_at: Option<f64>,
    pub live_prices_at: Option<f64>,
    pub background_tick_at: Option<f64>,
    pub(crate) history_at_by_wallet: HashMap<HistoryRefreshKey, f64>,
}

/// Which clock a completed refresh stamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RefreshKind {
    PendingTransactions,
    LivePrices,
    BackgroundTick,
}

impl RefreshClock {
    pub fn record(&mut self, kind: RefreshKind, now_unix: f64) {
        match kind {
            RefreshKind::PendingTransactions => self.pending_transactions_at = Some(now_unix),
            RefreshKind::LivePrices => self.live_prices_at = Some(now_unix),
            RefreshKind::BackgroundTick => self.background_tick_at = Some(now_unix),
        }
    }
    pub(crate) fn record_history(&mut self, key: HistoryRefreshKey, now_unix: f64) {
        self.history_at_by_wallet.insert(key, now_unix);
    }
    fn elapsed(last: Option<f64>, now_unix: f64, interval: f64) -> bool {
        match last {
            Some(at) => now_unix - at >= interval,
            None => true,
        }
    }
}

/// What only the device can tell core.
///
/// Core owns the sync profile, automatic cadence and refresh clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct DeviceConditions {
    pub app_is_active: bool,
    pub is_network_reachable: bool,
    pub is_constrained_network: bool,
    pub is_expensive_network: bool,
    pub is_low_power_mode: bool,
    /// 0…1. Report 1.0 where the platform has no battery to report.
    pub battery_level: f32,
    /// The user is looking at something that shows prices. View state, so the
    /// platform is the only one who knows.
    pub wants_price_refresh: bool,
}

/// What to do this tick, and how long to wait for the next one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePlan {
    pub refresh_pending_transactions: bool,
    pub refresh_live_prices: bool,
    /// The app is in the background and the interval has elapsed.
    pub run_background_tick: bool,
    /// Balances and history may be swept, rather than just pending sends.
    pub allow_heavy_background_work: bool,
    pub poll_seconds: u64,
}

/// Ordinary balances and visible prices refresh automatically every five minutes.
/// Transaction confirmation has its own independent cadence.
pub(crate) const AUTOMATIC_REFRESH_SECONDS: u64 = 5 * 60;
const ACTIVE_POLL_SECONDS: u64 = 30;
const INACTIVE_POLL_SECONDS: u64 = 60;
const BALANCED_PENDING_REFRESH_SECONDS: f64 = 60.0;
const BASE_BACKGROUND_INTERVAL_SECONDS: f64 = 15.0 * 60.0;

/// How often to re-poll a pending send while the app is in front.
fn pending_refresh_interval(settings: &AppSettings) -> f64 {
    use crate::store::state::BackgroundSyncProfile as Profile;
    match settings.background_sync_profile {
        Profile::Conservative => 30.0,
        Profile::Aggressive => 10.0,
        Profile::Balanced => BALANCED_PENDING_REFRESH_SECONDS,
    }
}

/// How long between background ticks, stretched by anything that costs the
/// user power or data.
fn background_interval(conditions: &DeviceConditions) -> f64 {
    let mut interval = BASE_BACKGROUND_INTERVAL_SECONDS;
    if conditions.is_constrained_network || conditions.is_expensive_network {
        interval = interval.max(30.0 * 60.0);
    }
    if conditions.is_low_power_mode {
        interval = interval.max(45.0 * 60.0);
    }
    if conditions.battery_level < 0.20 {
        interval = interval.max(60.0 * 60.0);
    }
    interval
}

/// Whether a background tick may sweep balances and history, or only watch
/// pending sends. The thresholds are the sync profile's.
fn allows_heavy_work(settings: &AppSettings, conditions: &DeviceConditions) -> bool {
    if !conditions.is_network_reachable {
        return false;
    }
    use crate::store::state::BackgroundSyncProfile as Profile;
    match settings.background_sync_profile {
        Profile::Conservative => {
            !conditions.is_constrained_network
                && !conditions.is_expensive_network
                && !conditions.is_low_power_mode
                && conditions.battery_level >= 0.30
        }
        Profile::Balanced => {
            !conditions.is_constrained_network
                && !conditions.is_low_power_mode
                && conditions.battery_level >= 0.20
        }
        Profile::Aggressive => {
            if conditions.is_low_power_mode && conditions.battery_level < 0.15 {
                return false;
            }
            conditions.battery_level >= 0.15
        }
    }
}

pub fn maintenance_plan(
    clock: &RefreshClock,
    settings: &AppSettings,
    conditions: &DeviceConditions,
    has_pending_transaction_work: bool,
    now_unix: f64,
) -> MaintenancePlan {
    if conditions.app_is_active {
        let refresh_pending_transactions = has_pending_transaction_work
            && RefreshClock::elapsed(
                clock.pending_transactions_at,
                now_unix,
                pending_refresh_interval(settings),
            );
        let refresh_live_prices = conditions.wants_price_refresh
            && RefreshClock::elapsed(
                clock.live_prices_at,
                now_unix,
                AUTOMATIC_REFRESH_SECONDS as f64,
            );
        // Nothing pending to watch means the next question can wait a whole
        // price interval. Spinning every thirty seconds to ask "anything to
        // do?" is pure heat on an idle device.
        let poll_seconds = if has_pending_transaction_work {
            ACTIVE_POLL_SECONDS
        } else {
            AUTOMATIC_REFRESH_SECONDS
        };
        return MaintenancePlan {
            refresh_pending_transactions,
            refresh_live_prices,
            run_background_tick: false,
            allow_heavy_background_work: false,
            poll_seconds,
        };
    }

    let run_background_tick = conditions.is_network_reachable
        && RefreshClock::elapsed(
            clock.background_tick_at,
            now_unix,
            background_interval(conditions),
        );
    MaintenancePlan {
        refresh_pending_transactions: false,
        refresh_live_prices: false,
        run_background_tick,
        allow_heavy_background_work: run_background_tick && allows_heavy_work(settings, conditions),
        poll_seconds: INACTIVE_POLL_SECONDS,
    }
}

/// Wallet/network histories whose successful refresh is outside the interval.
pub(crate) fn history_plans(
    clock: &RefreshClock,
    keys: Vec<HistoryRefreshKey>,
    interval: f64,
    now_unix: f64,
) -> Vec<HistoryRefreshKey> {
    keys.into_iter()
        .filter(|key| {
            RefreshClock::elapsed(
                clock.history_at_by_wallet.get(key).copied(),
                now_unix,
                interval,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conditions(active: bool) -> DeviceConditions {
        DeviceConditions {
            app_is_active: active,
            is_network_reachable: true,
            is_constrained_network: false,
            is_expensive_network: false,
            is_low_power_mode: false,
            battery_level: 1.0,
            wants_price_refresh: true,
        }
    }

    #[test]
    fn a_fresh_clock_plans_everything_it_has_work_for() {
        let plan = maintenance_plan(
            &RefreshClock::default(),
            &AppSettings::default(),
            &conditions(true),
            true,
            1_000.0,
        );
        assert!(plan.refresh_pending_transactions);
        assert!(plan.refresh_live_prices);
        assert_eq!(plan.poll_seconds, 30, "pending work is watched every 30s");

        // And with nothing pending the loop sleeps a whole price interval
        // rather than waking twice a minute to ask if there is anything to do.
        let idle = maintenance_plan(
            &RefreshClock::default(),
            &AppSettings::default(),
            &conditions(true),
            false,
            1_000.0,
        );
        assert!(!idle.refresh_pending_transactions);
        assert_eq!(idle.poll_seconds, 300);
    }

    #[test]
    fn a_stamped_clock_waits_out_the_interval() {
        let mut clock = RefreshClock::default();
        clock.record(RefreshKind::PendingTransactions, 1_000.0);
        clock.record(RefreshKind::LivePrices, 1_000.0);
        let settings = AppSettings::default();
        let soon = maintenance_plan(&clock, &settings, &conditions(true), true, 1_030.0);
        assert!(
            !soon.refresh_pending_transactions,
            "60s interval, 30s elapsed"
        );
        assert!(!soon.refresh_live_prices);
        let later = maintenance_plan(&clock, &settings, &conditions(true), true, 1_400.0);
        assert!(later.refresh_pending_transactions);
        assert!(later.refresh_live_prices);
    }

    /// The gate the background tick passes through, and the reason it exists.
    #[test]
    fn a_flat_battery_stops_heavy_background_work() {
        let settings = AppSettings::default();
        let mut low = conditions(false);
        low.battery_level = 0.05;
        let plan = maintenance_plan(&RefreshClock::default(), &settings, &low, false, 1_000.0);
        assert!(
            plan.run_background_tick,
            "watching pending sends is still fine"
        );
        assert!(
            !plan.allow_heavy_background_work,
            "sweeping balances is not"
        );

        let mut offline = conditions(false);
        offline.is_network_reachable = false;
        let plan = maintenance_plan(
            &RefreshClock::default(),
            &settings,
            &offline,
            false,
            1_000.0,
        );
        assert!(!plan.run_background_tick, "no network, nothing to do");
    }

    #[test]
    fn history_cooldown_is_per_wallet_and_network() {
        let mut clock = RefreshClock::default();
        let a = HistoryRefreshKey::new("A", "ethereum");
        let b = HistoryRefreshKey::new("b", "ethereum");
        let testnet = HistoryRefreshKey::new("a", "ethereum-sepolia");
        clock.record_history(a.clone(), 900.0);
        assert_eq!(
            history_plans(
                &clock,
                vec![a.clone(), b.clone(), testnet.clone()],
                300.0,
                1000.0
            ),
            vec![b, testnet]
        );
        assert_eq!(
            history_plans(&clock, vec![a.clone()], 300.0, 1200.0),
            vec![a]
        );
    }
}
