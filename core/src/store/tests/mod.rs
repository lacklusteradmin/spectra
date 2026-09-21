//! Store behavior and persistence regressions.
//!
//! Keep coverage for validation, identities, transaction deltas, failed writes,
//! concurrent updates and reopening persisted data. Registry metadata and pure
//! type rules are tested beside their owners, without snapshots of old callers.

mod a_bad_row_is_not_a_bad_database;
mod address_book;
mod built_in_tokens;
mod dashboard_groups;
mod feed_state_follows_commands;
mod import_address_validation;
mod keypool;
mod network_token_identity;
mod open_state_idempotence;
mod operational_events;
mod owned_state;
mod pinned_dashboard_assets;
mod resident_state_round_trip;
mod send_execution_shape;
mod send_rules;
mod status_trackers;
mod tracked_tokens_persist;
mod transaction_merge;
mod transaction_merge_strategy;
mod transaction_store;
mod wallet_derived_state;
mod wallet_import;
mod wallet_model_conversion;
mod wallet_update_if_present;
mod wallet_view_model;
