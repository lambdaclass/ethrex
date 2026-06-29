#!/usr/bin/env bash
# Derives the client-version channel (baked into the binary as VERGEN_GIT_BRANCH)
# and the image version (org.opencontainers.image.version label) from the build ref.
#
# Release (tag) builds use a clean, release-appropriate identity -- channel
# "stable" and the bare semver -- instead of the git tag. Promotion retags the
# tested RC image to :X.Y.Z / :latest, so whatever a tag build bakes in is what
# the stable image reports; echoing the RC tag here surfaced on :latest as e.g.
# "ethrex/v18.0.0-v18.0.0-rc.1-<sha>". Branch/PR builds keep the branch name and
# a dev-<sha> version.
#
# Inputs (env): REF_TYPE, REF_NAME, HEAD_REF, SHA.
# Output: channel=… / version=… appended to $GITHUB_OUTPUT (stdout if unset).
# Unit-tested by .github/workflows/pr_lint_gha.yaml.
set -euo pipefail

if [ "${REF_TYPE:-}" = "tag" ]; then
  channel="stable"
  version="${REF_NAME#v}"   # strip a leading v
  version="${version%%-*}"  # strip any pre-release suffix (-rc.N, -alpha, …)
else
  channel="${HEAD_REF:-${REF_NAME:-}}"
  version="dev-${SHA:-unknown}"
fi

{
  echo "channel=$channel"
  echo "version=$version"
} >> "${GITHUB_OUTPUT:-/dev/stdout}"
