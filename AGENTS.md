# Working on Spectra

## Git

Do not run any Git state-modifying command (including commit, add/stage, reset
or checkout) unless explicitly requested. Finish edits and stop; the user will
say when to commit.

## Rule 0

Rule 0 outranks every other rule here: **change behaviour, or remove
functionality, when doing so makes the system simpler or more correct.**
Preserving existing behaviour is not the goal. When the code as it stands and
the code as it should be disagree, write the second and delete the first.

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

- **Silence.** Record each behaviour change in
  [docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md) with before/after,
  rationale, a CLI check and the verification run.
- **Guessing at the safe side.** For funds, keys and addresses, refuse early,
  validate before storing and derive rather than trust caller input.
- **Dropping scope quietly.** Removing a feature is a decision stated in the
  change, not an omission to notice later.

If you find yourself writing "preserved exactly" or a test that asserts
today's oddity, stop and fix the oddity instead.

## Architecture

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) explains ownership and the
boundary rules; [docs/OPEN-ITEMS.md](docs/OPEN-ITEMS.md) holds remaining work.

- Core owns domain state and decisions; the CLI proves they need no platform;
  Swift renders and forwards. New domain logic belongs in `core/` and must be
  drivable from `spectra`.
- Per-chain facts belong on `registry::Chain`, not in caller-owned lists.
- Every request to a chain service, broadcasts included, lives in
  `core/src/api/<api>.rs`, one file per `EndpointApi` named after its
  `as_str()`. `api` depends only on its transport and `registry`; `fetch`,
  `send`, `staking` and `service` call it. `send/` builds and signs per chain
  and never implements a client.
- Do not add `core_plan_*` functions; core must own the state it decides about.
- Swift may hold view state, not authoritative domain state. If losing data on
  restart would be a bug, core owns it.
- Prefer deleting unnecessary Swift code over moving it into core.

## Verification

Match verification to the scope and risk of the change. Small, localized edits
(such as copy, styling, resources or documentation) need only relevant targeted
checks; do not run the full suite by default. Run the full suite for major
changes, such as broad refactors, core domain behaviour, persistence, funds/key
handling or cross-platform/FFI integration changes, unless the user says otherwise:

```sh
make verify
```

That is `make lint test test-cli test-ios` — `cargo fmt --check` and
`cargo clippy -- -D warnings`, then `cargo test --workspace`,
`scripts/cli-acceptance.sh`, and `xcodebuild test` on an iPhone simulator. Run
a single one by name when iterating. Report which checks actually ran.
CI runs everything but `test-ios`, which needs Xcode and a simulator.

The workspace is clippy-clean at `-D warnings` and rustfmt-clean; `make fmt`
applies the formatting. Both are gates, so a new warning fails the build rather
than accumulating.

CLI acceptance uses a throwaway directory without network. Prove a moved rule
there before deleting its Swift implementation. No iOS test is expected to fail,
including `ethereumTestNetworksExposeExpectedContextsAndEndpoints`.

## Platform constraints

- Check APIs, syntax and generated bindings against **UniFFI 0.31 and Swift 6**
  before changing FFI or Swift code. See [FFI-BOUNDARY.md](docs/FFI-BOUNDARY.md).
- Never hand-edit `swift/generated/`. Change the Rust API or generator patch
  and regenerate the bindings.
- iOS `reqwest` must use `rustls-tls-webpki-roots`. Native roots are empty on
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
- Name flow extensions `AppState+<Domain>.swift`, topic extensions/free functions
  `Store+<Topic>.swift`, and other files after their type. Split growing topics
  into siblings rather than moving code into unrelated files.
- Never combine `withCheckedContinuation` and `withObservationTracking` in a
  long-lived observation loop: cancellation does not resume the continuation
  and can retain `self`. Use `didSet` with a debounced, cancellable `Task` and
  `Task.sleep` instead.
- Tests use Swift Testing (`@Test`, `#expect`, `#require`), not XCTest. Tests
  run in parallel, so a test that touches process-wide state (the Keychain,
  the network runtime, windows) must stay correct alongside the others or sit
  in a `.serialized` suite. A test that needs an `AppState` uses
  `@Suite(.isolatedAppState)` and conforms to `IsolatedAppStateSuite`.

## Resources

`resources/` is the app target's synchronized group, and Xcode copies such a
group in **flat**: every file under it lands at the bundle's resource root,
whatever directory it sat in. `resources/strings/RuntimeStrings.en.json` ships
as `RuntimeStrings.en.json`.

Two rules follow, and both have already been broken once:

- **Nothing but a runtime resource belongs here.** Anything under `resources/`
  ships, read or not. Build-time inputs go elsewhere — icon sources in
  `icons/`, the translator glossary in `docs/LocalizationGlossary.json`.
- **The file name is the only disambiguator.** Keep `resources/` flat, and put
  the locale in the name (`RuntimeStrings.zh-Hans.json`). Per-locale
  subdirectories look like they separate files and do not: drop the suffix
  trusting the directory and the files silently overwrite each other in the
  bundle.

## Icons

Edit SVG sources in `icons/`, never `swift/Assets.xcassets/` by hand. Keep
sources out of `resources/`; iOS renders the generated asset catalog.

After adding or editing library icons, normalize and export:

```sh
scripts/normalize-icons.sh && scripts/export-swift-icons.sh
```

- `crypto/` and `fiat/` share a 64×64 SVG style and export to matching asset
  groups; only `fiat/` uses a namespace. Keep source and asset group names aligned.
- Use a full-disc `<circle cx="32" cy="32" r="32" fill="…"/>` as the first
  drawable (after any `<defs>`). `usd1` is the exception: its rings form the disc.
- Formatting is owned by `scripts/svgo.config.mjs`; use
  `scripts/normalize-icons.sh --check` to detect drift. Non-square artwork needs
  manual layout; `userSpaceOnUse` gradients may retain transforms.
- `appicon/` is excluded from normalization. Its 1024-point sources export as
  1024×1024 PNGs to `AppIcon.appiconset`; keep that destination name.
