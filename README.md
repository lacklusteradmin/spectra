# Spectra

[![CI](https://github.com/Sheny6n/SpectraWallet/actions/workflows/ci.yml/badge.svg)](https://github.com/Sheny6n/SpectraWallet/actions/workflows/ci.yml)

Spectra is a prelaunch, multi-chain self-custodial crypto wallet focused on
custody, payments, recovery and staking. It aims to be approachable for everyday
use while exposing the network settings, diagnostics and logs advanced users need.

## Product direction

- Local-first, privacy-conscious software that keeps users in control of their
  keys and wallet data.
- Consistent send, receive, history and backup flows across chains.
- Broad import and recovery support, including legacy wallet conventions and
  watch-only wallets.
- Configurable endpoints and providers, exportable diagnostics, and clear
  transaction verification states.
- Biometric protection, optional wallet passwords, customizable dashboards and
  staking where chain support makes sense.

Spectra does not aim to become a trading terminal or a swap-first product;
custody and payments take priority over speculative features and purchase
upsells.

## Development

One Rust wallet core serves several front ends: the `spectra` CLI, a native
SwiftUI iOS app, and an Android skeleton. The iOS app predates the core; work is
underway to finish moving domain state and decisions into Rust.

- [AGENTS.md](AGENTS.md): the rules for working in this repository. Read first.
- [PLAN.md](PLAN.md): Rule 0, current stages and remaining work.
- [Behaviour changes](docs/BEHAVIOUR-CHANGES.md): what changed on purpose, and why.
- [Architecture](docs/ARCHITECTURE.md): design decisions and ownership boundaries.
- [FFI boundary](docs/FFI-BOUNDARY.md): UniFFI 0.31 and Swift 6 integration.
- [iOS UI](docs/iosUI.md): layout, typography and Liquid Glass rules.

`rust-toolchain.toml` pins the toolchain, so `cargo` installs the right one on
first use and no version needs naming here.

### The CLI

The CLI is the front end with no platform under it, and every domain rule has to
be drivable from it. It needs nothing but a Rust toolchain:

```sh
cargo run -p spectra-cli -- --help
```

### The iOS app

The Swift bindings are generated from the Rust core and are **not** checked in,
so a fresh clone has to build them before the Xcode project will compile:

```sh
make ios-artifacts
```

That compiles the `ffi` crate for the iOS targets, lipo-merges the simulator
static libraries into `build/apple/`, and regenerates `swift/generated/`. Then
open `swift/Spectra.xcodeproj`. Never hand-edit `swift/generated/` — change the
Rust API and regenerate.

### Verification

Three suites gate a change, and `make verify` runs all of them:

```sh
make verify
```

| Target | What it runs |
|---|---|
| `make lint` | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` |
| `make test` | `cargo test --workspace` |
| `make test-cli` | `scripts/cli-acceptance.sh` — offline, throwaway data directory |
| `make test-ios` | `xcodebuild test` on an iPhone simulator |
| `make check-ui` | design-token and icon-normalization checks |

CI runs everything except `test-ios`, which needs Xcode and a simulator.

## License

Spectra is free software under the [GNU General Public License v3.0](LICENSE).
It comes with no warranty. A wallet you cannot inspect is a wallet you are
trusting on someone else's word, so the terms that keep modified versions open
are part of the point rather than an afterthought.
