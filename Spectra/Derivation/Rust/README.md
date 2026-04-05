# Rust Derivation Foundation

This crate is the migration target for the `Derivation/` core.

## Current state

- The Swift derivation engine is still the active implementation.
- The Rust crate has an early Bitcoin-only implementation draft.
- The exported C ABI is intentionally small and binary-based.

## Why this exists

The migration goal is:

- keep Swift as the app-facing layer
- move seed derivation and key derivation internals into Rust
- avoid JSON across the runtime FFI boundary
- keep all secret-bearing runtime inputs in binary form

## Planned ownership split

Swift keeps:

- presets
- user-facing validation
- app-facing request assembly
- migration fallback to the current WalletCore-backed path

Rust will own:

- mnemonic normalization
- mnemonic -> seed
- seed -> master key
- derivation path walking
- private/public key derivation
- address derivation

## Expected migration order

1. Make the ABI stable.
2. Make Bitcoin compile and behave correctly in Rust.
3. Implement Ethereum in Rust.
4. Implement Solana in Rust.
5. Switch Swift to call Rust behind the existing engine surface.
6. Remove WalletCore-backed internals only after parity is proven.

## Current Rust scope

- Bitcoin

## Deferred after Bitcoin compiles cleanly

- Ethereum-family
- Solana

## Important note

The FFI ids are now frozen in `include/spectra_derivation.h`.
Do not change those values casually once Swift starts calling the Rust core.
