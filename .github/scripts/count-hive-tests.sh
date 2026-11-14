#!/usr/bin/env bash
#
# Prints how many tests a hive run executed, from the JSON results in a
# directory (default: src/results, where the hive action leaves them).
#
#   .github/scripts/count-hive-tests.sh [results directory]
#
# The EELS simulators add cases of their own to a suite, such as "Base Hive
# Test" and "eels/consume-wirex-multi-test-clients", which pass even when the
# simulator found no fixture test to run. In an `eels/` suite only the cases
# with a parameter list, as in `test_x[fork_Prague-...]`, are counted, so a
# limit that matches nothing counts as zero. Every case of the other suites is
# a test.

set -euo pipefail

results_dir="${1:-src/results}"

shopt -s nullglob
total=0
for file in "$results_dir"/*.json; do
    [[ $file == *hive.json ]] && continue
    cases=$(jq '(.name // "") as $suite
        | [.testCases[]? | select(($suite | startswith("eels/") | not) or ((.name // "") | contains("[")))]
        | length' "$file")
    total=$((total + cases))
done
echo "$total"
