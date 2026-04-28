# FFI boundary

Reference for the Rust↔Swift contract. Read this before:

- adding a new endpoint that crosses to Swift
- changing the shape of a record that already crosses
- regenerating Swift bindings after a Rust change
- debugging a "type mismatch at the boundary" build error

The wallet's logic lives in `core/` (Rust). Swift consumes that logic via UniFFI 0.29, which generates Swift bindings from `#[uniffi::export]`-tagged Rust items. Everything that crosses the boundary is one of: a UniFFI Record, a UniFFI Enum, a UniFFI Object, or a UniFFI Error.

Code references in this doc point at concrete files; treat the file as the source of truth and the doc as the explanation.

---

## The two patterns

There are two ways an endpoint reaches Swift, and they look different on both sides. Pick deliberately.

### Pattern A — typed path (preferred)

A free function in `core/src/ffi.rs` (or a `#[uniffi::export]`-tagged free function in any module) that takes typed records and returns a typed record:

```rust
#[uniffi::export]
pub fn validate_address(request: AddressValidationRequest) -> AddressValidationResult {
    // ...
}
```

UniFFI generates a Swift function with real Swift struct argument and return types. No JSON intermediate; Swift never decodes a string. Type errors in either direction surface at Swift compile time.

**Use this for**: anything that doesn't dispatch on `Chain`. Validation, address probes, decoder helpers, presets, registry lookups, BIP-39 utilities, normalization, transaction merge logic.

**The home for these is `core/src/ffi.rs`** — the file is the canonical FFI surface. New endpoints should land there unless they have a specific reason to live elsewhere (e.g. they're tightly coupled to a `WalletService` field).

### Pattern B — dispatched path

A method on `WalletService` in `core/src/service/mod.rs` that takes `chain_id: u32` plus a per-chain-shaped JSON `Value`, dispatches on `Chain`, and returns a JSON-serialized `String`:

```rust
pub(crate) async fn sign_and_send(
    &self,
    chain_id: u32,
    params: serde_json::Value,
) -> Result<String, SpectraBridgeError> {
    let chain = Chain::from_id(chain_id).ok_or(...)?;
    match chain {
        Chain::Polkadot => { /* ... */ }
        Chain::Bittensor => { /* ... */ }
        // ...
    }
}
```

Swift gets `func signAndSend(chainId: UInt32, params: ...) async throws -> String` and decodes the JSON itself.

**Use this only for**: methods that genuinely dispatch on `Chain` and where the per-chain return shapes are heterogeneous enough that one UniFFI Record won't cover them. Send dispatch, fee preview, transaction broadcast.

**Inside the dispatched arm**, the input JSON should be parsed into a typed struct as the first line — see `service::types::PolkadotSendParams`, `BittensorSendParams`. Don't pull fields out of the `serde_json::Value` ad-hoc; that hides the contract.

### Why two patterns

Pattern A is strictly better when it works. Pattern B exists because:

1. UniFFI Records can't be heterogeneous-by-discriminant in a way that's ergonomic for ~20 different chain return shapes. Modeling every chain's send result as a separate UniFFI Record and adding a UniFFI Enum to wrap them is possible but adds binding surface for marginal gain.
2. The Swift call sites for chain-dispatched flows already had a "decode this JSON" shape from before UniFFI was adopted. Migrating them is mechanical but touches every chain.

A long-term goal is to migrate Pattern B to Pattern A on a per-method basis. The `service::types` typed-params structs are the first step — input is now typed even if the return is still a String.

---

## What can cross

UniFFI 0.29 supports these item kinds. Each has different rules.

### `#[derive(uniffi::Record)]`

A struct with named fields. All fields must themselves be UniFFI-compatible types: primitives, `String`, `Vec<T>`, `Option<T>`, other Records, Enums, or Objects.

```rust
#[derive(Debug, Clone, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ChainEndpoints {
    pub chain_id: u32,
    pub endpoints: Vec<String>,
    pub api_key: Option<String>,
}
```

**Rules:**
- Fields are public on both sides; no method generation.
- Add `#[serde(rename_all = "camelCase")]` so the JSON shape matches Swift's natural casing. The UniFFI binding doesn't care; serde does, for any path that goes through JSON (notably the dispatched-path return strings).
- If a field's name doesn't follow snake-case, override with `#[serde(rename = "...")]` per field. See `PersistedAppSettings` for examples (`etherscan_api_key` ↔ `etherscanAPIKey`).
- Every field is part of the public API. Renaming or reordering is a breaking change for Swift consumers.

### `#[derive(uniffi::Enum)]`

A C-style enum (no associated data) or an enum with named-field variants. Tagged unions cross fine; tuple variants don't (UniFFI requires named fields).

```rust
#[derive(uniffi::Enum)]
pub enum CoreTransactionKind {
    Send,
    Receive,
}
```

**Rules:**
- Variants are visible to Swift as enum cases.
- Adding a variant is a breaking change for any Swift code that exhaustively matches.
- If an enum is `serde::Deserialize`-d from a wire shape that uses different variant names than Rust's, use `#[serde(rename_all = "camelCase")]` or `#[serde(rename = "...")]` per variant.

### `#[derive(uniffi::Object)]`

An opaque reference type with methods. Used for stateful actors like `WalletService` and `BalanceRefreshEngine`. Swift gets a class with the methods exposed by `#[uniffi::export]` impl blocks.

```rust
#[derive(uniffi::Object)]
pub struct WalletService {
    // private fields, never seen by Swift
}

#[uniffi::export]
impl WalletService {
    pub async fn sign_and_send(&self, /* ... */) -> Result<String, SpectraBridgeError> { /* ... */ }
}
```

**Rules:**
- Fields are private to Rust; Swift sees only methods.
- Every method in an `#[uniffi::export]` impl block is exposed regardless of `pub(crate)` visibility. **This is a footgun**: don't put internal helpers inside the same impl block as your FFI surface. Helpers go in a sibling `impl WalletService { ... }` block (no attribute) or in a separate module.
- Constructors must be tagged: `#[uniffi::constructor]` on a method named `new` (or any name with that attribute) makes it Swift-callable.
- Async methods work via UniFFI's tokio integration. They become `async throws` on the Swift side.

### `#[derive(uniffi::Error)]`

A typed error enum that Swift sees as a real Swift `Error`-conforming type. Used by `SpectraBridgeError`.

```rust
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SpectraBridgeError {
    #[error("{message}")]
    Network { message: String },
    #[error("{message}")]
    Decode { message: String },
    #[error("{message}")]
    InvalidInput { message: String },
    #[error("{message}")]
    Failure { message: String },
}
```

**Rules:**
- Variants must use named fields, not tuple variants. UniFFI rejects `MyError(String)`.
- The `thiserror::Error` derivation provides `Display`; UniFFI uses it for the Swift error description.
- Sub-module errors should convert via `From` impls on `SpectraBridgeError` (see `core/src/lib.rs`). Conversions that lose structure (everything → `Failure`) lose information; prefer routing to the right variant when the source error has discriminating info (e.g. `reqwest::Error::is_decode()` → `Decode` vs `Network`).

### Free functions with `#[uniffi::export]`

Any free function tagged `#[uniffi::export]` becomes a Swift function. Arguments and return types must be UniFFI-compatible.

```rust
#[uniffi::export]
pub fn list_all_builtin_tokens() -> Vec<tokens::TokenEntry> {
    tokens::list_tokens(u32::MAX)
}
```

This is the lightest-weight FFI surface and the right default for stateless utilities.

---

## What can't cross

UniFFI 0.29 doesn't support:

- **Generics on FFI items.** `fn validate<T>(...)` doesn't cross. Monomorphize at the boundary.
- **Trait objects.** `Box<dyn Foo>` doesn't cross. Convert to a typed enum or wrap in a UniFFI Object.
- **Lifetimes.** All FFI types must be `'static`. Don't borrow on the boundary.
- **`Result<T, E>` where `E` isn't a UniFFI Error.** Use `SpectraBridgeError` or define a dedicated UniFFI Error.
- **Tuples or fixed-size arrays as fields.** `[u8; 32]` and `(u32, u32)` aren't crossings. Wrap in a Record (`pub struct Sha256Hash { pub bytes: Vec<u8> }`) or expose as `Vec<u8>` and validate length on read.
- **Raw pointers.** Obviously.
- **Async functions in non-`Object` impls.** Async only works on methods of a `#[derive(uniffi::Object)]` type. Free async functions don't cross. Put them on a service object if you need async.
- **`HashMap<K, V>`** where K isn't a string-like primitive. Stringly-keyed dictionaries cross fine; integer-keyed don't.

---

## How bindings get regenerated

Workflow:

1. Edit Rust code in `core/`.
2. Run `cargo build --release --manifest-path core/Cargo.toml` to confirm it compiles.
3. Run `bash scripts/bindgen-ios.sh`. This:
   - Builds `libspectra_core.dylib` (debug profile).
   - Runs `spectra-uniffi-bindgen generate --language swift` against the dylib.
   - Writes generated Swift to `swift/generated/`.
4. Build the Swift app with `xcodebuild` (or open in Xcode).

For the iOS build artifacts (the static framework that ships with the app), use `bash scripts/build-ios.sh`. That builds the dylib for every iOS architecture and writes them to `build/apple/`. Run it before shipping; not needed for day-to-day development.

For Android: `bash scripts/bindgen-android.sh` and `bash scripts/build-android.sh` are the equivalents. Kotlin bindings land in `kotlin/`.

### What "regenerated" means in practice

Anything in `swift/generated/` is overwritten on every bindgen run. Don't edit those files. If you need a Swift-side wrapper around a generated type, add it in `swift/shell/`, `swift/rustbridge/`, or wherever the consuming code lives — never in `swift/generated/`.

The pattern is:

- Rust defines `CoreCoin` as a UniFFI Record.
- `swift/generated/` produces a Swift `CoreCoin` struct.
- `swift/shell/CoreModels.swift` does `typealias Coin = CoreCoin` and adds Swift-side extensions (`Identifiable` conformance, helper initializers, color resolution).

That keeps the generated layer mechanical and the Swift-side ergonomics in code you control.

---

## Boundary-breaking changes

These break Swift consumers and require a regenerate + rebuild:

- Renaming a Record field, Enum variant, or Object method.
- Adding a required (non-`Option<_>`) field to a Record that's persisted (UserDefaults, SQLite). Old persisted data won't decode. **Add `Option<T>` and use `#[serde(default)]` for new fields on persisted records.**
- Changing a field's type (`u32` → `u64`, `String` → `Option<String>`, etc.).
- Removing a public function or method.
- Changing an Error variant's payload.

These don't break Swift consumers (Swift code keeps compiling and the binding regenerate is a no-op for unchanged items):

- Adding a new function or method.
- Adding a new Record (Swift gains a new struct definition).
- Adding a new variant to an Enum **as long as no Swift code exhaustively matches the enum**.
- Renaming an internal (non-exported) function.

When in doubt, regenerate bindings and recompile Swift. The compile error will tell you whether the change reached the boundary.

---

## JSON-on-the-wire conventions

The dispatched path (Pattern B) ships JSON between Rust and Swift. Both sides need to agree on the shape:

- **camelCase** field names. Rust uses `#[serde(rename_all = "camelCase")]`; Swift's default `JSONEncoder` already produces camelCase.
- **Numeric types**: prefer `i64` / `u32` / `f64` for fields that round-trip through JSON. `u128` doesn't survive — JSON numbers can't represent it. For u128, use `String` and parse with a typed serde helper (see `deserialize_u128_from_string_or_number` in `service::types`).
- **Optional fields**: Rust `Option<T>` with `#[serde(skip_serializing_if = "Option::is_none")]` produces "omit when None"; Swift's `decodeIfPresent` handles the omission. Don't emit `null` — Swift treats `null` and "absent" the same on read but persistence layers (SQLite) don't.
- **Dates/times**: unix epoch seconds (`f64`) for wire shapes that flow through merge/refresh; Swift reference time (`f64`, seconds since 2001-01-01 UTC) for shapes that flow through SQLite persistence. The two formats look identical but use different epochs — see the `CoreTransactionRecord` vs `CorePersistedTransactionRecord` doc comments for why.
- **Hex bytes**: lowercase, no `0x` prefix unless the chain's wire format requires one (EVM does; Bitcoin doesn't).

---

## When to keep state on Rust vs Swift

A frequent question on new endpoints: should the endpoint be a `WalletService` method (Pattern B / Object) or a free function (Pattern A)?

- **Free function** if the endpoint is stateless: input → output, no observation of prior calls, no shared cache. Examples: validation, decoding, BIP-39 utilities, transaction merge.
- **`WalletService` method** if the endpoint touches shared connection state: HTTP client, endpoint lists, secret store, the refresh engine. Examples: send, balance fetch, history fetch.
- **Pure Swift** if the endpoint is purely derived from observable state already on `AppState` and doesn't need to consult Rust at all. Examples: formatting, fiat conversion display.

The wallet has been steadily moving logic from Swift into Rust — see `ARCHITECTURE.md` for the rationale. New work should default to Rust unless the logic is genuinely UI-shaped (e.g. a SwiftUI binding helper).

---

## Common boundary errors

**"Cannot find type `CoreFoo` in scope"** in Swift after an Rust change.
The bindings haven't been regenerated. Run `bash scripts/bindgen-ios.sh`.

**"Type `Foo` does not conform to protocol `Codable`"** in Swift.
The Rust struct probably gained a non-`Codable` field, or the generated binding doesn't include `Codable`. Add a Swift extension that conforms it (`extension CoreFoo: Codable {}`) — UniFFI doesn't generate Codable conformance automatically.

**Swift sees an empty struct** where Rust has fields.
Almost always: the Rust struct is missing `#[derive(uniffi::Record)]` and is being generated as an opaque wrapper. Add the derive.

**"unknown chain_id: 999"** at runtime in a method that takes a `Chain`.
Swift sent a chain id Rust doesn't know about. Either Swift is using a stale `chainID` constant (check `WalletServiceBridge.swift`'s `nonisolated static let` chain ids), or a freshly added chain in Rust hasn't had its Swift constant added.

**A `Result<String, _>` from a Pattern B method fails to decode** in Swift.
Rust serialized one shape; Swift expected another. Print the raw string at the boundary (`print(result)`), confirm the keys are camelCase, and confirm `Optional` fields aren't emitted as `null` when None.

---

## Further reading

- `core/src/lib.rs` — `SpectraBridgeError` definition and conversion impls.
- `core/src/ffi.rs` — the typed-path FFI surface, with a header doc explaining the dichotomy.
- `core/src/service/mod.rs` — the dispatched-path methods.
- `core/src/service/types.rs` — typed parameter structs for chain dispatch arms.
- `core/src/service/standalone.rs` — synchronous, stateless UniFFI exports (token catalog, BIP-39).
- `swift/rustbridge/WalletServiceBridge.swift` — the Swift singleton that holds the `WalletService` instance and exposes call sites for Swift code.
- `scripts/bindgen-ios.sh` / `scripts/build-ios.sh` — the binding regeneration and iOS artifact builds.
