#!/usr/bin/env bash
# Build the Rust core as the package an Android app links.
#
#   make android    # into target/core-android/, for spectra-android's minimum
#
#   ANDROID_MIN_SDK=33 scripts/build-android-package.sh <package-dir>
#
# The package is core as a shared library per Android ABI, in `jniLibs/<abi>/`,
# and the UniFFI Kotlin bindings generated from that same library, in
# `kotlin/`. The app's Gradle build takes both as sources and compiles no Rust.
#
# ANDROID_MIN_SDK is the app's minSdk. The NDK's compiler for that API level
# links every library, so a library never calls what the app's oldest device
# lacks.
#
# The NDK is ANDROID_NDK_HOME, or the newest one under ANDROID_HOME's `ndk/`
# (`~/Library/Android/sdk` on a Mac when ANDROID_HOME is unset). NDK r28 or
# newer links with 16 KB pages, which Android 15 devices with that page size
# need.
#
# PROFILE is `dev` (the default) or `release`; `build-inputs` records which.
# ABIS is a subset of `arm64-v8a x86_64`, both by default: phones are arm64,
# and x86_64 is for emulators on Intel hosts.
#
# The package directory is this script's output; give it one of its own. A run
# that finds nothing changed leaves it as it was. The bindings are rewritten
# only when their text changed. The generator patches nothing: a Kotlin
# problem in the bindings is a UniFFI version or API-shape problem.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ $# -ne 1 ]]; then
  echo "usage: ANDROID_MIN_SDK=… $0 <package-dir>" >&2
  exit 2
fi

if [[ ! "${ANDROID_MIN_SDK:-}" =~ ^[0-9]+$ ]]; then
  echo "error: ANDROID_MIN_SDK must be one API level, such as 33; got '${ANDROID_MIN_SDK:-}'" >&2
  exit 2
fi

PROFILE="${PROFILE:-dev}"
case "${PROFILE}" in
  dev) PROFILE_FLAG="" PROFILE_DIR=debug ;;
  release) PROFILE_FLAG=--release PROFILE_DIR=release ;;
  *)
    echo "error: PROFILE must be dev or release; got '${PROFILE}'" >&2
    exit 2
    ;;
esac

ABIS="${ABIS:-arm64-v8a x86_64}"
TRIPLES=()
for abi in ${ABIS}; do
  case "${abi}" in
    arm64-v8a) TRIPLES+=(aarch64-linux-android) ;;
    x86_64) TRIPLES+=(x86_64-linux-android) ;;
    *)
      echo "error: unknown ABI '${abi}'; ABIS takes arm64-v8a and x86_64" >&2
      exit 2
      ;;
  esac
done
if [[ ${#TRIPLES[@]} -eq 0 ]]; then
  echo "error: ABIS names no ABI" >&2
  exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required to build the Rust core; install Rust from https://rustup.rs" >&2
  exit 1
fi

NDK="${ANDROID_NDK_HOME:-}"
if [[ -z "${NDK}" ]]; then
  SDK="${ANDROID_HOME:-${HOME}/Library/Android/sdk}"
  # The newest side-by-side NDK, by version number.
  NDK="$(find "${SDK}/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | sort -V | tail -1 || true)"
fi
if [[ -z "${NDK}" || ! -f "${NDK}/source.properties" ]]; then
  echo "error: no Android NDK found; set ANDROID_NDK_HOME, or install one with sdkmanager \"ndk;<version>\"" >&2
  exit 1
fi
NDK_MAJOR="$(sed -n 's/^Pkg.Revision *= *\([0-9]*\).*/\1/p' "${NDK}/source.properties")"
if [[ "${NDK_MAJOR:-0}" -lt 28 ]]; then
  echo "error: NDK r${NDK_MAJOR} links with 4 KB pages; use r28 or newer: ${NDK}" >&2
  exit 1
fi
case "$(uname -s)" in
  Darwin) HOST_TAG=darwin-x86_64 ;; # Universal binaries, Apple silicon included.
  Linux) HOST_TAG=linux-x86_64 ;;
  *)
    echo "error: no NDK toolchain for $(uname -s)" >&2
    exit 1
    ;;
esac
TOOLCHAIN="${NDK}/toolchains/llvm/prebuilt/${HOST_TAG}/bin"

mkdir -p "$1"
PACKAGE="$(cd "$1" && pwd)"

# Its own target directory: these builds run with the NDK's compilers, which a
# host build sharing target/ would make `cc` rerun.
export CARGO_TARGET_DIR="${ROOT}/target/android"
mkdir -p "${CARGO_TARGET_DIR}"

cd "${ROOT}"

LIBRARIES=()
for triple in "${TRIPLES[@]}"; do
  if command -v rustup >/dev/null 2>&1; then
    rustup target add "${triple}" >/dev/null 2>&1 || true
  fi
  # What `cc` and cargo take for this target: the NDK's clang for the app's
  # minimum, which also links, and its archiver.
  env_triple="${triple//-/_}"
  upper_triple="$(printf '%s' "${env_triple}" | tr '[:lower:]' '[:upper:]')"
  clang="${TOOLCHAIN}/${triple}${ANDROID_MIN_SDK}-clang"
  if [[ ! -x "${clang}" ]]; then
    echo "error: NDK r${NDK_MAJOR} has no compiler for API ${ANDROID_MIN_SDK}: ${clang}" >&2
    exit 1
  fi
  export "CC_${env_triple}=${clang}"
  export "CXX_${env_triple}=${clang}++"
  export "AR_${env_triple}=${TOOLCHAIN}/llvm-ar"
  export "CARGO_TARGET_${upper_triple}_LINKER=${clang}"
  # Core itself is the library the app loads: its rlib built as a shared one.
  # shellcheck disable=SC2086 # PROFILE_FLAG is one flag or none.
  cargo rustc ${PROFILE_FLAG} --manifest-path core/Cargo.toml --target "${triple}" \
    --lib --crate-type cdylib
  LIBRARIES+=("${CARGO_TARGET_DIR}/${triple}/${PROFILE_DIR}/libspectra_core.so")
done

INPUTS="$(
  printf 'profile=%s\n' "${PROFILE}"
  printf 'abis=%s\n' "${ABIS}"
  printf 'android-min-sdk=%s\n' "${ANDROID_MIN_SDK}"
)"
fresh=1
if [[ "$(cat "${PACKAGE}/build-inputs" 2>/dev/null || true)" != "${INPUTS}" ]]; then
  fresh=0
fi
for library in "${LIBRARIES[@]}"; do
  if [[ "${library}" -nt "${PACKAGE}/build-inputs" ]]; then fresh=0; fi
done
if [[ ! -e "${PACKAGE}/kotlin/uniffi/spectra_core/spectra_core.kt" ]]; then fresh=0; fi
for abi in ${ABIS}; do
  if [[ ! -e "${PACKAGE}/jniLibs/${abi}/libspectra_core.so" ]]; then fresh=0; fi
done
if [[ ${fresh} -eq 1 ]]; then
  echo "The Android package is up to date: ${PACKAGE}"
  exit 0
fi

SCRATCH="${CARGO_TARGET_DIR}/package"
rm -rf "${SCRATCH}"
mkdir -p "${SCRATCH}/bindings"

# A host tool, optimized: it reads the whole library, which unoptimized takes
# seconds a run. It reads the unstripped library, whose metadata it needs.
cargo run --release --quiet --manifest-path tools/uniffi-bindgen/Cargo.toml -- \
  generate --language kotlin --no-format --library "${LIBRARIES[0]}" --out-dir "${SCRATCH}/bindings"

replace() {
  if ! cmp -s "$1" "$2"; then
    mkdir -p "$(dirname "$2")"
    cp "$1" "$2"
  fi
}
replace "${SCRATCH}/bindings/uniffi/spectra_core/spectra_core.kt" \
  "${PACKAGE}/kotlin/uniffi/spectra_core/spectra_core.kt"

# Stripped of everything the loader does not read: a dev library's debug
# information and symbol names come to about three hundred megabytes, all of
# it copied to the device on every install. The unstripped library stays in
# target/android/ for `ndk-stack` to name a native crash's functions.
rm -rf "${PACKAGE}/jniLibs"
i=0
for abi in ${ABIS}; do
  mkdir -p "${PACKAGE}/jniLibs/${abi}"
  "${TOOLCHAIN}/llvm-strip" --strip-all -o "${PACKAGE}/jniLibs/${abi}/libspectra_core.so" "${LIBRARIES[${i}]}"
  i=$((i + 1))
done

# Last, so a run that failed part way is not taken for a finished one.
printf '%s\n' "${INPUTS}" >"${PACKAGE}/build-inputs"
echo "The Android package built (${PROFILE}: ${ABIS}): ${PACKAGE}"
