# Rust Derivation Core

This crate is the runtime derivation core behind `Derivation/`.

## Current State

- Swift derivation entry points call into Rust through FFI.
- Rust supports the full current `SeedDerivationChain` set in the bridge.
- The boundary is raw C ABI (not JSON).
- The Rust library is built and linked during Xcode builds.

## Boundary

- Header: `include/spectra_derivation.h`
- Symbols:
  - `spectra_derivation_derive`
  - `spectra_derivation_response_free`
  - `spectra_derivation_buffer_free`

## Safety Model

- Secret fields are passed as UTF-8 byte buffers over FFI.
- Swift zeroizes temporary UTF-8 seed/passphrase/hmac buffers after call.
- Rust zeroizes sensitive owned strings on drop and zeroizes derived seed bytes.
- Rust allocates response buffers and Swift must free with `spectra_derivation_response_free`.

## Ownership Split

- Swift owns:
  - presets/catalog selection
  - app-facing request assembly
  - UI-facing validation/normalization behavior
- Rust owns:
  - mnemonic normalization/parsing
  - seed derivation
  - key derivation
  - address/public/private output derivation

## Important Rule

FFI numeric IDs in `spectra_derivation.h` are treated as stable contract values.
Do not change them casually.
