// Rust-owned mirrors of Swift's PersistenceModels.swift.
//
// These types define the canonical on-disk JSON shape for Spectra's persisted
// state. Swift still owns the write path for most stores during the migration,
// but the serde shapes here are the contract — any change must be made in
// lockstep on both sides and covered by a roundtrip test in `models::tests`.
//
// JSON encoding conventions that must be preserved:
// * Keys use camelCase (matches Swift CodingKey defaults).
// * UUIDs are uppercase strings (Swift JSONEncoder default).
// * Dates written via the vanilla `JSONEncoder()` are `f64` seconds since
//   Swift's reference date (2001-01-01T00:00:00Z). Types using that strategy
//   keep `createdAt` as `f64`.
// * Version integers and store wrappers match the Swift `currentVersion`
//   constants.
//
// Wallet / Coin / WalletStore shapes are intentionally not re-declared here —
// see `crate::migration::LegacyPersisted*`, which already mirrors them and is
// exercised by its own roundtrip test.

pub mod models;
