#!/usr/bin/env bash
#
# Prints the zkVM SDK version the stateless-validator guest is built against, for
# use in release asset names.
#
# The guests reach their SDK through `ere-platform-{zisk,sp1,openvm}`, so there is
# no direct `tag = "vX.Y.Z"` in their manifests to read. The versions are instead
# fixed by the pinned `ere` revision below, mirroring what `ere-catalog` resolves
# at that rev. Bumping the rev therefore REQUIRES updating this table, and the
# consistency check enforces that: it fails if the manifests disagree with
# ERE_REV, so a silent rev bump cannot mislabel an artifact.
#
# Usage: zkvm-version.sh <zisk|sp1|openvm>

set -euo pipefail

ERE_REV=a25f1aed9664c3b63e73ef05360090a4c41da31b

# SDK versions resolved by ere-catalog at ERE_REV.
zkvm_version() {
    case "$1" in
        zisk)   echo "1.0.0-alpha" ;;
        sp1)    echo "6.3.1" ;;
        openvm) echo "2.0.0" ;;
        *)      echo "unknown zkvm: $1" >&2; return 1 ;;
    esac
}

ZKVM="${1:?usage: zkvm-version.sh <zisk|sp1|openvm>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/crates/guest-program/stateless-validator/bin/$ZKVM/Cargo.toml"

[[ -f $MANIFEST ]] || { echo "no manifest at $MANIFEST" >&2; exit 1; }

# The table above is only valid for ERE_REV; refuse to guess if it has moved.
if ! grep -q "rev = \"$ERE_REV\"" "$MANIFEST"; then
    {
        echo "ere rev in $MANIFEST does not match ERE_REV ($ERE_REV)."
        echo "Update the zkvm_version table in this script to the SDK versions"
        echo "that ere-catalog resolves at the new rev, then update ERE_REV."
    } >&2
    exit 1
fi

zkvm_version "$ZKVM"
