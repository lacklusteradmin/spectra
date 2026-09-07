# Spectra

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

Spectra is intended to be open source. It does not aim to become a trading
terminal or a swap-first product; custody and payments take priority over
speculative features and purchase upsells.

## Development

One Rust wallet core serves several front ends: the `spectra` CLI, a native
SwiftUI iOS app, and an Android skeleton. The iOS app predates the core; work is
underway to finish moving domain state and decisions into Rust.

- [PLAN.md](PLAN.md): current stages, remaining work and intentional behaviour changes.
- [Architecture](docs/ARCHITECTURE.md): design decisions and ownership boundaries.
- [FFI boundary](docs/FFI-BOUNDARY.md): UniFFI 0.31 and Swift 6 integration.
- [iOS UI](docs/iosUI.md): layout, typography and Liquid Glass rules.

To inspect the command-line interface:

```sh
cargo run -p spectra-cli -- --help
```
