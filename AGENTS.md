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

## Docs

Read the one that covers what you are changing:

| Doc | Covers |
|---|---|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Ownership and boundary rules, module layout |
| [CHAIN-SUPPORT.md](docs/CHAIN-SUPPORT.md) | What each network supports and refuses |
| [FFI-BOUNDARY.md](docs/FFI-BOUNDARY.md) | UniFFI and Swift binding mechanics |
| [IOS-UI.md](docs/IOS-UI.md) | Liquid Glass, typography, color, layout and corner radii |
| [OPEN-ITEMS.md](docs/OPEN-ITEMS.md) | Remaining work |
| [tests/README.md](tests/README.md) | Where tests live, acceptance suites, SDK vector generators |
| [scripts/README.md](scripts/README.md) | Build, source-scan and icon tools |

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

- Core owns domain state and decisions; Swift and the CLI render and forward.
  New domain logic belongs in `core/`, and its rules are tested there with
  `cargo test`, without a platform.
- Per-chain facts belong on `registry::Chain`, not in caller-owned lists.
- Every request to a chain service, broadcasts included, lives in
  `core/src/api/`, one `<api>.rs` per `EndpointApi`, named after its `as_str()`
  with underscores for hyphens. `send`, `staking` and `service` call `api`,
  never the reverse. `send/` builds and signs per chain and never implements a
  client.
- Core must own the state it decides about: do not add an export that takes
  data Swift read from core only to return a decision about it.
- Swift may hold view state, not authoritative domain state. If losing data on
  restart would be a bug, core owns it.
- Prefer deleting unnecessary Swift code over moving it into core.

## Funds and keys

- A registry capability is a promise. Add one only when core owns the complete
  operation and the CLI can prove it; until then the operation is refused.
- A protocol, derivation, signing or key change needs reference data from an
  independent SDK (`tests/vectors/`) rather than core's own output, CLI
  acceptance, its refusal paths and `make verify`.
- No test broadcasts to a real chain, and a live endpoint probe stays
  read-only.
- A secret operation goes through `SecretStore`, so the CLI can exercise it.

## Verification

Match verification to the scope and risk of the change, and report which checks
actually ran.

| Change | Run |
|---|---|
| Docs, copy, resources | Only what the edit can affect |
| Swift views, design tokens, icons | `make check-ui`, plus the target covering any logic touched |
| A localized code change | The target that covers it |
| Core behaviour or persistence, with no funds, keys or FFI change | `make lint test`, plus `make test-cli` when a CLI flow is affected |
| Broad refactors, funds or keys, FFI and cross-platform changes | `make verify`, unless I say otherwise |

`make verify` is `make lint test test-cli check-ui test-ios`: formatting, clippy
at `-D warnings` and the source scans, then `cargo test`,
`python3 -B tests/acceptance`, the design-token and icon checks, and
`xcodebuild test` on an iPhone simulator. The Makefile holds the exact commands.
Run a single target by name when iterating, and `cargo test <filter>` for one
rule; `-p` selects other features and rebuilds dependencies;
`make fmt` applies the formatting. When another task is building the same Xcode
project, give yours its own directory with
`make test-ios IOS_TEST_DERIVED_DATA=<dir>`.

CI runs all of it but `test-ios`, which needs Xcode and a simulator, so that one
guards a change only when you run it.

No test is expected to fail. Treat a failure as real even when it looks
unrelated to the change, and report it; do not write it off as flaky or
pre-existing without evidence.

CLI acceptance drives the real binary in a throwaway directory without network.
A rule is tested in core; add an acceptance check only for what the binary alone
shows: state across processes, files on disk, exit codes, confirmation gates,
and send or staking flows against loopback nodes.

## Platform constraints

- Check APIs, syntax and generated bindings against **UniFFI 0.31.2 and Swift 6**
  before changing FFI or Swift code. See [FFI-BOUNDARY.md](docs/FFI-BOUNDARY.md).
- Never hand-edit `swift/generated/`. Change the Rust API and rebuild.
- Core's `reqwest` must use `rustls-tls-webpki-roots`. Native roots are empty on
  iOS and cause HTTPS `UnknownIssuer` failures.
- [docs/IOS-UI.md](docs/IOS-UI.md) is the authority for Liquid Glass,
  typography, color, layout and corner radii.
- The app targets **iOS 26** (the deployment target in the Xcode project) in
  the **Swift 6** language mode. Write for that floor with the current idiom:
  no `#available` checks or fallbacks for older releases, `@Observable` rather
  than `ObservableObject`/`@Published`, `NavigationStack` rather than
  `NavigationView`, and async/await rather than completion handlers or GCD
  unless an API requires a queue. If an iOS 26 API is unfamiliar, check
  Apple's current documentation rather than falling back to an older pattern.
- Resolve Swift 6 concurrency diagnostics with correct isolation. Do not add
  `@unchecked Sendable`, `nonisolated(unsafe)` or `@preconcurrency` to silence
  one unless the preceding line explains why it is sound.

## Swift conventions

- `Task` closures in `AppState` and its extensions capture `[weak self]` unless
  the preceding line explains why the task must keep the state alive.
- Pure transformations belong in core or free functions; `AppState` methods
  are thin adapters. Constructing `AppState` pulls in SQLite, Keychain and Rust.
- `swift/app/` holds every app source, grouped by feature: `App/` (the
  `AppState` core, lifecycle and platform services), `Models/` (extensions of
  core records and the catalogs built from them), `Components/` (shared views
  and design tokens), then one folder per feature — `Dashboard/`, `Wallets/`
  (with `Tools/`), `Send/`, `Receive/`, `History/`, `Staking/`, `Settings/`,
  `Diagnostics/`, `Wiki/` — with a feature's state beside its views. The
  folder is an Xcode synchronized group: adding or moving a file under it needs
  no project edit, and file names stay unique across the module. The Live
  Activity extension is in `swift/extensions/`, the tests in `swift/tests/`.
- Name an extension file `<Type>+<Topic>.swift` (`AppState+SendFlow.swift`,
  `SendFlowState+Preview.swift`), a file of free functions or of extensions
  across several types after its topic, and any other file after its type.
  Split growing topics into siblings rather than moving code into unrelated
  files.
- Do not build an observation loop from `withObservationTracking` and
  `withCheckedContinuation`: cancellation does not resume the continuation and
  can retain `self`. Debounce a reaction with a stored, cancellable `Task`
  around `Task.sleep`, as `SendFlowState.scheduleRecipientResolution` does.
- Tests use Swift Testing (`@Test`, `#expect`, `#require`), not XCTest. Tests
  run in parallel, and `.serialized` orders only a suite's own tests, not that
  suite against the others. A test touching process-wide state (the Keychain,
  the network runtime, windows) must stay correct alongside every other suite;
  tests that contend for it belong in one `.serialized` suite. A test that needs
  an `AppState` uses `@Suite(.isolatedAppState)` and conforms to
  `IsolatedAppStateSuite`.

## Resources

`resources/` is the app target's synchronized group, and Xcode copies such a
group in **flat**: every file under it lands at the bundle's resource root,
whatever directory it sat in. `resources/strings/RuntimeStrings.en.json` ships
as `RuntimeStrings.en.json`. Two rules follow:

- **Nothing but a runtime resource belongs here.** Anything under `resources/`
  ships, read or not. Build-time inputs go elsewhere — icon sources in
  `icons/`, the translator glossary in `docs/LocalizationGlossary.json`.
- **A file name must be unique across the whole tree.** A directory such as
  `strings/` groups files for people and scripts; it does not separate them in
  the bundle. Put the locale in the name (`RuntimeStrings.zh-Hans.json`):
  per-locale subdirectories with one shared name would silently overwrite each
  other.

## Icons

Edit SVG sources in `icons/`, never `swift/Assets.xcassets/` by hand. Keep
sources out of `resources/`; iOS renders the generated asset catalog.

After adding or editing library icons, normalize and export. This needs `svgo`,
and ImageMagick for app icons (`brew install svgo imagemagick`):

```sh
scripts/normalize-icons.sh && scripts/export-swift-icons.sh
```

- `crypto/` is a 64×64 SVG library and exports to the matching `crypto` asset
  group, without a namespace. Keep source and asset group names aligned.
- Use a full-disc `<circle cx="32" cy="32" r="32" fill="…"/>` as the first
  drawable (after any `<defs>`). `usd1` is the exception: its rings form the disc.
- Formatting is owned by `scripts/svgo.config.mjs`; use
  `scripts/normalize-icons.sh --check` to detect drift. Non-square artwork needs
  manual layout; `userSpaceOnUse` gradients may retain transforms.
- `appicon/` is excluded from normalization. Its 1024-point sources export as
  1024×1024 PNGs to `AppIcon.appiconset`; keep that destination name.
