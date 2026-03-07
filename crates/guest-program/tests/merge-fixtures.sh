#!/bin/bash
# Merge committer.json + prover.json into a single test fixture.
# Usage: ./merge-fixtures.sh <dump-dir>/<app>/batch_<N>
#
# Reads:  committer.json (required), prover.json (optional)
# Writes: fixture.json   (test-ready, same format as tests/fixtures/)
#
# If prover.json is missing (e.g. exec backend), the fixture will have
# prover: null — tests automatically skip prover-specific checks.
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

python3 -c "
import json, sys, os

with open('$COMMITTER') as f:
    c = json.load(f)

prover_data = None
if os.path.isfile('$PROVER'):
    with open('$PROVER') as f:
        prover_data = json.load(f)

fixture = {
    'app': c.get('app', 'unknown'),
    'batch_number': c.get('batch_number', 0),
    'program_type_id': c.get('program_type_id', 0),
    'chain_id': c.get('chain_id', 0),
    'description': c.get('description', ''),
    'committer': c.get('committer', {}),
}
if prover_data is not None:
    fixture['prover'] = prover_data

with open('$DIR/fixture.json', 'w') as f:
    json.dump(fixture, f, indent=2)

suffix = ' (with prover)' if prover_data else ' (committer only)'
print(f'Written: $DIR/fixture.json' + suffix)
"
