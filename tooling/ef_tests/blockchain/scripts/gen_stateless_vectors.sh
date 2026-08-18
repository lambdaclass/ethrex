#!/usr/bin/env bash
#
# Generate stateless-validation conformance vectors by filling the upstream
# EIP-8025 tests at a pinned execution-specs commit.
#
# Pinned to 3c3b6f4af315b268a61e20d5a4da8aa4f24c91f0 -- execution-specs master at
# the merge of #3278, which is #3248 (progressive SSZ) + #3278 (ChainConfig
# removal). No tests-zkevm release carries this schema: v0.6.2 (2026-07-13)
# predates both, and it reuses schema id 0x1501 for the OLD body, so its vectors
# misparse against this schema. Delete this script and switch back to
# zkevm-vectors when a tests-zkevm v0.7.x exists.
#
# Two upstream quirks are load-bearing:
#   - Python 3.14 cannot build coincurve (scikit-build-core); we pin 3.12 via uv.
#   - The spec's wheel omits forks/amsterdam/execution_engine/, so we run the
#     source tree via PYTHONPATH and install only its dependencies. Do not
#     "fix" this by relying on the installed package.
#
# Usage: gen_stateless_vectors.sh [output-dir]

set -euo pipefail

SPEC_SHA=3c3b6f4af315b268a61e20d5a4da8aa4f24c91f0
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/vectors_stateless_3278}"
WORK="${STATELESS_VECTOR_WORKDIR:-$ROOT/.stateless-vector-work}"
SPECS="$WORK/execution-specs"
VENV="$WORK/venv"

command -v uv >/dev/null || { echo "uv is required (brew install uv)" >&2; exit 1; }

mkdir -p "$WORK"
if [[ ! -d $SPECS/.git ]]; then
    git clone --filter=blob:none --quiet https://github.com/ethereum/execution-specs "$SPECS"
fi
git -C "$SPECS" checkout --quiet "$SPEC_SHA"
[[ "$(git -C "$SPECS" rev-parse HEAD)" == "$SPEC_SHA" ]] || {
    echo "checkout drifted from $SPEC_SHA" >&2; exit 1; }

if [[ ! -x $VENV/bin/fill ]]; then
    uv venv --python 3.12 "$VENV"
    VIRTUAL_ENV="$VENV" uv pip install --quiet "$SPECS"
    VIRTUAL_ENV="$VENV" uv pip install --quiet "$SPECS/packages/testing"
fi

rm -rf "$OUT"
# PYTHONPATH shadows the installed copy with the source tree; see the header.
PYTHONPATH="$SPECS/src" "$VENV/bin/fill" \
    "$SPECS/tests/amsterdam/eip8025_optional_proofs/" \
    --fork Amsterdam --output "$OUT" -q --no-html

# Fail loudly if the vector set is not what the downstream tasks assume.
"$VENV/bin/python" - "$OUT" <<'EOF'
import json, os, sys

out = sys.argv[1]
succ = fail = 0
bad_len = []
# os.walk, not glob("**"): `fill` embeds the input path in the output path, so the
# fixtures sit under the dot-prefixed work dir, and glob's ** skips hidden dirs.
files = [
    os.path.join(d, n)
    for d, _, ns in os.walk(os.path.join(out, "blockchain_tests"))
    for n in ns
    if n.endswith(".json")
]
for f in files:
    for _, t in json.load(open(f)).items():
        for b in t.get("blocks", []):
            o = b.get("statelessOutputBytes")
            if not o:
                continue
            raw = bytes.fromhex(o.removeprefix("0x"))
            if len(raw) != 43:
                bad_len.append(len(raw))
            succ += raw[32] == 1
            fail += raw[32] == 0
print(f"vectors: {succ} success, {fail} failure")
if bad_len:
    sys.exit(f"FAIL: non-43-byte outputs {sorted(set(bad_len))} -- pre-#3278 schema?")
if succ == 0:
    sys.exit("FAIL: no true-success vector; the set cannot prove execution works")
EOF
