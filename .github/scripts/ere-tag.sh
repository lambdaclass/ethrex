#!/usr/bin/env bash
#
# Prints the ere release tag the stateless-validator guests are pinned to, and
# fails if the pins disagree with each other.
#
# The guests are built by ere's compiler image and then run under ere's server
# image, so the ere the ELF is compiled with and the ere it executes under have
# to be the same release. Those used to be stated independently — the manifests
# pinned a bare `rev`, the release workflow hardcoded an `ERE_TAG` — and they
# silently diverged: the manifests sat on a commit that predated the tag the
# images were pulled at. Nothing failed, because nothing compared them.
#
# So the manifests are the single source of truth and everything else derives
# from here. Adding a new pin means adding it to MANIFESTS below.
#
# Usage: ere-tag.sh            -> v0.17.0   (git tag, as written in Cargo.toml)
#        ere-tag.sh --docker   -> 0.17.0    (image tag; ghcr has no `v` prefix)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BASE="$ROOT/crates/guest-program/stateless-validator"

MANIFESTS=(
    "$BASE/Cargo.toml"
    "$BASE/bin/sp1/Cargo.toml"
    "$BASE/bin/zisk/Cargo.toml"
    "$BASE/bin/openvm/Cargo.toml"
)

tag=""
for manifest in "${MANIFESTS[@]}"; do
    [[ -f $manifest ]] || { echo "no manifest at $manifest" >&2; exit 1; }

    # Every ere dependency line in the file, so a manifest pinning two ere
    # crates at different tags is caught rather than read from the first match.
    # Kept to newline-separated text rather than an array: `mapfile` is bash 4+
    # and macOS still ships bash 3.2, where this would fail only for local runs.
    # `|| true`: with `pipefail` a no-match grep would abort the script here,
    # which is the case this check exists to report. Let it yield empty instead
    # so the diagnostic below is what the caller actually sees.
    found=$(
        grep -oE 'git = "https://github\.com/eth-act/ere"[^}]*' "$manifest" \
            | grep -oE 'tag = "[^"]+"' | grep -oE '"[^"]+"' | tr -d '"' | sort -u \
            || true
    )
    count=$(printf '%s' "$found" | grep -c . || true)

    if [[ $count -eq 0 ]]; then
        echo "$manifest pins ere without a 'tag = \"vX.Y.Z\"'." >&2
        echo "Pin ere by tag so the compiler and server images can be derived from it." >&2
        exit 1
    fi
    if [[ $count -gt 1 ]]; then
        echo "$manifest pins ere at more than one tag: $(echo "$found" | tr '\n' ' ')" >&2
        exit 1
    fi

    if [[ -z $tag ]]; then
        tag="$found"
    elif [[ $tag != "$found" ]]; then
        echo "ere pins disagree: expected $tag but $manifest pins $found." >&2
        echo "All stateless-validator manifests must build against one ere release." >&2
        exit 1
    fi
done

if [[ ${1:-} == "--docker" ]]; then
    echo "${tag#v}"
else
    echo "$tag"
fi
