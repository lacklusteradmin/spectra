.PHONY: ios android test check clean bindgen-ios bindgen-android

# Build iOS static libraries and regenerate Swift bindings
ios:
	scripts/build-ios.sh
	scripts/bindgen-ios.sh

# Build Android .so libraries and regenerate Kotlin bindings
android:
	scripts/build-android.sh
	scripts/bindgen-android.sh

# Run Rust unit tests (no mobile toolchain required)
test:
	cargo test -p spectra_core

# Type-check without codegen
check:
	cargo check --workspace

# Regenerate Swift bindings from a previously built host dylib
bindgen-ios:
	scripts/bindgen-ios.sh

# Regenerate Kotlin bindings from a previously built host dylib
bindgen-android:
	scripts/bindgen-android.sh

# Remove all build artifacts
clean:
	cargo clean
	rm -rf build/
