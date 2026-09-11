//! Store regressions, grouped by domain.
mod snapshot;

// ── Owned application state ──────────────────────────────────────────────────
//
// `WalletService` owns `CoreAppState`. These pin the contract every front end
// relies on: a command mutates core's copy, persists it, and the next process
// sees the change.

mod owned_state;

// ── Address book (core-owned) ────────────────────────────────────────────────
//
// Validation lives in the reducer, not at the call site: a front end that
// forgets to check cannot save a duplicate or an invalid address.

mod address_book;

// ── Owned transaction store ──────────────────────────────────────────────────
//
// Transactions live in SQLite, not in `CoreAppState` — history is unbounded and
// `apply_state_command` returns the whole state. What core owns here is the
// writes and, critically, the added/updated/removed delta: whether a record is
// new is a property of the store, and callers used to have to guess.

mod transaction_store;

mod transaction_merge;

// ── CoreImportedWallet → WalletSummary ───────────────────────────────────────
//
// The app's wallet record converted into the model core computes with. What
// these pin is which fields survive and which deliberately do not.

mod wallet_model_conversion;

mod wallet_view_model;

mod wallet_update_if_present;

mod open_state_idempotence;

/// Confirmation-poll backoff. Core owns the tracker table; these cover the
/// intents Swift drives it with.
mod status_trackers;

/// Importing is a core operation now: it plans, builds and stores in one call.
mod wallet_import;

/// Keypool reservation. The property that matters is that an index is never
/// handed out twice — so these hammer it concurrently.
#[cfg(test)]
/// The per-chain send shape, transcribed from the ten Swift call sites that
/// carried it inline. These values decide whether a send is refused for
/// insufficient fee and how the fee reaches the signer, so the transcription
/// is pinned rather than trusted.
mod send_execution_shape;

/// The dashboard's rows. Grouping the same asset across chains and ordering
/// them are domain rules; they lived in the shell and the CLI could not reach
/// them.
mod dashboard_groups;

/// Operational events: core stamps, caps and persists them.
mod operational_events;

/// The built-in token catalog and the merge that folds it into stored
/// preferences. Both moved into core when the planner went away.
mod built_in_tokens;

mod keypool;

/// Pinned dashboard assets are user choices that must survive a restart, so
/// core owns them like any other setting.
mod pinned_dashboard_assets;

/// Adding a field to `AppSettings` must not make an already-written state file
/// unreadable. This is not hypothetical: adding
/// `pinned_dashboard_asset_symbols` without `#[serde(default)]` made every
/// launch on an existing database fail with "missing field".
mod settings_forward_compatibility;

/// The merge strategy per chain used to be eighteen Swift wrappers. These pin
/// the registry to exactly what those wrappers did, so the move cannot change
/// behaviour for any chain that already had one.
mod transaction_merge_strategy;

/// `supportedEVMToken` used to exclude a chain's native asset with six
/// hand-written chain/symbol pairs. It asks the registry now, so these pin the
/// registry to exactly what those six said.
mod evm_native_symbols;

/// The send rule per chain used to be a `match` on chain-name strings inside
/// `can_send_holding`. These pin the registry to exactly what it said.
mod send_rules;

/// `wallet_derived_state` replaced two planners that returned holding indices
/// for the caller to resolve. These cover the parts that indirection made easy
/// to get wrong: grouping, network-mode-dependent identity, and send gating.
mod wallet_derived_state;

/// `resolvedAddress(for:chainName:)` used to be a 24-case switch mapping each
/// chain name to its own accessor. It asks core for the derivation chain now,
/// so core must answer for every chain that switch listed — a missing entry
/// would silently return no address rather than fail to compile.
mod seed_derivation_chain_coverage;

/// Import addresses are validated for every chain, not for some of them.
///
/// The iOS path validated three chains and passed the other twenty-one through
/// untouched, so whether a malformed address reached storage depended only on
/// which chain it was typed under.
mod import_address_validation;

/// Known tokens survive a reopen.
///
/// They did not. `token_preferences` is a field on `CoreAppState`, but
/// `app_state_save` wrote `settings`, `wallets` and the address book and never
/// this — so every launch loaded an empty list, and `PLAN.md`'s claim that
/// they "arrive with the rest of the state" was false. Nothing caught it
/// because no test reopened the database after tracking a token, and the app
/// keeps them in memory for the life of a session.
mod tracked_tokens_persist;

/// Every collection on `CoreAppState` survives a reopen.
///
/// Written before the price-alert command, because the token list shipped
/// unpersisted for exactly as long as nobody reopened the database after
/// writing one. This walks every collection rather than the newest, so the
/// next field added is covered by the same test or fails it.
mod resident_state_round_trip;

/// One unreadable collection must not take the wallet list with it.
mod a_bad_row_is_not_a_bad_database;
