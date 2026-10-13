#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/cli-assertions.sh"
PASSED=0
FAILED=0
fixture() { printf '%s\n' "$1"; return "$2"; }

# False positives: matching error output, absent text on failure, wrong status.
contains 'failed command with matching text' needle fixture needle 7 > /dev/null
lacks 'failed command without forbidden text' forbidden fixture error 7 > /dev/null
contains_exit 3 'wrong rejection status' needle fixture needle 2 > /dev/null
# False negatives and ordinary successes exercise both output predicates.
contains 'missing required text' needle fixture other 0 > /dev/null
lacks 'forbidden text present' forbidden fixture forbidden 0 > /dev/null
contains 'successful match' needle fixture needle 0 > /dev/null
lacks 'successful absence' forbidden fixture other 0 > /dev/null
contains_exit 3 'expected rejection' needle fixture needle 3 > /dev/null
check 'exit only' 2 fixture error 2 > /dev/null
check 'wrong exit' 0 fixture error 2 > /dev/null
[[ "$PASSED" == 4 && "$FAILED" == 6 ]] || {
    printf 'Assertion regression: passed=%s failed=%s\n' "$PASSED" "$FAILED" >&2
    exit 1
}
