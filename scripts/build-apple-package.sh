#!/usr/bin/env bash
# Build the Rust core as a local Swift package for an Apple app.
#
#   make apple    # into target/SpectraCore/, for spectra-apple's minimums
#
#   IPHONEOS_DEPLOYMENT_TARGET=26.0 MACOSX_DEPLOYMENT_TARGET=26.0 \
#     scripts/build-apple-package.sh <package-dir>
#
# The package is the SpectraCore library: core as a static library per
# platform, bundled as `spectra_core.xcframework`; the C header of its FFI as
# the `spectra_coreFFI` module; and the UniFFI Swift bindings generated from
# that same library as the `SpectraCore` target. The app links it and compiles
# no Rust.
#
# The two minimums are the app's. Every object in a library is stamped with
# one, and a library stamped newer than the app's minimum fails the app's link.
#
# PROFILE is `dev` (the default) or `release`; a release app ships a release
# package, and `build-inputs` records which this is. PLATFORMS is a subset of
# `ios ios-sim macos`, all three by default; an app build for a platform the
# package lacks fails in Xcode.
#
# The package directory is this script's output; give it one of its own. A run
# that finds nothing changed leaves it as it was, and the header and bindings
# are rewritten only when their text changed, so an edit that leaves the FFI as
# it was recompiles no Swift in an app that copies the package with its
# modification times (a Finder drag, `cp -Rp` or `ditto`). The generator patches nothing: a Swift 6
# problem in the bindings is a UniFFI version or API-shape problem.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -ne 1 ]]; then
  echo "usage: IPHONEOS_DEPLOYMENT_TARGET=… MACOSX_DEPLOYMENT_TARGET=… $0 <package-dir>" >&2
  exit 2
fi

for variable in IPHONEOS_DEPLOYMENT_TARGET MACOSX_DEPLOYMENT_TARGET; do
  if [[ ! "${!variable:-}" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]]; then
    echo "error: ${variable} must be one version, such as 26.0; got '${!variable:-}'" >&2
    exit 2
  fi
  export "${variable?}"
done

PROFILE="${PROFILE:-dev}"
case "${PROFILE}" in
  dev) PROFILE_FLAG="" PROFILE_DIR=debug ;;
  release) PROFILE_FLAG=--release PROFILE_DIR=release ;;
  *)
    echo "error: PROFILE must be dev or release; got '${PROFILE}'" >&2
    exit 2
    ;;
esac

PLATFORMS="${PLATFORMS:-ios ios-sim macos}"
TRIPLES=()
for platform in ${PLATFORMS}; do
  case "${platform}" in
    ios) TRIPLES+=(aarch64-apple-ios) ;;
    ios-sim) TRIPLES+=(aarch64-apple-ios-sim) ;;
    macos) TRIPLES+=(aarch64-apple-darwin) ;;
    *)
      echo "error: unknown platform '${platform}'; PLATFORMS takes ios, ios-sim and macos" >&2
      exit 2
      ;;
  esac
done
if [[ ${#TRIPLES[@]} -eq 0 ]]; then
  echo "error: PLATFORMS names no platform" >&2
  exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required to build the Rust core; install Rust from https://rustup.rs" >&2
  exit 1
fi

mkdir -p "$1"
PACKAGE="$(cd "$1" && pwd)"

# Its own target directory. These builds run with the app's minimums set, and
# `cc` reruns a build script when one changes, so sharing the workspace's
# target/ would rebuild what terminal cargo builds without them, and back.
export CARGO_TARGET_DIR="${ROOT}/target/apple"
mkdir -p "${CARGO_TARGET_DIR}"

# From the checkout, so rustup takes the toolchain rust-toolchain.toml pins.
cd "${ROOT}"

LIBRARIES=()
for triple in "${TRIPLES[@]}"; do
  case "${triple}" in
    aarch64-apple-darwin) minimum="${MACOSX_DEPLOYMENT_TARGET}" ;;
    *) minimum="${IPHONEOS_DEPLOYMENT_TARGET}" ;;
  esac
  # `ring` 0.16.20, reached through arti's `x509-signature`, has a build script
  # that emits no `rerun-if-env-changed`, so cargo keeps its C objects across a
  # change of minimum and the link warns about them on every build. Only
  # discarding the platform's objects helps.
  stamp="${CARGO_TARGET_DIR}/${triple}.minimum"
  if [[ "$(cat "${stamp}" 2>/dev/null || true)" != "${minimum}" ]]; then
    rm -rf "${CARGO_TARGET_DIR:?}/${triple}"
    printf '%s' "${minimum}" >"${stamp}"
  fi

  if command -v rustup >/dev/null 2>&1; then
    rustup target add "${triple}" >/dev/null 2>&1 || true
  fi
  # Core itself is the library the app links: its rlib built as a static one.
  # shellcheck disable=SC2086 # PROFILE_FLAG is one flag or none.
  cargo rustc ${PROFILE_FLAG} --manifest-path core/Cargo.toml --target "${triple}" \
    --lib --crate-type staticlib
  LIBRARIES+=("${CARGO_TARGET_DIR}/${triple}/${PROFILE_DIR}/libspectra_core.a")
done

INPUTS="$(
  printf 'profile=%s\n' "${PROFILE}"
  printf 'platforms=%s\n' "${PLATFORMS}"
  printf 'iphoneos-deployment-target=%s\n' "${IPHONEOS_DEPLOYMENT_TARGET}"
  printf 'macosx-deployment-target=%s\n' "${MACOSX_DEPLOYMENT_TARGET}"
)"
fresh=1
if [[ "$(cat "${PACKAGE}/build-inputs" 2>/dev/null || true)" != "${INPUTS}" ]]; then
  fresh=0
fi
for library in "${LIBRARIES[@]}"; do
  if [[ "${library}" -nt "${PACKAGE}/build-inputs" ]]; then fresh=0; fi
done
for output in Package.swift spectra_core.xcframework Sources/SpectraCore/spectra_core.swift \
  Sources/spectra_coreFFI/spectra_coreFFI.h Sources/spectra_coreFFI/module.modulemap; do
  if [[ ! -e "${PACKAGE}/${output}" ]]; then fresh=0; fi
done
if [[ ${fresh} -eq 1 ]]; then
  echo "SpectraCore is up to date: ${PACKAGE}"
  exit 0
fi

SCRATCH="${CARGO_TARGET_DIR}/package"
rm -rf "${SCRATCH}"
mkdir -p "${SCRATCH}/bindings"

# A host tool, optimized: it reads the whole library, which unoptimized takes
# seconds a run.
cargo run --release --quiet --manifest-path tools/uniffi-bindgen/Cargo.toml -- \
  generate --language swift --library "${LIBRARIES[0]}" --out-dir "${SCRATCH}/bindings"

# Libraries only. The header stays out of the XCFramework, which is rebuilt
# for every core edit: Xcode recompiles every Swift file above a C module whose
# header looks new, and the header changes only with the FFI.
XCFRAMEWORK_ARGS=()
for library in "${LIBRARIES[@]}"; do
  XCFRAMEWORK_ARGS+=(-library "${library}")
done
xcodebuild -create-xcframework "${XCFRAMEWORK_ARGS[@]}" \
  -output "${SCRATCH}/spectra_core.xcframework" >/dev/null

cat >"${SCRATCH}/Package.swift" <<EOF
// swift-tools-version: 6.0
// Written by spectra's scripts/build-apple-package.sh, with the rest of this
// directory. Do not edit: rerun the script.
import PackageDescription

let package = Package(
    name: "SpectraCore",
    platforms: [.iOS("${IPHONEOS_DEPLOYMENT_TARGET}"), .macOS("${MACOSX_DEPLOYMENT_TARGET}")],
    products: [
        .library(name: "SpectraCore", targets: ["SpectraCore"]),
    ],
    targets: [
        .binaryTarget(name: "spectra_core", path: "spectra_core.xcframework"),
        .systemLibrary(name: "spectra_coreFFI"),
        .target(name: "SpectraCore", dependencies: ["spectra_core", "spectra_coreFFI"]),
    ]
)
EOF

# Replace only what changed: a new modification time alone makes Xcode
# re-resolve the package or recompile what imports it.
replace() {
  if ! cmp -s "$1" "$2"; then
    mkdir -p "$(dirname "$2")"
    cp "$1" "$2"
  fi
}
replace "${SCRATCH}/Package.swift" "${PACKAGE}/Package.swift"
replace "${SCRATCH}/bindings/spectra_core.swift" "${PACKAGE}/Sources/SpectraCore/spectra_core.swift"
replace "${SCRATCH}/bindings/spectra_coreFFI.h" "${PACKAGE}/Sources/spectra_coreFFI/spectra_coreFFI.h"
replace "${SCRATCH}/bindings/spectra_coreFFI.modulemap" "${PACKAGE}/Sources/spectra_coreFFI/module.modulemap"
rm -rf "${PACKAGE}/spectra_core.xcframework"
mv "${SCRATCH}/spectra_core.xcframework" "${PACKAGE}/spectra_core.xcframework"

# Last, so a run that failed part way is not taken for a finished one.
printf '%s\n' "${INPUTS}" >"${PACKAGE}/build-inputs"
echo "SpectraCore built (${PROFILE}: ${PLATFORMS}): ${PACKAGE}"
