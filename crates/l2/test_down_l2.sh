#!/usr/bin/env bash
# Regression test for #6912: `make down-l2` signals nothing on macOS.
#
# The recipe used a single `$1` inside an awk program:
#
#     pgrep -a -f "ethrex l2" | awk '!/prover/ {print $1}' | xargs -r kill -s SIGINT
#
# Make expands `$1` (its own automatic variable) to the empty string *before* awk
# ever sees it, so the recipe actually runs `awk '!/prover/ {print }'`. That prints
# the whole `pgrep -a` line -- PID *and* full command line -- and `kill` then aborts
# on the first non-numeric token (`kill: illegal process id: /path/ethrex`), so no
# signal is delivered at all and the node keeps running.
#
# This test extracts the real recipe from the Makefile, runs it through `make -n`
# to observe what the shell would actually receive, and asserts:
#   1. awk receives a literal `$1` (not an eaten one)
#   2. the prover is actually excluded from the kill set
#   3. the signal sent is one the monitor-enabled L2 node exits on
#
# Run: ./test_down_l2.sh   (or: make test-down-l2)
set -uo pipefail

MAKEFILE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/Makefile"
pass=0
fail=0

check() {
    local name="$1" ok="$2" detail="${3:-}"
    if [ "$ok" = "1" ]; then
        printf 'ok   %s\n' "$name"
        pass=$((pass + 1))
    else
        printf 'FAIL %s\n' "$name"
        [ -n "$detail" ] && printf '       %s\n' "$detail"
        fail=$((fail + 1))
    fi
}

if [ ! -f "$MAKEFILE" ]; then
    echo "cannot find $MAKEFILE" >&2
    exit 1
fi

# --- extract the real `down-l2` recipe from the Makefile -------------------------
recipe=$(sed -n '/^down-l2:/,/^$/p' "$MAKEFILE" | sed -n '2,$p' | sed 's/^[[:space:]]*//' | sed '/^$/d')
if [ -z "$recipe" ]; then
    echo "could not extract the down-l2 recipe from $MAKEFILE" >&2
    exit 1
fi

printf '# extracted from crates/l2/Makefile\ndown-l2:\n\t%s\n' "$recipe" > /tmp/down_l2_recipe.mk

# --- 1. what the shell actually receives after make's expansion -----------------
expanded=$(make -n -f /tmp/down_l2_recipe.mk down-l2 2>&1)

# The bug signature: awk's program has `print` with no field reference.
if printf '%s' "$expanded" | grep -qE "awk[^|]*\{ *print *\}"; then
    check "make does not eat awk's \$1" 0 \
          "recipe expands to: $expanded"
else
    check "make does not eat awk's \$1" 1
fi

# --- 2. the prover must be excluded from the kill set ---------------------------
# `pgrep -f` alone matches BOTH `ethrex l2 ...` and `ethrex l2 prover ...`, so the
# exclusion has to be done on the *command line*, not on PID-only output.
if printf '%s' "$expanded" | grep -qE "grep -v prover|awk .*!.*/prover/"; then
    check "prover is filtered on the command line" 1
else
    check "prover is filtered on the command line" 0 \
          "no prover exclusion found in: $expanded"
fi

# A PID-only `pgrep -f` piped to `grep -v prover` silently fails to exclude the
# prover, because "prover" is not present in a bare list of PIDs.
if printf '%s' "$expanded" | grep -qE "pgrep( +-[a-zA-Z]+)* -f" \
   && ! printf '%s' "$expanded" | grep -q "\-a " \
   && printf '%s' "$expanded" | grep -q "grep -v prover"; then
    check "prover filter is not a no-op on PID-only pgrep" 0 \
          "pgrep without -a emits PIDs only, so 'grep -v prover' cannot match"
else
    check "prover filter is not a no-op on PID-only pgrep" 1
fi

# --- 3. the signal must be one the node actually exits on -----------------------
# The L2 monitor is enabled by default (`enabled: !opts.no_monitor` in
# cmd/ethrex/l2/options.rs) and wedges on SIGINT (see #6911), so SIGTERM is the
# signal that reliably stops the node.
if printf '%s' "$expanded" | grep -q "SIGINT"; then
    check "sends a signal the monitor-enabled node exits on" 0 \
          "recipe sends SIGINT; the L2 monitor wedges on SIGINT (#6911)"
else
    check "sends a signal the monitor-enabled node exits on" 1
fi

# --- 4. GNU-only flags ----------------------------------------------------------
# BSD/macOS xargs has no -r.
if printf '%s' "$expanded" | grep -qE "xargs[^|]*[[:space:]]-r([[:space:]]|$)"; then
    check "no GNU-only xargs flags" 0 \
          "recipe uses 'xargs -r', which BSD/macOS xargs rejects"
else
    check "no GNU-only xargs flags" 1
fi

rm -f /tmp/down_l2_recipe.mk

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
