SHELL := /bin/bash

.DEFAULT_GOAL := help

XCODE_PROJECT := swift/Spectra.xcodeproj
XCODE_SCHEME ?= Spectra
IOS_SIM_DEST ?= generic/platform=iOS Simulator

.PHONY: help check test ios iosr ios-artifacts ios-artifacts-release android androidr bindgen-ios bindgen-android clean clean-generated

help:
	@printf "Spectra build targets\n"
	@printf "\n"
	@printf "  make check                 Cargo check the full Rust workspace\n"
	@printf "  make test                  Run spectra_core Rust tests\n"
	@printf "  make ios                   Build the iOS app through Xcode (Debug simulator)\n"
	@printf "  make iosr                  Build the iOS app through Xcode (Release simulator)\n"
	@printf "  make ios-artifacts         Build standalone iOS Rust libs + Swift bindings\n"
	@printf "  make ios-artifacts-release Build standalone release iOS Rust libs + Swift bindings\n"
	@printf "  make android               Build Android Rust libs + Kotlin bindings (debug)\n"
	@printf "  make androidr              Build Android Rust libs + Kotlin bindings (release)\n"
	@printf "  make bindgen-ios           Regenerate Swift bindings only\n"
	@printf "  make bindgen-android       Regenerate Kotlin bindings only\n"
	@printf "  make clean                 Remove Rust, mobile, and generated artifacts\n"

check:
	cargo check --workspace

test:
	cargo test -p spectra_core

ios:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Debug -destination "$(IOS_SIM_DEST)" build

iosr:
	xcodebuild -project "$(XCODE_PROJECT)" -scheme "$(XCODE_SCHEME)" -configuration Release -destination "$(IOS_SIM_DEST)" build

ios-artifacts:
	scripts/build-ios.sh
	scripts/bindgen-ios.sh

ios-artifacts-release:
	scripts/build-ios.sh --release
	scripts/bindgen-ios.sh

android:
	scripts/build-android.sh
	scripts/bindgen-android.sh

androidr:
	scripts/build-android.sh --release
	scripts/bindgen-android.sh

bindgen-ios:
	scripts/bindgen-ios.sh

bindgen-android:
	scripts/bindgen-android.sh

clean: clean-generated
	cargo clean
	rm -rf build/

clean-generated:
	rm -rf swift/generated/ kotlin/app/src/main/kotlin/uniffi/ kotlin/app/src/main/jniLibs/
