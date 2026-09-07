# Architecture decisions

Spectra is one application with a shared Rust core and native front ends.
[PLAN.md](../PLAN.md) tracks implementation status; [FFI-BOUNDARY.md](FFI-BOUNDARY.md)
covers binding mechanics.

## Workspace

| Directory | Responsibility |
|---|---|
| `core/` | Domain logic, persistence, network, crypto and UniFFI exports |
| `ffi/` | Binding crate; re-exports core |
| `cli/` | Argument parsing, terminal output and password prompting |
| `swift/` | Native iOS UI and platform services |
| `kotlin/` | Android shell, currently a skeleton |
| `tools/uniffi-bindgen/` | Binding generation binary |

Core stays one crate. A crate per domain would add Cargo overhead without an
external consumer that needs the isolation. There are no published library APIs
or semver commitments between these crates. Core builds and tests without Xcode
or an Android toolchain.

## Ownership

Rust owns wallets, settings, address rules, transaction history, persistence,
network policy and signing. Front ends send intents and render the results.
The CLI is the check that none of those operations depends on a phone.

Swift owns navigation, sheets, text being edited, focus, animation and rendering
caches. Keychain, biometrics, notifications, Live Activities and device signals
stay on the platform and supply inputs to core-owned policy. Losing view state
on restart may cost a redraw; losing domain state must not lose user data.

`CoreAppState` holds small resident collections. Unbounded history is a separate
queryable store so changing a setting does not clone every transaction.
`WalletService::open_state` binds the database; state commands persist before
returning and no-op commands emit no events or writes. UI projections have one
writer. Derived domain data is computed from core's store and adopted by the UI,
not computed by sending the projection back to core.

Core drives secret reads, writes and password encryption through `SecretStore`.
The platform implements storage; native file and in-memory backends let the CLI
and tests use the same domain operations.

## Chain rules

`registry::Chain` is the authority for chain identity and capabilities, backed
by `core/data/chains.toml`. Adding a chain requires a catalog row and an enum
variant in matching order. Registry tests check that relationship independently.
Address formats, derivation paths, address slots, EVM membership and routing
facts belong there rather than in caller-owned lists.

Chain-specific implementations live under each domain's `chains/` directory.
Keep differences that carry protocol meaning; share cryptographic primitives
and wrappers that differ only in a chain name. Tests over the registry should
assert complete capability coverage, rather than only test named examples.

## Boundary design

Exports live next to their implementations. Stateful operations are service
methods; genuinely stateless calculations may be free functions. Do not merge
distinct preview inputs into a wide command/response union just to lower the
export count: Bitcoin's xpub and gap bounds are different inputs from an EVM
nonce and fee configuration.

The native UIs share domain code, not a cross-platform UI framework. Core must
not exit the process or install a global logger; the executable owns logging
configuration and keeps stdout available for CLI JSON.
