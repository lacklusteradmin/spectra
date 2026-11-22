SHELL := /bin/bash

.PHONY: fmt lint check-icons test test-cli clean

# ── Verification ────────────────────────────────────────────────────
# Four targets, each run by name: lint, test, test-cli and check-icons.
# AGENTS.md says which a change needs. The app's own checks run in
# spectra-apple, against a checkout of this repository beside it.

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

clean:
	cargo clean
