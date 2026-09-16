# Working on Spectra

## Git

Do not run any Git state-modifying command (including commit, add/stage, reset
or checkout) unless explicitly requested. Finish edits and stop; the user will
say when to commit.

## Architecture

Read [PLAN.md](PLAN.md), starting with **Rule 0**, which outranks the rules
below: rewrite for simplicity and correctness rather than reproduce existing
behaviour. Record each behaviour change in
[docs/BEHAVIOUR-CHANGES.md](docs/BEHAVIOUR-CHANGES.md) with before/after,
rationale and a CLI check. For funds, keys and addresses,
refuse early, validate before storing and derive rather than trust caller input.

Spectra is prelaunch with no existing users. Change storage formats, schemas,
keychain keys and serialized structures directly; do not add migration or
backward-compatibility shims.

- Core owns domain state and decisions; the CLI proves they need no platform;
  Swift renders and forwards. New domain logic belongs in `core/` and must be
  drivable from `spectra`.
- Per-chain facts belong on `registry::Chain`, not in caller-owned lists.
- Do not add `core_plan_*` functions; core must own the state it decides about.
- Swift may hold view state, not authoritative domain state. If losing data on
  restart would be a bug, core owns it.
- Prefer deleting unnecessary Swift files over porting them.

## Verification

All three suites must pass before calling a change done:

```sh
make verify
```

That is `make lint test test-cli test-ios` — `cargo fmt --check` and
`cargo clippy -- -D warnings`, then `cargo test --workspace`,
`scripts/cli-acceptance.sh`, and `xcodebuild test` on an iPhone simulator. Run
a single one by name when iterating; run `make verify` before calling it done.
CI runs everything but `test-ios`, which needs Xcode and a simulator.

The workspace is clippy-clean at `-D warnings` and rustfmt-clean; `make fmt`
applies the formatting. Both are gates, so a new warning fails the build rather
than accumulating.

CLI acceptance uses a throwaway directory without network. Prove a moved rule
there before deleting its Swift implementation. No iOS test is expected to fail,
including `testEthereumTestNetworksExposeExpectedContextsAndEndpoints`.

## Platform constraints

- Check APIs, syntax and generated bindings against **UniFFI 0.31 and Swift 6**
  before changing FFI or Swift code. See [FFI-BOUNDARY.md](docs/FFI-BOUNDARY.md).
- Never hand-edit `swift/generated/`. Change the Rust API or generator patch
  and regenerate the bindings.
- iOS `reqwest` must use `rustls-tls-webpki-roots`. Native roots are empty on
  iOS and cause HTTPS `UnknownIssuer` failures.
- [docs/iosUI.md](docs/iosUI.md) is the authority for iOS 26 Liquid Glass,
  typography, color, layout and corner radii.

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

## Resources

`resources/` is the app target's synchronized group, and Xcode copies such a
group in **flat**: every file under it lands at the bundle's resource root,
whatever directory it sat in. `resources/strings/CommonContent.en.json` ships
as `CommonContent.en.json`.

Two rules follow, and both have already been broken once:

- **Nothing but a runtime resource belongs here.** Anything under `resources/`
  ships, read or not. Build-time inputs go elsewhere — icon sources in
  `icons/`, the translator glossary in `docs/LocalizationGlossary.json`.
- **The file name is the only disambiguator.** Keep `resources/` flat, and put
  the locale in the name (`CommonContent.zh-Hans.json`). Per-locale
  subdirectories look like they separate files and do not: drop the suffix
  trusting the directory and the files silently overwrite each other in the
  bundle.

## Icons

Edit SVGs in `icons/`, then run `scripts/export-swift-icons.sh`. Never edit
`swift/Assets.xcassets/` by hand.

Icon sources live outside `resources/` for the reason above, sharpened by one
platform fact: iOS has no runtime SVG decoder — `UIImage(contentsOfFile:)`
returns nil and ImageIO registers no SVG UTI — so a bundled `.svg` is pure dead
weight. Only `Assets.car`, which `actool` builds from the catalog, renders.

| Source under `icons/` | Generated destination | Format |
|---|---|---|
| `appicon/` | `AppIcon.appiconset/` | 1024×1024 PNG |
| `crypto/` | `crypto/` | SVG imageset, no namespace |
| `fiat/` | `fiat/` | SVG imageset, provides-namespace |

`crypto/` and `fiat/` map onto their catalog group by identity, so renaming one
is a `git mv` at each end. `appicon/` keeps the suffix rather than shortening to
`app/`: its destination is not free — `ASSETCATALOG_COMPILER_APPICON_NAME =
AppIcon` fixes `AppIcon.appiconset` — and the name mirroring that destination is
what marks it as the odd one out, a PNG source rather than a member of the
library below.

### House style

`icons/crypto/` and `icons/fiat/` are one 64×64 icon library, not a pile
of exporter output. `scripts/normalize-icons.sh` rewrites every icon into that
shape through `scripts/svgo.config.mjs`; `--check` fails when one has drifted.
The pass is idempotent, so run it after adding an icon and then export:

```sh
scripts/normalize-icons.sh && scripts/export-swift-icons.sh
```

Most of the style is svgo's own doing; the parts worth stating:

- `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">` — no `width`,
  `height` or root `fill`, two-space indent, trailing newline.
- A square viewBox at some other size is rescaled onto `0 0 64 64` for you. A
  non-square one is left alone: fitting that artwork onto a disc is a design
  decision, not a rewrite.
- A full-bleed `<circle cx="32" cy="32" r="32" fill="…"/>` as the first drawable,
  after `<defs>` when the icon needs a gradient. Never a path that draws that
  circle. `usd1` is the one exception: its artwork is concentric rings that paint
  the disc themselves.
- Lowercase hex, `#fff` short form, no colour names, and the disc names its fill
  even when it is black — svgo drops `fill="#000"` as a default, so a config
  plugin puts it back.
- Nothing that draws nothing: no `clip-rule` outside `<clipPath>`, no
  `fill="none"` on the root, no editor metadata.

Artwork lands in the 0–64 coordinate space wherever svgo can bake the transform.
Icons whose gradients are `userSpaceOnUse` keep their `<g transform>`, which is
why "no transforms" is not one of the rules.

`icons/appicon/` is out of scope for the normalizer: those are 1024-point
sources for a PNG conversion, not members of the library.
