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

Spectra does not aim to become a trading terminal or a swap-first product;
custody and payments take priority over speculative features and purchase
upsells.

## Development

This repository is the Rust wallet core and the `spectra` CLI. The apps live
in their own repositories and link core as a package built here:
[spectra-apple](https://github.com/lacklusteradmin/spectra-apple), the SwiftUI
app for iOS and macOS, and `spectra-android`, not started. Core owns domain
state and decisions; the front ends render its results and forward user intent.

- [AGENTS.md](AGENTS.md): the rules for working in this repository, Rule 0
  first. Read first.
- [Open items](docs/OPEN-ITEMS.md): remaining work and known limitations.
- [Architecture](docs/ARCHITECTURE.md): design decisions and ownership boundaries.
- [Chain support](docs/CHAIN-SUPPORT.md): what each network supports and refuses.
- [FFI boundary](docs/FFI-BOUNDARY.md): what core exports to the apps through
  UniFFI 0.31.2.
- [Tests](tests/README.md) and [scripts](scripts/README.md): the suites and tools
  behind the Makefile.

It builds with the current stable Rust, with rustfmt and clippy for
`make lint`; `rustup update` keeps it current.

### The CLI

The CLI is the front end with no platform under it. It needs nothing but a Rust
toolchain:

```sh
cargo run -p spectra_cli
```

With no command, at a terminal, that opens tabs and windows in the manner of
kitty, each window a prompt that runs the commands `--help` lists; `Tab`
completes them and `Ctrl+B` then `?` lists the keys. The same commands work
directly for scripts, with `--json` for a report to parse:

```sh
cargo run -p spectra_cli -- --help
```

### Verification

Four targets gate a change, each run by name; [AGENTS.md](AGENTS.md#verification)
says which a change needs:

| Target | What it runs |
|---|---|
| `make lint` | `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, the source-scan tests and the unused-code source scan |
| `make test` | `cargo test` |
| `make test-cli` | `python3 -B tests/acceptance` — the real binary end to end, offline, throwaway data directory |
| `make check-icons` | the icon-normalization check; needs `svgo` |

The checked-in reference data for the SDK generators in `tests/vectors/` stays
in `core/tests/fixtures/`; normal tests do not regenerate it.

## License

Spectra is free software under the [GNU General Public License v3.0](LICENSE).
It comes with no warranty. A wallet you cannot inspect is a wallet you are
trusting on someone else's word, so the terms that keep modified versions open
are part of the point rather than an afterthought.
