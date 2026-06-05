#!/usr/bin/env bash
# Guards the [patch."https://github.com/lambdaclass/ethrex"] section in Cargo.toml.
#
# ethrex-tooling (ethrex-monitor / ethrex-repl) pulls ethrex crates from the
# ethrex git source. The [patch] section redirects those to the local workspace
# so the build uses a single copy of each crate. If a new workspace crate that
# tooling depends on is missing from that section, it silently resolves to a
# fetched git copy instead, producing a duplicate / divergent build.
#
# This fails CI when any ethrex crate still resolves to the git source, listing
# exactly which [patch] entries are missing.
set -euo pipefail

leaked=$(cargo metadata --format-version 1 \
  | jq -r '.packages[]
      | select(.source != null
          and (.source | test("git\\+https://github\\.com/lambdaclass/ethrex(\\?|#|$)")))
      | .name' \
  | sort -u)

if [ -n "$leaked" ]; then
  echo "ERROR: these ethrex crates resolve to the git source instead of the local workspace:"
  echo "$leaked" | sed 's/^/  - /'
  echo
  echo "Add a matching entry for each to the"
  echo "[patch.\"https://github.com/lambdaclass/ethrex\"] section in Cargo.toml."
  echo "See docs/updating-ethrex-tooling.md."
  exit 1
fi

echo "OK: all ethrex crates resolve to the local workspace; [patch] section is complete."
