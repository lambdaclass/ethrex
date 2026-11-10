#!/usr/bin/env bash
#
# Bumps the stateless-validator guests to an ere release. The bump-ere workflow
# runs it on a schedule, and it can be run by hand from anywhere in the repo:
#
#   .github/scripts/bump-ere.sh            # the newest ere release
#   .github/scripts/bump-ere.sh v0.18.1    # a specific release
#
# It does every mechanical part of a bump:
#   - the ere tag in the four stateless-validator manifests;
#   - ERE_TAG and the SDK table in zkvm-version.sh, read from ere's own Cargo.lock
#     at the tag with the same rule ere-catalog uses;
#   - versioned release-asset names, `stateless-validator-ethrex-<zkvm>-<sdk>`;
#   - the four lockfiles.
#
# Not mechanical, and left to review: openvm `[patch]` tables and toolchain
# overrides, which have broken past bumps, and whether the guests still build for
# their zkVM targets, which PR CI does not do (dispatch tag_release.yaml).
#
# Exit status: 0 when bumped or already current, 1 on any error. If
# BUMP_ERE_SUMMARY is set, a Markdown summary of the bump is written there, for
# use as a PR body.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION_SCRIPT=.github/scripts/zkvm-version.sh
BASE=crates/guest-program/stateless-validator
WORKSPACES=("$BASE" "$BASE/bin/sp1" "$BASE/bin/zisk" "$BASE/bin/openvm")

die() { echo "bump-ere: $*" >&2; exit 1; }

# The current state, validated: zkvm-version.sh refuses to answer if the
# manifests disagree with each other or with its own ERE_TAG.
old_tag=$("$VERSION_SCRIPT" --ere-tag)
old_sp1=$("$VERSION_SCRIPT" sp1)
old_zisk=$("$VERSION_SCRIPT" zisk)
old_openvm=$("$VERSION_SCRIPT" openvm)

new_tag=${1:-}
if [[ -z $new_tag ]]; then
    new_tag=$(gh release list --repo eth-act/ere --exclude-drafts --exclude-pre-releases \
        --limit 100 --json tagName --jq '.[].tagName' | sort -V | tail -1)
    [[ -n $new_tag ]] || die "could not list eth-act/ere releases"
fi
[[ $new_tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "not an ere release tag: $new_tag"

if [[ $new_tag == "$old_tag" ]]; then
    echo "bump-ere: already at $old_tag"
    exit 0
fi
# ere keeps more than one release line alive, and the one GitHub marks "latest"
# is not always the highest, so only ever move forward.
if [[ $(printf '%s\n%s\n' "$old_tag" "$new_tag" | sort -V | tail -1) != "$new_tag" ]]; then
    die "$new_tag is older than the pinned $old_tag; refusing to downgrade"
fi

# SDK versions, derived from ere's Cargo.lock exactly as ere-catalog's
# resolve_pkg_version does: a git dependency reports its `?tag=` value (or the
# first 7 hex digits of its rev), a registry dependency reports `v<version>`.
lock=$(mktemp)
trap 'rm -f "$lock"' EXIT
curl -fsSL "https://raw.githubusercontent.com/eth-act/ere/$new_tag/Cargo.lock" -o "$lock" ||
    die "could not fetch ere's Cargo.lock at $new_tag"

sdk_version() { # <owner crate> <dependency>, as ere-catalog's detect_dep_version
    local entry version source repr
    # A lockfile can hold one package name more than once (ere carries both an
    # upstream and a forked openvm), so resolve the dependency through the crate
    # that owns it. Its entry reads "dep", "dep version" or "dep version (source)",
    # depending on how much Cargo needs to disambiguate.
    entry=$(awk -v owner="$1" -v dep="$2" '
        function flush() {
            if (name == "") return
            n++; pname[n] = name; pver[n] = ver; psrc[n] = src
            if (name == owner) odeps = deps
            name = ver = src = deps = ""; indeps = 0
        }
        /^\[\[package\]\]/        { flush(); next }
        indeps && /^\]/           { indeps = 0; next }
        indeps                    { gsub(/^[ \t]*"|",?[ \t]*$/, ""); deps = deps "\n" $0; next }
        /^name = /                { name = $3; gsub(/"/, "", name); next }
        /^version = /             { ver = $3; gsub(/"/, "", ver); next }
        /^source = /              { src = $3; gsub(/"/, "", src); next }
        /^dependencies = \[/      { indeps = 1; next }
        END {
            flush()
            m = split(odeps, lines, "\n")
            for (i = 1; i <= m; i++)
                if (lines[i] == dep || index(lines[i], dep " ") == 1) { line = lines[i]; hits++ }
            if (hits != 1) exit 3
            rest = substr(line, length(dep) + 2)
            if (rest != "") {
                sp = index(rest, " ")
                if (sp == 0) want_ver = rest
                else { want_ver = substr(rest, 1, sp - 1); want_src = substr(rest, sp + 2); sub(/\)$/, "", want_src) }
            }
            for (i = 1; i <= n; i++) {
                if (pname[i] != dep) continue
                if (want_ver != "" && pver[i] != want_ver) continue
                if (want_src != "" && index(psrc[i], want_src) != 1) continue
                count++; out = pver[i] "\t" psrc[i]
            }
            if (count != 1) exit 3
            print out
        }' "$lock") || die "could not resolve $1's '$2' dependency in ere's Cargo.lock at $new_tag"
    version=${entry%%$'\t'*}
    source=${entry#*$'\t'}
    if [[ $source == git+* ]]; then
        repr=${source#git+}
        repr=${repr%%#*}
        if [[ $repr == *'?tag='* ]]; then echo "${repr##*\?tag=}"; else echo "${source##*#}" | cut -c1-7; fi
    else
        echo "v$version"
    fi
}
new_sp1=$(sdk_version ere-verifier-sp1 sp1-verifier)
new_zisk=$(sdk_version ere-verifier-zisk zisk-verifier)
new_openvm=$(sdk_version ere-platform-openvm openvm)

# From here on the tree is edited.
for workspace in "${WORKSPACES[@]}"; do
    OLD="$old_tag" NEW="$new_tag" perl -pi -e 's/(eth-act\/ere", tag = ")\Q$ENV{OLD}\E"/$1$ENV{NEW}"/g' "$workspace/Cargo.toml"
done

OLD_TAG="$old_tag" NEW_TAG="$new_tag" \
    OLD_SP1="$old_sp1" NEW_SP1="$new_sp1" \
    OLD_ZISK="$old_zisk" NEW_ZISK="$new_zisk" \
    OLD_OPENVM="$old_openvm" NEW_OPENVM="$new_openvm" \
    perl -pi -e '
        s/^ERE_TAG=\Q$ENV{OLD_TAG}\E$/ERE_TAG=$ENV{NEW_TAG}/;
        s/^(\s+zisk\)\s+echo ")\Q$ENV{OLD_ZISK}\E"/$1$ENV{NEW_ZISK}"/;
        s/^(\s+sp1\)\s+echo ")\Q$ENV{OLD_SP1}\E"/$1$ENV{NEW_SP1}"/;
        s/^(\s+openvm\)\s+echo ")\Q$ENV{OLD_OPENVM}\E"/$1$ENV{NEW_OPENVM}"/;
        s/(-> )\Q$ENV{OLD_SP1}\E(\s+\(SDK version\))/$1$ENV{NEW_SP1}$2/;
        s/(-> )\Q$ENV{OLD_TAG}\E(\s+\(git tag\))/$1$ENV{NEW_TAG}$2/;
        (my $old_image = $ENV{OLD_TAG}) =~ s/^v//;
        (my $new_image = $ENV{NEW_TAG}) =~ s/^v//;
        s/(-> )\Q$old_image\E(\s+\(image tag)/$1$new_image$2/;
    ' "$VERSION_SCRIPT"

rename_assets() {
    local old_name="stateless-validator-ethrex-$1-$2" new_name="stateless-validator-ethrex-$1-$3" file
    [[ $old_name == "$new_name" ]] && return 0
    while IFS= read -r file; do
        OLD="$old_name" NEW="$new_name" perl -pi -e 's/\Q$ENV{OLD}\E/$ENV{NEW}/g' "$file"
    done < <(git grep -lF "$old_name" -- ':!*Cargo.lock' || true)
}
rename_assets sp1 "$old_sp1" "$new_sp1"
rename_assets zisk "$old_zisk" "$new_zisk"
rename_assets openvm "$old_openvm" "$new_openvm"

# Re-resolve each lockfile. `cargo metadata` updates only the entries the manifest
# change touches, so unrelated dependencies do not move.
for workspace in "${WORKSPACES[@]}"; do
    (cd "$workspace" && cargo metadata --format-version 1 >/dev/null) ||
        die "could not resolve $workspace after the bump"
done

# Checks that would otherwise only surface after merge, when the guests build.
[[ $("$VERSION_SCRIPT" --ere-tag) == "$new_tag" ]] || die "manifests do not agree on $new_tag after the bump"
for workspace in "${WORKSPACES[@]}"; do
    if grep -qE 'eth-act/ere\?tag=' "$workspace/Cargo.lock" &&
        grep -oE 'eth-act/ere\?tag=v[0-9.]+' "$workspace/Cargo.lock" | grep -vqxF "eth-act/ere?tag=$new_tag"; then
        die "$workspace/Cargo.lock still resolves an ere source other than $new_tag"
    fi
done

row() {
    if [[ $2 == "$3" ]]; then echo "| $1 | \`$2\` | unchanged |"; else echo "| $1 | \`$2\` | \`$3\` |"; fi
}
summary=$(
    echo "Automated bump of the stateless-validator guests from ere \`$old_tag\` to \`$new_tag\`."
    echo
    echo "| | from | to |"
    echo "|---|---|---|"
    row ere "$old_tag" "$new_tag"
    row SP1 "$old_sp1" "$new_sp1"
    row ZisK "$old_zisk" "$new_zisk"
    row OpenVM "$old_openvm" "$new_openvm"
    echo
    echo "Release notes: https://github.com/eth-act/ere/compare/$old_tag...$new_tag"
    echo
    echo "**Before merging:** PR CI does not build the guest ELFs. This workflow dispatches \`tag_release.yaml\` on the branch, which builds all three guests and dry-runs the release assets without publishing; check that run. Also review any openvm \`[patch]\` tables and toolchain overrides, which past bumps have broken and this script does not touch."
)
echo "$summary"
if [[ -n ${BUMP_ERE_SUMMARY:-} ]]; then
    echo "$summary" >"$BUMP_ERE_SUMMARY"
fi
