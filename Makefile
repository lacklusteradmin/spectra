SHELL := /bin/bash

XCODE_PROJECT := swift/Spectra.xcodeproj
XCODE_SCHEME ?= Spectra
IOS_SIM_DEST ?= generic/platform=iOS Simulator
IOS_TEST_DEST ?= platform=iOS Simulator,name=iPhone 17 Pro
# Optional isolation when another task is building the same Xcode project.
IOS_TEST_DERIVED_DATA ?=

.PHONY: verify fmt lint check-ui test test-cli test-ios \
	ios iosr android androidr \
	bindgen-android clean

# ── Verification ────────────────────────────────────────────────────
# `verify` runs the full gate for major changes described in AGENTS.md.
# CI runs `lint test test-cli`; `test-ios`
# needs Xcode and a simulator, so it stays local.
verify: lint test test-cli test-ios

# Source scans: nothing exported, public or shipped may go unused, no FFI
# record field may go unwritten, and the app may not spell chain names or
# amount precision itself.
SOURCE_SCANS := scripts/unreachable-exports.sh scripts/uncalled-core-fns.sh \
	scripts/unwritten-record-fields.sh scripts/unused-strings.sh \
	scripts/swift-shell-literals.sh

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings
	python3 -B scripts/test-source-scan.py
	python3 -B scripts/test-record-writers.py
	@set -e; for scan in $(SOURCE_SCANS); do echo "$$scan"; $$scan; done

fmt:
	cargo fmt --all

check-ui:
	scripts/check-design-tokens.sh
	scripts/normalize-icons.sh --check

test:
	cargo test --workspace

test-cli:
	scripts/cli-acceptance.sh

# No sysdiagnose: a failing test's own output says what failed, and the
# collection can wait out a ten-minute timeout on a simulator another tool is
# attached to, even after every test passed.
test-ios:
	xcodebuild test -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" \
		-destination "$(IOS_TEST_DEST)" -collect-test-diagnostics never \
		$(if $(IOS_TEST_DERIVED_DATA),-derivedDataPath "$(IOS_TEST_DERIVED_DATA)")

# ── Builds ──────────────────────────────────────────────────────────
ios:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Debug -destination "$(IOS_SIM_DEST)" build

iosr:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Release -destination "$(IOS_SIM_DEST)" build

android:
	scripts/build-android.sh
	scripts/bindgen-android.sh

androidr:
	scripts/build-android.sh --release
	scripts/bindgen-android.sh

bindgen-android:
	scripts/bindgen-android.sh

clean:
	cargo clean
	rm -rf swift/generated/ kotlin/app/src/main/kotlin/uniffi/ kotlin/app/src/main/jniLibs/
