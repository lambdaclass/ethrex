#!/usr/bin/env bash
#
# Prints the zkVM SDK version the stateless-validator guest is built against, for
# use in release asset names.
#
# The `v` prefix is part of the name: eth-act/ere-guests publishes
# `stateless-validator-<guest>-<zkvm>-v<version>.{elf,vk}` and republishes guest
# ELFs verbatim, so dropping it makes our assets not drop-in for that pipeline.
#
# The guests reach their SDK through `ere-platform-{zisk,sp1,openvm}`, so the ere
# tag in their manifests is not the SDK version. The table below instead mirrors
# what `ere-catalog` resolves at the pinned ere tag, and the check further down
# fails if the manifests have moved off that tag — a silent bump cannot mislabel
# an artifact.
#
# Usage: zkvm-version.sh <zisk|sp1|openvm>

set -euo pipefail

ERE_TAG=v0.17.0

# SDK versions resolved by ere-catalog at ERE_TAG.
zkvm_version() {
    case "$1" in
        zisk)   echo "v1.1.0-alpha" ;;
        sp1)    echo "v6.4.0" ;;
        openvm) echo "v2.1.0-preview" ;;
        *)      echo "unknown zkvm: $1" >&2; return 1 ;;
    esac
}

ZKVM="${1:?usage: zkvm-version.sh <zisk|sp1|openvm>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/crates/guest-program/stateless-validator/bin/$ZKVM/Cargo.toml"

[[ -f $MANIFEST ]] || { echo "no manifest at $MANIFEST" >&2; exit 1; }

# The table above is only valid for ERE_TAG; refuse to guess if it has moved.
if ! grep -q "tag = \"$ERE_TAG\"" "$MANIFEST"; then
    {
        echo "ere tag in $MANIFEST does not match ERE_TAG ($ERE_TAG)."
        echo "Update the zkvm_version table in this script to the SDK versions"
        echo "that ere-catalog resolves at the new tag, then update ERE_TAG."
    } >&2
    exit 1
fi

zkvm_version "$ZKVM"
