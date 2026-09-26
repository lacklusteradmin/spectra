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
# `verify` is the gate AGENTS.md describes: all three suites must pass
# before a change is done. CI runs `lint test test-cli`; `test-ios`
# needs Xcode and a simulator, so it stays local.
verify: lint test test-cli test-ios

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

check-ui:
	scripts/check-design-tokens.sh
	scripts/normalize-icons.sh --check

test:
	cargo test --workspace

test-cli:
	scripts/cli-acceptance.sh

test-ios:
	xcodebuild test -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" \
		-destination "$(IOS_TEST_DEST)" \
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
	rm -rf build/ swift/generated/ kotlin/app/src/main/kotlin/uniffi/ kotlin/app/src/main/jniLibs/
