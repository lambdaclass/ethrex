#!/usr/bin/env bash
#
# Extracts one conformance vector's statelessInputBytes / statelessOutputBytes
# pair into raw binary, for the ere-server acceptance check in tag_release.yaml.
#
# Picks a TRUE-SUCCESS case (successful_validation == 1) deterministically. That
# is not a detail: the root, chain_id and schema_id are all computed before or
# without executing the block, so a guest whose execution is completely broken
# still reproduces a failure vector exactly. Only a success case proves the ELF
# can actually validate a block.
#
# Writes: output/stateless-input.bin, output/expected-output.bin

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VECTORS="$ROOT/tooling/ef_tests/blockchain/vectors_stateless_3278/blockchain_tests"
OUT="$ROOT/output"
mkdir -p "$OUT"

if [[ ! -d $VECTORS ]]; then
    echo "No vectors at $VECTORS; run 'make -C tooling/ef_tests/blockchain stateless-vector' first" >&2
    exit 1
fi

# `find | sort` for determinism, so a failure is reproducible rather than
# dependent on filesystem order. os-walk rather than a glob because the fill
# output nests fixtures under a dot-prefixed work directory.
FOUND=""
while IFS= read -r file; do
    PAIR=$(jq -r '
        to_entries[] | .value                                as $t
        | ($t.blocks // [])[]                                as $b
        | select(($b.statelessInputBytes // "") != "")
        | select(($b.statelessOutputBytes // "") != "")
        # byte 32 of the SSZ result is successful_validation; hex chars 64..66.
        | select(($b.statelessOutputBytes | ltrimstr("0x") | .[64:66]) == "01")
        | "\($b.statelessInputBytes)\t\($b.statelessOutputBytes)"
    ' "$file" 2>/dev/null | head -1 || true)
    if [[ -n $PAIR ]]; then
        FOUND="$file"
        printf '%s' "${PAIR%%$'\t'*}" | sed 's/^0x//' | xxd -r -p > "$OUT/stateless-input.bin"
        printf '%s' "${PAIR##*$'\t'}" | sed 's/^0x//' | xxd -r -p > "$OUT/expected-output.bin"
        break
    fi
done < <(find "$VECTORS" -name '*.json' | sort)

if [[ -z $FOUND ]]; then
    echo "No true-success vector found under $VECTORS" >&2
    echo "A vector set with no successful_validation==1 case cannot prove the ELF executes." >&2
    exit 1
fi

# Belt and braces: assert what we wrote is a 43-byte success result.
LEN=$(wc -c < "$OUT/expected-output.bin" | tr -d ' ')
[[ $LEN -eq 43 ]] || { echo "expected output is $LEN bytes, want 43" >&2; exit 1; }
SUCCESS=$(xxd -p -s 32 -l 1 "$OUT/expected-output.bin")
[[ $SUCCESS == "01" ]] || { echo "expected output is not a success case ($SUCCESS)" >&2; exit 1; }

echo "Using fixture: ${FOUND#"$ROOT"/}"
echo "  input:  $(wc -c < "$OUT/stateless-input.bin" | tr -d ' ') bytes"
echo "  output: $LEN bytes, successful_validation=1"
