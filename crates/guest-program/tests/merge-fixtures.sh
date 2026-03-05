#!/bin/bash
# Merge committer.json + prover.json into a single test fixture.
# Usage: ./merge-fixtures.sh <dump-dir>/<app>/batch_<N>
#
# Reads:  committer.json, prover.json  (from ETHREX_DUMP_FIXTURES output)
# Writes: fixture.json                 (test-ready, same format as tests/fixtures/)
#
# Example:
#   ETHREX_DUMP_FIXTURES=/tmp/fixtures docker compose up
#   ./merge-fixtures.sh /tmp/fixtures/zk-dex/batch_11
#   cp /tmp/fixtures/zk-dex/batch_11/fixture.json \
#      crates/guest-program/tests/fixtures/zk-dex/batch_11.json

set -euo pipefail

DIR="${1:?Usage: $0 <batch-dir>}"

COMMITTER="$DIR/committer.json"
PROVER="$DIR/prover.json"

if [ ! -f "$COMMITTER" ]; then echo "Missing $COMMITTER"; exit 1; fi
if [ ! -f "$PROVER" ]; then echo "Missing $PROVER"; exit 1; fi

# Use python3 (or jq) to merge
python3 -c "
import json, sys

with open('$COMMITTER') as f:
    c = json.load(f)
with open('$PROVER') as f:
    p = json.load(f)

fixture = {
    'app': c.get('app', 'unknown'),
    'batch_number': c.get('batch_number', 0),
    'program_type_id': c.get('program_type_id', 0),
    'chain_id': c.get('chain_id', 0),
    'description': c.get('description', ''),
    'prover': p,
    'committer': c.get('committer', {}),
}

with open('$DIR/fixture.json', 'w') as f:
    json.dump(fixture, f, indent=2)

print(f'Written: $DIR/fixture.json')
"
