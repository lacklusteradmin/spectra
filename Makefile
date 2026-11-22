SHELL := /bin/bash

.PHONY: fmt lint check-icons test test-cli apple clean

# ── Verification ────────────────────────────────────────────────────
# Four targets, each run by name: lint, test, test-cli and check-icons.
# AGENTS.md says which a change needs. The app's own checks run in
# spectra-apple, given this checkout as its SPECTRA.

# Source scans: nothing public may go unused. They read source and compile
# nothing, so lint's only build is clippy's.
SOURCE_SCANS := scripts/uncalled-core-fns.py

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings
	python3 -B scripts/test-source-scan.py
	@set -e; for scan in $(SOURCE_SCANS); do echo "$$scan"; python3 -B $$scan; done

fmt:
	cargo fmt --all

check-icons:
	scripts/normalize-icons.sh --check

test:
	cargo test

test-cli:
	python3 -B tests/acceptance

# ── Apple package ───────────────────────────────────────────────────
# The SpectraCore Swift package spectra-apple links, written to
# target/SpectraCore/; copy that directory to the app repository's root. The
# minimums are spectra-apple's deployment targets. PROFILE (dev or release)
# and PLATFORMS (a subset of `ios ios-sim macos`) pass through to the script.
IPHONEOS_DEPLOYMENT_TARGET ?= 26.0
MACOSX_DEPLOYMENT_TARGET ?= 26.0

apple:
	IPHONEOS_DEPLOYMENT_TARGET="$(IPHONEOS_DEPLOYMENT_TARGET)" \
	MACOSX_DEPLOYMENT_TARGET="$(MACOSX_DEPLOYMENT_TARGET)" \
		scripts/build-apple-package.sh target/SpectraCore

clean:
	cargo clean
