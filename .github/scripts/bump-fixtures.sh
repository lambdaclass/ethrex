#!/usr/bin/env bash
#
# Moves the EF fixture release pins to an execution-specs `tests@` release.
#
#   .github/scripts/bump-fixtures.sh v21.0.3 --zkevm v21.0.8   # a release and its zkEVM bundle
#   .github/scripts/bump-fixtures.sh --zkevm v21.0.8           # the zkEVM bundle, on the pinned release
#   .github/scripts/bump-fixtures.sh                           # the newest tests@ release
#
# It does the mechanical part of a bump:
#   - `tooling/ef_tests/.fixtures_url`, the bundle the ef-test runners download;
#   - `tag` in `.github/config/hive/pins.yaml`, the EELS simulator image hive runs;
#   - the release named in `docs/roadmaps/forks-roadmap.md` and
#     `docs/developers/l1/testing/hive.md`;
#   - with `--zkevm`, `tooling/ef_tests/.fixtures_url_zkevm`, `zkevm_release`
#     and `zkevm_base`, the last read from that zkEVM release's notes.
# Before editing anything it checks that the release has a fixture bundle, that
# execution-specs has published the image of every simulator we run for it, and
# that the zkEVM bundle is filled from it. A zkEVM bundle is filled from one
# `tests@` release, so a bump of the base needs `--zkevm` with a zkEVM release
# filled from the new base, and is refused without one: the two pins have to
# move in the same change for the pin check to pass.
#
# Not mechanical, and left to the pull request: new forks or networks the
# release adds to the fixtures, which the ef-test runners have to learn, and
# whatever the new tests catch in the client. consume-wirex keeps the image in
# `consume_wirex_tag`, which no release publishes yet, so its fixtures stay on
# the release that image was built from until that pin is moved by hand.
#
# Exit status: 0 when bumped or already current, 1 on any error. If
# BUMP_FIXTURES_SUMMARY is set, a Markdown summary is written there, for use as
# a pull request body.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SPECS=ethereum/execution-specs
PINS=.github/config/hive/pins.yaml
FIXTURES_URL=tooling/ef_tests/.fixtures_url
ZKEVM_URL=tooling/ef_tests/.fixtures_url_zkevm
ROADMAP=docs/roadmaps/forks-roadmap.md
HIVE_DOC=docs/developers/l1/testing/hive.md
# The EELS simulators that run from the release's own image. consume-wirex has
# its own pin, because no release ships its image yet.
SIMULATORS=(consume-engine consume-rlp consume-enginex execute-blobs)

die() { echo "bump-fixtures: $*" >&2; exit 1; }
pin() { sed -n "s/^$1:[[:space:]]*//p" "$PINS"; }
is_release() { [[ $1 =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; }

new=""
new_zkevm=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --zkevm)
            [[ $# -ge 2 ]] || die "--zkevm needs a tests-zkevm release, such as v21.0.8"
            new_zkevm=$2
            shift 2
            ;;
        -*) die "unknown option: $1" ;;
        *)
            [[ -z $new ]] || die "more than one release given"
            new=$1
            shift
            ;;
    esac
done

# The current state has to be consistent, or there is no single release to move from.
.github/scripts/check-fixture-pins.sh >/dev/null ||
    die "the pins disagree before the bump; fix that first (make check-fixture-pins)"
old=$(pin tag)

if [[ -z $new && -n $new_zkevm ]]; then
    # Only a zkEVM release was named: the base stays where it is.
    new=$old
elif [[ -z $new ]]; then
    new=$(gh release list --repo "$SPECS" --exclude-drafts --exclude-pre-releases --limit 200 \
        --json tagName --jq '.[].tagName' | sed -n 's/^tests@\(v[0-9][0-9.]*\)$/\1/p' | sort -V | tail -1)
    [[ -n $new ]] || die "could not list the tests@ releases of $SPECS"
fi
is_release "$new" || die "not a tests@ release version: $new (expected the form v21.0.3)"
[[ -z $new_zkevm ]] || is_release "$new_zkevm" || die "not a tests-zkevm release version: $new_zkevm"

old_zkevm=$(pin zkevm_release)
if [[ $new == "$old" && ${new_zkevm:-$old_zkevm} == "$old_zkevm" ]]; then
    echo "bump-fixtures: already at tests@$old and tests-zkevm@$old_zkevm"
    exit 0
fi
if [[ $(printf '%s\n%s\n' "$old" "$new" | sort -V | tail -1) != "$new" ]]; then
    die "tests@$new is older than the pinned tests@$old; refusing to downgrade"
fi

# The release has to ship the bundle the runners download.
bundle="https://github.com/$SPECS/releases/download/tests%40$new/fixtures.tar.gz"
[[ $new == "$old" ]] || curl -fsIL -o /dev/null "$bundle" || die "tests@$new has no fixtures.tar.gz at $bundle"

# Every simulator image has to exist at the release tag, or its hive job fails
# while building the simulator, before running a test.
missing=()
for simulator in "${SIMULATORS[@]}"; do
    # A release that is already pinned is not checked again.
    [[ $new != "$old" ]] || break
    repository="ethereum/execution-specs/hive/$simulator"
    token=$(curl -fsS "https://ghcr.io/token?scope=repository:$repository:pull" |
        sed -n 's/.*"token":"\([^"]*\)".*/\1/p') || true
    [[ -n $token ]] || die "could not get a registry token for ghcr.io/$repository"
    code=$(curl -s -o /dev/null -w '%{http_code}' \
        -H "Authorization: Bearer $token" \
        -H "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json" \
        "https://ghcr.io/v2/$repository/manifests/$new") || true
    [[ $code == 200 ]] || missing+=("ghcr.io/$repository:$new (HTTP $code)")
done
if [[ ${#missing[@]} -gt 0 ]]; then
    printf 'bump-fixtures: missing simulator image: %s\n' "${missing[@]}" >&2
    die "execution-specs has not published every simulator image for tests@$new"
fi

# The base of a zkEVM release is stated in its notes: "Is based on tests@vX.Y.Z".
old_zkevm_base=$(pin zkevm_base)
new_zkevm_base=$old_zkevm_base
if [[ -n $new_zkevm ]]; then
    notes=$(gh release view "tests-zkevm@$new_zkevm" --repo "$SPECS" --json body --jq .body) ||
        die "could not read the release notes of tests-zkevm@$new_zkevm"
    [[ $notes =~ [Bb]ased\ on[^A-Za-z0-9]*tests@(v[0-9]+\.[0-9]+\.[0-9]+) ]] ||
        die "the notes of tests-zkevm@$new_zkevm do not state the tests@ release it is based on"
    new_zkevm_base=${BASH_REMATCH[1]}
    if [[ $(printf '%s\n%s\n' "$old_zkevm" "$new_zkevm" | sort -V | tail -1) != "$new_zkevm" ]]; then
        die "tests-zkevm@$new_zkevm is older than the pinned tests-zkevm@$old_zkevm; refusing to downgrade"
    fi
    zkevm_bundle="https://github.com/$SPECS/releases/download/tests-zkevm%40$new_zkevm/fixtures_zkevm.tar.gz"
    curl -fsIL -o /dev/null "$zkevm_bundle" || die "tests-zkevm@$new_zkevm has no bundle at $zkevm_bundle"
fi
# Checked before any edit, so a refused bump leaves the tree as it was.
if [[ $new_zkevm_base != "$new" ]]; then
    if [[ -n $new_zkevm ]]; then
        die "tests-zkevm@$new_zkevm is filled from tests@$new_zkevm_base, not tests@$new"
    fi
    die "the pinned tests-zkevm@$old_zkevm is filled from tests@$old_zkevm_base: pass --zkevm with a zkEVM release filled from tests@$new"
fi

# From here on the tree is edited.
OLD="$old" NEW="$new" perl -pi -e 's/tests%40\Q$ENV{OLD}\E\/fixtures\.tar\.gz/tests%40$ENV{NEW}\/fixtures.tar.gz/' "$FIXTURES_URL" "$HIVE_DOC"
OLD="$old" NEW="$new" perl -pi -e 's/^tag: \Q$ENV{OLD}\E$/tag: $ENV{NEW}/' "$PINS"
OLD="$old" NEW="$new" perl -pi -e '
    if (/^- Fixtures: \[`tests\@/) { s/(tests(?:\@|%40))\Q$ENV{OLD}\E(?![0-9])/$1$ENV{NEW}/g }
    s/^(- EELS simulator image tag: `)\Q$ENV{OLD}\E(`)$/$1$ENV{NEW}$2/;
    if (/^- Status: /) { s/the \Q$ENV{OLD}\E bundle/the $ENV{NEW} bundle/ }
' "$ROADMAP"
if [[ -n $new_zkevm ]]; then
    OLD="$old_zkevm" NEW="$new_zkevm" perl -pi -e 's/tests-zkevm%40\Q$ENV{OLD}\E\//tests-zkevm%40$ENV{NEW}\//' "$ZKEVM_URL"
    OLD="$old_zkevm" NEW="$new_zkevm" perl -pi -e 's/^zkevm_release: \Q$ENV{OLD}\E$/zkevm_release: $ENV{NEW}/' "$PINS"
    OLD="$old_zkevm_base" NEW="$new_zkevm_base" perl -pi -e 's/^zkevm_base: \Q$ENV{OLD}\E$/zkevm_base: $ENV{NEW}/' "$PINS"
fi

row() {
    if [[ $2 == "$3" ]]; then echo "| $1 | \`$2\` | unchanged |"; else echo "| $1 | \`$2\` | \`$3\` |"; fi
}
summary=$(
    if [[ $new == "$old" ]]; then
        echo "Bump of the zkEVM fixture bundle from \`tests-zkevm@$old_zkevm\` to \`tests-zkevm@$new_zkevm\`."
    else
        echo "Bump of the EF fixture pins from \`tests@$old\` to \`tests@$new\`."
    fi
    echo
    echo "| | from | to |"
    echo "|---|---|---|"
    row "EF fixtures and hive image tag" "tests@$old" "tests@$new"
    row "zkEVM bundle" "tests-zkevm@$old_zkevm" "tests-zkevm@${new_zkevm:-$old_zkevm}"
    row "zkEVM bundle base" "tests@$old_zkevm_base" "tests@$new_zkevm_base"
    echo
    if [[ $new == "$old" ]]; then
        echo "Release notes: https://github.com/$SPECS/releases/tag/tests-zkevm%40$new_zkevm"
    else
        echo "Release notes: https://github.com/$SPECS/releases/tag/tests%40$new"
        echo
        echo "Simulator images checked at \`$new\`: ${SIMULATORS[*]}."
    fi
    echo
    echo "consume-wirex does not move with this bump: it runs the image pinned in \`consume_wirex_tag\`, with the fixtures that image was built from, until that pin is moved by hand."
    echo
    echo "**Before merging:** check the release notes for forks or networks the ef-test runners do not know yet, and review the Fixtures and Status lines in \`$ROADMAP\`, which this script only re-versions: the Status line says the suites are green on the new bundle, so reword it if this pull request's CI says otherwise."
)
echo "$summary"
if [[ -n ${BUMP_FIXTURES_SUMMARY:-} ]]; then
    echo "$summary" >"$BUMP_FIXTURES_SUMMARY"
fi

.github/scripts/check-fixture-pins.sh || die "the pins disagree after the bump, see above"
