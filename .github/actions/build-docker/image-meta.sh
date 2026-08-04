#!/usr/bin/env bash
# Derives the build-time client-version channel (baked as VERGEN_GIT_BRANCH) and
# the image version (org.opencontainers.image.version label) from the build ref.
#
# The channel baked here is only the DEFAULT. The stability of a release is not
# known at build time -- an RC may be discarded and re-cut -- so a release-tag
# build bakes its RC suffix (e.g. "rc.1"), not "stable". When an RC is promoted,
# tag_latest.yaml stamps ETHREX_CHANNEL=stable onto the image config (the binary
# reads that at runtime, see cmd/ethrex/utils.rs::get_channel), so:
#   :18.0.0-rc.1  -> ethrex/v18.0.0-rc.1-<sha>     (candidate, honest)
#   :latest       -> ethrex/v18.0.0-stable-<sha>   (promoted, honest)
# Branch/PR builds keep the branch name and a dev-<sha> version.
#
# Inputs (env): REF_TYPE, REF_NAME, HEAD_REF, SHA.
# Output: channel=… / version=… appended to $GITHUB_OUTPUT (stdout if unset).
# Unit-tested by .github/workflows/pr_lint_gha.yaml.
set -euo pipefail

if [ "${REF_TYPE:-}" = "tag" ]; then
  ref="${REF_NAME#v}"     # strip a leading v -> 18.0.0-rc.1
  version="${ref%%-*}"    # bare semver -> 18.0.0
  if [ "$ref" != "$version" ]; then
    channel="${ref#*-}"   # the pre-release suffix, e.g. rc.1 (build-time default)
  else
    channel="release"     # a final vX.Y.Z tag with no suffix
  fi
else
  channel="${HEAD_REF:-${REF_NAME:-}}"
  version="dev-${SHA:-unknown}"
fi

{
  echo "channel=$channel"
  echo "version=$version"
} >> "${GITHUB_OUTPUT:-/dev/stdout}"
