## Git

Do not git commit or push unless I said so.

## Rule 0

**Change behaviour, or remove functionality, when doing so makes the system
simpler or more correct.** Preserving existing behaviour is not the goal. When
the code as it stands and the code as it should be disagree, write the second
and delete the first.

Rule 0 outranks any reason to keep something as it is: precedent, callers, an
existing test. It does not suspend the other rules in this file.

- **Fix an inconsistency instead of preserving it.** If twenty chains do one
  thing and three do another, pick the right one for all of them. Do not write
  a test that pins the split in place.
- **Delete a feature that is not worth its complexity**, and say so.
- **Collapse two models of one thing**, even when both have callers.
- **Change a stored shape, an id format or a schema outright.** Spectra is
  prelaunch with no existing users: change storage formats, schemas, keychain
  keys and serialized structures directly, without migration or
  backward-compatibility shims.

What it does not license:

- **Guessing about funds, keys or addresses.** Refuse early, validate before
  storing and derive rather than trust caller input.
- **Dropping scope quietly.** Removing a feature is a decision stated in the
  change, not an omission to notice later.
- **Widening a change unasked.** Fix what the task touches. Report an oddity
  elsewhere instead of folding it in.

If you find yourself writing "preserved exactly" or a test that asserts
today's oddity, stop and fix the oddity instead.

## Repositories

Spectra is three repositories. This one holds the Rust core, the CLI and the
crypto icon library. The apps live beside it, and each takes this repository
as its `spectra` git submodule:

| Repository | Holds |
|---|---|
| `spectra` (this one) | `core/`, `cli/`, `tools/`, `tests/`, `icons/crypto/` |
| `spectra-apple` | The iOS and macOS app |
| `spectra-android` | The Android app, not started |

An app sees a change here only once it moves its submodule to that change.
Check an FFI change against the app before it lands: put it in spectra-apple's
`spectra/` checkout and run that repository's checks.

## Docs

Read the one that covers what you are changing:

| Doc | Covers |
|---|---|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Ownership and boundary rules, module layout |
| [CHAIN-SUPPORT.md](docs/CHAIN-SUPPORT.md) | What each network supports and refuses |
| [FFI-BOUNDARY.md](docs/FFI-BOUNDARY.md) | What core exports to the apps, and how |
| [OPEN-ITEMS.md](docs/OPEN-ITEMS.md) | Remaining work |
| [tests/README.md](tests/README.md) | Where tests live, acceptance suites, SDK vector generators |
| [scripts/README.md](scripts/README.md) | Source-scan, icon and data tools |

Keep them true:

- A change that alters what a doc says updates that doc in the same change: a
  capability or refusal in `CHAIN-SUPPORT.md`, a suite in `tests/README.md`, a
  script in `scripts/README.md`.
- A doc states what is true now. History belongs in git, and a file or symbol
  is named only while it exists.
- `OPEN-ITEMS.md` holds remaining work only. Delete an item when it is done.
- `docs/audits/` is dated evidence. Add a new record rather than editing an old
  one.

## Architecture

- Core owns domain state and decisions; the front ends — the CLI here and the
  apps — render and forward. New domain logic belongs in `core/`, and its
  rules are tested there with `cargo test`, without a platform.
- Per-chain facts belong on `registry::Chain`, not in caller-owned lists.
- Every request to a chain service, broadcasts included, lives in
  `core/src/api/`, one `<api>.rs` per `EndpointApi`, named after its `as_str()`
  with underscores for hyphens. `send`, `staking` and `service` call `api`,
  never the reverse. `send/` builds and signs per chain and never implements a
  client.
- Core must own the state it decides about: do not add an export that takes
  data an app read from core only to return a decision about it.
- An app may hold view state, not authoritative domain state. If losing data
  on restart would be a bug, core owns it.
- Prefer deleting unnecessary app code over moving it into core.

## Funds and keys

- A registry capability is a promise. Add one only when core owns the complete
  operation and the CLI can prove it; until then the operation is refused.
- A protocol, derivation, signing or key change needs reference data from an
  independent SDK (`tests/vectors/`) rather than core's own output, CLI
  acceptance, its refusal paths, every verification target here and
  spectra-apple's checks against the change.
- No test broadcasts to a real chain, and a live endpoint probe stays
  read-only.
- A secret operation goes through `SecretStore`, so the CLI can exercise it.

## Verification

Match verification to the scope and risk of the change, and report which checks
actually ran.

| Change | Run |
|---|---|
| Docs | Only what the edit can affect |
| Crypto icons | `make check-icons` |
| A localized code change | The target that covers it |
| Core behaviour or persistence, with no funds, keys or FFI change | `make lint test`, plus `make test-cli` when a CLI flow is affected |
| Broad refactors, funds or keys, and FFI changes | All four targets, and spectra-apple's checks with the change in its `spectra/` checkout, unless I say otherwise |

The four targets are `make lint`, `test`, `test-cli` and `check-icons`:
formatting, clippy at `-D warnings` and the source scans, then `cargo test`,
`python3 -B tests/acceptance` and the icon-normalization check, which needs
`svgo`. No target runs them all: each is run by name. The Makefile holds the
exact commands. Run `cargo test <filter>` for one rule; `-p` selects other
features and rebuilds dependencies; `make fmt` applies the formatting. CI runs
all four.

No test is expected to fail. Treat a failure as real even when it looks
unrelated to the change, and report it; do not write it off as flaky or
pre-existing without evidence.

CLI acceptance drives the real binary in a throwaway directory without network.
A rule is tested in core; add an acceptance check only for what the binary alone
shows: state across processes, files on disk, exit codes, confirmation gates,
and send or staking flows against loopback nodes.

## Platform constraints

- Check APIs, syntax and generated bindings against **UniFFI 0.31.2** before
  changing an export. See [FFI-BOUNDARY.md](docs/FFI-BOUNDARY.md).
- An app never hand-edits its generated bindings. Change the Rust API here and
  rebuild the app.
- Core's `reqwest` must use `rustls-tls-webpki-roots`. Native roots are empty on
  iOS and cause HTTPS `UnknownIssuer` failures.

## Icons

`icons/crypto/` is the 64×64 SVG library the apps draw tokens and chains with.
Every artwork name core's catalog gives a token or chain has a file here, which
`cargo test every_named_mark_ships_a_file` checks. Each app exports the library
into its own assets: spectra-apple to the `crypto` asset group, without a
namespace, so keep file names and artwork names aligned.

After adding or editing an icon, normalize it. This needs `svgo`
(`brew install svgo`):

```sh
scripts/normalize-icons.sh
```

- Use a full-disc `<circle cx="32" cy="32" r="32" fill="…"/>` as the first
  drawable (after any `<defs>`). `usd1` is the exception: its rings form the disc.
- Formatting is owned by `scripts/svgo.config.mjs`; use
  `scripts/normalize-icons.sh --check` to detect drift. Non-square artwork needs
  manual layout; `userSpaceOnUse` gradients may retain transforms.
