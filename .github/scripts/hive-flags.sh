#!/usr/bin/env bash
#
# Builds the hive flags and picks the hive commit for one simulation, from the
# pins in .github/config/hive/pins.yaml. PR CI and the daily report both call
# it, so the two cannot drift apart.
#
#   .github/scripts/hive-flags.sh --simulation ethereum/eels/consume-engine \
#       --limit '.*fork_Prague.*' --parallelism 4 --loglevel 1
#
# Prints two lines in the form a workflow appends to "$GITHUB_OUTPUT":
#
#   flags=--sim.parallelism 4 --sim.loglevel 1 ...
#   hive_version=<commit>
#
# Options:
#   --simulation <name>   hive simulator, as passed to `--sim` (required)
#   --limit <regex>       `--sim.limit`; empty or absent runs every test
#   --parallelism <n>     `--sim.parallelism` (default 4)
#   --loglevel <n>        `--sim.loglevel` (default 3)
#   --fork <name>         fork to run, for the execute simulators
#   --pins <file>         pin file (default: the one in this repository)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PINS="$ROOT/.github/config/hive/pins.yaml"

die() { echo "hive-flags: $*" >&2; exit 1; }

simulation=""
limit=""
parallelism=4
loglevel=3
fork=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --simulation|--limit|--parallelism|--loglevel|--fork|--pins)
            [[ $# -ge 2 ]] || die "$1 needs a value"
            case "$1" in
                --simulation)  simulation=$2 ;;
                --limit)       limit=$2 ;;
                --parallelism) parallelism=$2 ;;
                --loglevel)    loglevel=$2 ;;
                --fork)        fork=$2 ;;
                --pins)        PINS=$2 ;;
            esac
            shift 2
            ;;
        *) die "unknown option: $1" ;;
    esac
done
[[ -n $simulation ]] || die "--simulation is required"
[[ -f $PINS ]] || die "no pin file at $PINS"

# A pin is a `key: value` line. A missing or empty pin is an error rather than
# an empty flag, which hive would accept and then run the wrong thing.
pin() {
    local value
    value=$(sed -n "s/^$1:[[:space:]]*//p" "$PINS")
    [[ -n $value ]] || die "pin '$1' is missing from $PINS"
    [[ $value != *$'\n'* ]] || die "pin '$1' appears more than once in $PINS"
    printf '%s' "$value"
}

hive_version=$(pin hive_version)
# `--docker.buildoutput` relays the output of image builds. Without it a failed
# simulator build only reports the exit code of the step that failed.
flags="--sim.parallelism $parallelism --sim.loglevel $loglevel --docker.buildoutput"

case "$simulation" in
    ethereum/eels/consume-wirex)
        hive_version=$(pin consume_wirex_hive_version)
        flags+=" --sim.buildarg tag=$(pin consume_wirex_tag)"
        ;;
    ethereum/eels/*)
        # The EELS simulators run from the image execution-specs publishes for
        # the pinned fixture release.
        flags+=" --sim.buildarg tag=$(pin tag)"
        ;;
    ethereum/rpc-compat)
        hive_version=$(pin rpc_compat_hive_version)
        flags+=" --sim.buildarg branch=$(pin rpc_compat_branch)"
        ;;
esac

case "$simulation" in
    ethereum/eels/consume-engine|ethereum/eels/consume-rlp)
        flags+=" --client.checktimelimit=180s"
        ;;
    ethereum/eels/execute-blobs)
        # The execute simulators drive a live client over RPC instead of
        # consuming fixtures, so the caller names the fork to run.
        if [[ -n $fork ]]; then
            flags+=" --sim.buildarg fork=$fork"
        fi
        flags+=" --client.checktimelimit=300s"
        ;;
esac

if [[ -n $limit ]]; then
    # Single-quote the limit so regex metacharacters, such as the `|` of a
    # multi-fork limit, survive the hive action's shell re-evaluation of the
    # flags. The quote escaping is an assignment of its own: inside a
    # double-quoted string bash 3.2 keeps the backslashes of the replacement.
    escaped_limit=${limit//\'/\'\\\'\'}
    flags+=" --sim.limit '$escaped_limit'"
fi

echo "flags=$flags"
echo "hive_version=$hive_version"
