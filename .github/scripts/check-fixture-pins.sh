#!/usr/bin/env bash
#
# Fails when the places that name the EF fixture release disagree.
#
# One execution-specs `tests@` release is named in several files: the bundle
# the ef-test runners download, the EELS simulator image hive runs, the base
# the zkEVM bundle is filled from, and two documents. They grade the same
# client, so a bump that misses one of them either fails every fixture the two
# releases disagree on or documents a release that is not the one being run.
#
# The base of the zkEVM bundle is only stated in that release's notes, so it is
# taken from `zkevm_base` in the pin file, not derived from the bundle's URL.
#
#   .github/scripts/check-fixture-pins.sh          # check this repository
#   .github/scripts/check-fixture-pins.sh <root>   # check another tree
#
# Prints the release on success. Exit status: 0 when every pin agrees, 1
# otherwise, with one line per disagreement.

set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
PINS="$ROOT/.github/config/hive/pins.yaml"
FIXTURES_URL="$ROOT/tooling/ef_tests/.fixtures_url"
ZKEVM_URL="$ROOT/tooling/ef_tests/.fixtures_url_zkevm"
ROADMAP="$ROOT/docs/roadmaps/forks-roadmap.md"
HIVE_DOC="$ROOT/docs/developers/l1/testing/hive.md"

status=0
mismatch() { echo "check-fixture-pins: $*" >&2; status=1; }

for file in "$PINS" "$FIXTURES_URL" "$ZKEVM_URL" "$ROADMAP" "$HIVE_DOC"; do
    [[ -f $file ]] || { echo "check-fixture-pins: no file at $file" >&2; exit 1; }
done

pin() { sed -n "s/^$1:[[:space:]]*//p" "$PINS"; }

# The scripts and the Makefile read a pin as the text after `key: `, so a pin
# line has to be exactly that. A trailing space or comment would become part of
# a hive flag.
while IFS= read -r line; do
    mismatch "${PINS#"$ROOT"/} has a pin that is not a bare \`key: value\` line: '$line'"
done < <(grep -E '^[A-Za-z_]+:' "$PINS" | grep -vE '^[a-z_]+: [^[:space:]#]+$' || true)
# A key that appears twice would be read as a value of two lines.
while IFS= read -r key; do
    mismatch "${PINS#"$ROOT"/} has more than one \`$key\` line"
done < <(grep -oE '^[a-z_]+:' "$PINS" | sort | uniq -d)

tag=$(pin tag)
if [[ ! $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "check-fixture-pins: \`tag\` in ${PINS#"$ROOT"/} is '$tag', not a release such as v21.0.2" >&2
    exit 1
fi

# The bundle the blockchain, state and engine ef-test runners download.
url_release=$(sed -n 's|^https://github.com/ethereum/execution-specs/releases/download/tests%40\(v[0-9][0-9.]*\)/fixtures\.tar\.gz$|\1|p' "$FIXTURES_URL")
if [[ -z $url_release ]]; then
    mismatch "${FIXTURES_URL#"$ROOT"/} is not an execution-specs tests@ release URL"
elif [[ $url_release != "$tag" ]]; then
    mismatch "${FIXTURES_URL#"$ROOT"/} is at tests@$url_release but the hive image tag is $tag"
fi

# The release the zkEVM bundle is filled from.
zkevm_base=$(pin zkevm_base)
if [[ $zkevm_base != "$tag" ]]; then
    mismatch "\`zkevm_base\` is '$zkevm_base' but the hive image tag is $tag: the zkEVM bundle and the base bundle describe different chains"
fi

if ! grep -qxE 'https://github\.com/ethereum/execution-specs/releases/download/tests-zkevm%40v[0-9]+\.[0-9]+\.[0-9]+/fixtures_zkevm\.tar\.gz' "$ZKEVM_URL"; then
    mismatch "${ZKEVM_URL#"$ROOT"/} is not an execution-specs tests-zkevm@ release URL"
fi

# The forks roadmap states the fixture release and the image tag.
roadmap_release=$(sed -n 's|^- Fixtures: \[`tests@\(v[0-9][0-9.]*\)`\].*|\1|p' "$ROADMAP")
if [[ $roadmap_release != "$tag" ]]; then
    mismatch "${ROADMAP#"$ROOT"/} lists fixtures 'tests@$roadmap_release', not tests@$tag"
fi
roadmap_tag=$(sed -n 's|^- EELS simulator image tag: `\(.*\)`$|\1|p' "$ROADMAP")
if [[ $roadmap_tag != "$tag" ]]; then
    mismatch "${ROADMAP#"$ROOT"/} lists image tag '$roadmap_tag', not $tag"
fi

# The hive guide shows the content of `.fixtures_url`. No URL there at all is a
# mismatch too, or the guide could lose it without this check noticing.
doc_releases=$(grep -o 'releases/download/tests%40v[0-9][0-9.]*/fixtures\.tar\.gz' "$HIVE_DOC" | sed 's|.*tests%40\(v[0-9][0-9.]*\)/.*|\1|' | sort -u || true)
if [[ -z $doc_releases ]]; then
    mismatch "${HIVE_DOC#"$ROOT"/} shows no tests@ fixtures URL"
fi
while IFS= read -r doc_release; do
    if [[ -n $doc_release && $doc_release != "$tag" ]]; then
        mismatch "${HIVE_DOC#"$ROOT"/} shows a tests@$doc_release fixtures URL, not tests@$tag"
    fi
done <<<"$doc_releases"

if [[ $status -eq 0 ]]; then
    echo "fixture pins agree: tests@$tag"
fi
exit "$status"
