#!/usr/bin/env bash
#
# Prints the zkVM SDK version the stateless-validator guest is built against, for
# use in release asset names, or the ere release tag the guests are pinned to.
#
# The `v` prefix is part of the name: eth-act/ere-guests publishes
# `stateless-validator-<guest>-<zkvm>-v<version>.{elf,vk}` and republishes guest
# ELFs verbatim, so dropping it makes our assets not drop-in for that pipeline.
#
# The guests reach their SDK through `ere-platform-{zisk,sp1,openvm}`, so the ere
# tag in their manifests is not the SDK version. The table below instead mirrors
# what `ere-catalog` resolves at ERE_TAG, and every run first checks the manifests
# still pin that tag — a silent bump cannot mislabel an artifact.
#
# The same check is why the release workflow asks for the tag here rather than
# repeating it: the guests are compiled by ere's compiler image and then executed
# under ere's server image, so both have to be the release the manifests pin, and
# a second copy of the version is a second thing to keep in sync. A bare `rev` is
# rejected for the same reason — it cannot be matched against an image tag.
#
# Usage: zkvm-version.sh <zisk|sp1|openvm>   -> v6.4.0    (SDK version)
#        zkvm-version.sh --ere-tag           -> v0.17.0   (git tag)
#        zkvm-version.sh --ere-tag --docker  -> 0.17.0    (image tag, no `v`)

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

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BASE="$ROOT/crates/guest-program/stateless-validator"

# The parent is checked alongside the bins because it pins `ere-platform-core`
# while each bin pins its own `ere-platform-<zkvm>`. If they disagree, one guest
# links two ere versions, and checking only the bin being asked about would let
# that through.
MANIFESTS=(
    "$BASE/Cargo.toml"
    "$BASE/bin/sp1/Cargo.toml"
    "$BASE/bin/zisk/Cargo.toml"
    "$BASE/bin/openvm/Cargo.toml"
)

# The table above is only valid for ERE_TAG; refuse to guess if anything moved.
for manifest in "${MANIFESTS[@]}"; do
    [[ -f $manifest ]] || { echo "no manifest at $manifest" >&2; exit 1; }
    grep -q "tag = \"$ERE_TAG\"" "$manifest" && continue
    {
        echo "ere tag in $manifest does not match ERE_TAG ($ERE_TAG)."
        echo "Every stateless-validator manifest must pin the same ere release,"
        echo "by tag rather than rev. If ere was bumped, update the zkvm_version"
        echo "table in this script to the SDK versions that ere-catalog resolves"
        echo "at the new tag, then update ERE_TAG."
    } >&2
    exit 1
done

if [[ ${1:-} == "--ere-tag" ]]; then
    if [[ ${2:-} == "--docker" ]]; then echo "${ERE_TAG#v}"; else echo "$ERE_TAG"; fi
    exit 0
fi

zkvm_version "${1:?usage: zkvm-version.sh <zisk|sp1|openvm>|--ere-tag [--docker]}"
