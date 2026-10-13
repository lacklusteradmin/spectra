#!/usr/bin/env bash
# Sourced by CLI acceptance and its harness regression check.
# The caller owns PASSED and FAILED; assertions keep running after a failure.
check() {
    local description="$1" expected="$2"
    shift 2
    assert_result "$description" "$expected" any '' "$@"
}

contains() { contains_exit 0 "$@"; }
contains_exit() {
    local expected="$1" description="$2" needle="$3"
    shift 3
    assert_result "$description" "$expected" contains "$needle" "$@"
}

lacks() {
    local description="$1" needle="$2"
    shift 2
    assert_result "$description" 0 lacks "$needle" "$@"
}

assert_result() {
    local description="$1" expected="$2" match="$3" needle="$4"
    shift 4
    local output status=0 matched=0
    output="$("$@" 2>&1)" || status=$?
    case "$match" in
        any) matched=1 ;;
        contains) [[ "$output" == *"$needle"* ]] && matched=1 ;;
        lacks) [[ "$output" != *"$needle"* ]] && matched=1 ;;
    esac
    if [[ "$status" == "$expected" && "$matched" == 1 ]]; then
        PASSED=$((PASSED + 1))
        printf '  \033[32m✓\033[0m %s\n' "$description"
    else
        FAILED=$((FAILED + 1))
        printf '  \033[31m✗\033[0m %s (exit %s, wanted %s; %s %s)\n' \
            "$description" "$status" "$expected" "$match" "$needle"
        printf '    %s\n' "$output"
    fi
}
