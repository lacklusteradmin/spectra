# Rust Core

This crate is the Rust application core behind the Swift UI layers.

## Current State

- Swift calls Rust through generated UniFFI bindings.
- The public bridge surface is JSON-oriented for complex payloads.
- Rust supports the current derivation, catalog, localization, state, fetch-planning, and send-planning APIs used by the app.
- The Rust library and Swift bindings are generated during Xcode builds.

## Boundary

- Rust exports are declared with `#[uniffi::export]`.
- Generated Swift bindings live under `Derivation/Core/Generated/`.
- Swift bridge adapters stay in:
  - `Derivation/Core/WalletRustDerivationBridge.swift`
  - `Derivation/Core/WalletRustAppCoreBridge.swift`
  - `ProviderCatalog/Core/WalletRustEndpointCatalogBridge.swift`

## Safety Model

- Sensitive request fields are encoded as Swift strings and passed through UniFFI-generated bindings.
- Rust zeroizes sensitive owned strings on drop and zeroizes derived seed bytes.
- Errors cross the boundary as typed UniFFI errors instead of manual status structs.

## Ownership Split

- Swift owns:
  - UI flow orchestration
  - app-facing request assembly
  - presentation-specific validation and fallback behavior
- Rust owns:
  - derivation and address generation
  - static catalogs and localization loading
  - state reduction and migration
  - fetch/send planning logic

## Important Rule

Numeric chain, network, curve, and algorithm IDs are still shared contract values between Swift and Rust.
Keep them aligned when changing either side of the bridge.
