SHELL := /bin/bash

XCODE_PROJECT := swift/Spectra.xcodeproj
XCODE_SCHEME ?= Spectra
IOS_SIM_DEST ?= generic/platform=iOS Simulator
# The first available iPhone simulator, unless a UDID is given.
IOS_TEST_SIM ?= $(shell xcrun simctl list --json devices available | python3 -c "import json, sys; print(next(d['udid'] for r in json.load(sys.stdin)['devices'].values() for d in r if d['name'].startswith('iPhone')))")
# Optional isolation when another task is building the same Xcode project.
IOS_TEST_DERIVED_DATA ?=
# Macs are arm64 only, as the project's simulators are.
MAC_DEST ?= platform=macOS,arch=arm64

.PHONY: fmt lint check-ui test test-cli test-ios test-mac ios iosr mac macr clean

# ── Verification ────────────────────────────────────────────────────
# Six targets, each run by name: lint, test, test-cli, check-ui, test-ios and
# test-mac. AGENTS.md says which a change needs. CI runs all but `test-ios`
# and `test-mac`, which need Xcode, so they stay local.

# Source scans: nothing exported, public or shipped may go unused, and the app
# may not spell chain names or amount precision itself. They read source and
# compile nothing, so lint's only build is clippy's.
SOURCE_SCANS := scripts/unreachable-exports.py scripts/uncalled-core-fns.py \
	scripts/unused-strings.py scripts/swift-shell-literals.py

lint:
	cargo fmt --all -- --check
	cargo clippy --all-targets -- -D warnings
	python3 -B scripts/test-source-scan.py
	@set -e; for scan in $(SOURCE_SCANS); do echo "$$scan"; python3 -B $$scan; done

fmt:
	cargo fmt --all

check-ui:
	python3 -B scripts/check-design-tokens.py
	scripts/normalize-icons.sh --check

test:
	cargo test

test-cli:
	python3 -B tests/acceptance

# No sysdiagnose: a failing test's own output says what failed, and the
# collection can wait out a ten-minute timeout on a simulator another tool is
# attached to, even after every test passed.
#
# Booted first, because xcodebuild shuts down a simulator it booted itself and
# booting one again costs about 40 seconds a run. It stays up between runs;
# `xcrun simctl shutdown all` gives its memory back.
test-ios:
	xcrun simctl bootstatus "$(IOS_TEST_SIM)" -b > /dev/null
	xcodebuild test -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" \
		-destination "platform=iOS Simulator,id=$(IOS_TEST_SIM)" -collect-test-diagnostics never \
		$(if $(IOS_TEST_DERIVED_DATA),-derivedDataPath "$(IOS_TEST_DERIVED_DATA)")

# The same suite, hosted by the Mac app. It needs the app signed with its
# keychain access group — Xcode's automatic signing, with a Mac development
# profile for the team — because the Keychain tests use the real data
# protection keychain, which refuses an app without one.
test-mac:
	xcodebuild test -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" \
		-destination "$(MAC_DEST)" -collect-test-diagnostics never

# ── Builds ──────────────────────────────────────────────────────────
ios:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Debug -destination "$(IOS_SIM_DEST)" build

iosr:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Release -destination "$(IOS_SIM_DEST)" build

mac:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Debug -destination "$(MAC_DEST)" build

macr:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Release -destination "$(MAC_DEST)" build

clean:
	cargo clean
	rm -rf swift/generated/
