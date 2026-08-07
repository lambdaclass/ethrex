#!/usr/bin/env bash
#
# Sign the stateless-validator release artifacts with minisign.
#
# The zkEVM guest handbook requires the ELF and verification key to be released
# as signed assets from a public, open-source CI pipeline:
# https://github.com/eth-act/zkevm-standards/blob/main/handbooks/guest-handbook.md
#
# Mechanism and secret names match eth-act/ere-guests' own release pipeline
# (`compile-and-release.yml`), so anyone already verifying ere-guests artifacts
# can verify ours with the same command.
#
# Usage: sign-stateless-artifacts.sh <artifact-dir>
#
# Required environment:
#   MINISIGN_SECRET_KEY  minisign private key, as stored in the repo secret
#   MINISIGN_PASSWORD    password for the private key (empty for a `-W` key)
#
# The public key is always *derived* from the secret key with `minisign -R`, so
# the key published with a release can never disagree with the key that signed
# it. Any other copy is treated as a claim to be checked against that, never as
# a substitute for it.
#
# Optional environment:
#   TRUSTED_COMMENT_SUFFIX  appended to each signed trusted comment, e.g. the tag
#                           and commit. Signed, so it is a provenance claim.
#   MINISIGN_PUBLIC_KEY     the public key as recorded in the repo secret. Not
#                           required — it is cross-checked against the derived
#                           key so a stale or mistyped secret is caught.
#   COMMITTED_PUBLIC_KEY    path to the in-repo public key (default
#                           .github/minisign.pub). The out-of-band trust anchor;
#                           when present it must match the derived key.
set -euo pipefail

ARTIFACT_DIR="${1:?usage: sign-stateless-artifacts.sh <artifact-dir>}"
COMMITTED_PUBLIC_KEY="${COMMITTED_PUBLIC_KEY:-.github/minisign.pub}"
TRUSTED_COMMENT_SUFFIX="${TRUSTED_COMMENT_SUFFIX:-}"

: "${MINISIGN_SECRET_KEY:?MINISIGN_SECRET_KEY is not set}"
MINISIGN_PASSWORD="${MINISIGN_PASSWORD:-}"

if [ ! -d "$ARTIFACT_DIR" ]; then
  echo "error: artifact directory '$ARTIFACT_DIR' does not exist" >&2
  exit 1
fi

# ── Key material ──────────────────────────────────────────────────────────────

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
umask 077

printf '%s\n' "$MINISIGN_SECRET_KEY" > "$WORK_DIR/minisign.key"

# Derive the public key from the secret key. `minisign -R` reads the password on
# stdin exactly as `-S` does, and works for passwordless (`-G -W`) keys too.
if ! printf '%s\n' "$MINISIGN_PASSWORD" \
  | minisign -R -s "$WORK_DIR/minisign.key" -p "$WORK_DIR/minisign.pub" > /dev/null 2>&1; then
  echo "error: could not derive the public key from MINISIGN_SECRET_KEY" >&2
  echo "       (wrong MINISIGN_PASSWORD, or the secret is not a minisign key)" >&2
  exit 1
fi

# minisign key files are a comment line followed by the base64 key. Compare the
# key itself, so a differing comment line is not treated as a mismatch.
key_line() {
  grep -v '^untrusted comment:' "$1" | tr -d '[:space:]'
}
DERIVED_KEY="$(key_line "$WORK_DIR/minisign.pub")"

# Cross-check any other recorded copy of the public key against the derived one.
# Neither is used *instead of* the derived key — the point is to catch a copy
# that has gone stale, which is a sign the key was rotated somewhere and not
# everywhere.
check_matches_derived() { # check_matches_derived <file> <what>
  local recorded
  recorded="$(key_line "$1")"
  if [ "$DERIVED_KEY" != "$recorded" ]; then
    cat >&2 <<EOF
error: the signing key does not match $2.

MINISIGN_SECRET_KEY derives to:
  $DERIVED_KEY
but $2 records:
  $recorded

They are different keypairs. Either the signing key was rotated without updating
$2, or the wrong value was recorded. Consumers verify against the recorded key,
so this release's signatures would not verify for them.
EOF
    exit 1
  fi
}

if [ -n "${MINISIGN_PUBLIC_KEY:-}" ]; then
  printf '%s\n' "$MINISIGN_PUBLIC_KEY" > "$WORK_DIR/secret-copy.pub"
  check_matches_derived "$WORK_DIR/secret-copy.pub" "the MINISIGN_PUBLIC_KEY secret"
  echo "Signing key matches the MINISIGN_PUBLIC_KEY secret"
fi

# A public key shipped inside the same release it authenticates proves nothing:
# anyone able to replace the artifacts can replace the key beside them, and a
# repo secret is no better — it is not something a downloader can consult. The
# signature is only meaningful against a key published out-of-band, which is why
# the in-repo copy is the trust anchor and everything else is a convenience.
#
# It is not required, because requiring it would fail the first release made
# after the signing secrets were configured. Once committed it is enforced.
PUBLIC_KEY_SOURCE="$WORK_DIR/minisign.pub"
if [ -f "$COMMITTED_PUBLIC_KEY" ]; then
  check_matches_derived "$COMMITTED_PUBLIC_KEY" "'$COMMITTED_PUBLIC_KEY'"
  echo "Signing key matches $COMMITTED_PUBLIC_KEY"
  PUBLIC_KEY_SOURCE="$COMMITTED_PUBLIC_KEY"
else
  # A GitHub warning annotation, so this is visible on the run summary rather
  # than buried in the log.
  echo "::warning::No committed public key at $COMMITTED_PUBLIC_KEY."
  cat >&2 <<EOF

The release will carry a public key derived from MINISIGN_SECRET_KEY, but a key
published only inside the release it authenticates gives consumers nothing to
check it against. Commit this file as '$COMMITTED_PUBLIC_KEY' to make the
signatures meaningful; once it is there, this script enforces the match.

$(cat "$WORK_DIR/minisign.pub")

EOF
fi

# ── Artifacts ─────────────────────────────────────────────────────────────────

artifacts=()
while IFS= read -r -d '' file; do
  artifacts+=("$file")
done < <(
  find "$ARTIFACT_DIR" -type f -name 'stateless-validator-ethrex-*' \
    \( -name '*.elf' -o -name '*.vk' \) -print0 | sort -z
)

# A hard failure, not a warning. The artifact names are built from
# zkvm-version.sh, so a version bump or a rename can move them out from under
# this glob — and an unsigned release that still looks green is exactly the
# failure this script exists to prevent. The same trap already caught the
# release download pattern once (`ethrex*` vs `*ethrex*`).
if [ "${#artifacts[@]}" -eq 0 ]; then
  echo "error: no stateless-validator .elf/.vk artifacts found under '$ARTIFACT_DIR'" >&2
  echo "       expected files named stateless-validator-ethrex-<zkvm>-<version>.{elf,vk}" >&2
  find "$ARTIFACT_DIR" -type f | sort >&2
  exit 1
fi

echo "Signing ${#artifacts[@]} artifact(s) under '$ARTIFACT_DIR':"

for file in "${artifacts[@]}"; do
  filename="$(basename "$file")"
  trusted_comment="$filename"
  if [ -n "$TRUSTED_COMMENT_SUFFIX" ]; then
    trusted_comment="$filename $TRUSTED_COMMENT_SUFFIX"
  fi

  # The filename leads the trusted comment, matching ere-guests, so a consumer
  # comparing it against the asset name still works.
  printf '%s\n' "$MINISIGN_PASSWORD" | minisign \
    -S \
    -m "$file" \
    -s "$WORK_DIR/minisign.key" \
    -x "$file.minisig" \
    -t "$trusted_comment" \
    > /dev/null

  # Verify what was just produced, against the committed key when there is one — this is the check a consumer will run, so running it here
  # means a broken keypair fails the release instead of shipping.
  #
  # Overlaps with the key-match check above by design: that one fails fast with a
  # precise diagnostic, this one is the last line of defence on the real release
  # path, where nothing else verifies the output.
  minisign -V -m "$file" -p "$PUBLIC_KEY_SOURCE" -x "$file.minisig" > /dev/null

  echo "  signed + verified: $filename"
done

# Publish the committed key alongside the artifacts. It carries no authority by
# itself (see above) — it is there so a consumer who already trusts this
# repository does not have to fetch it separately.
#
# One directory deep, matching every other downloaded artifact, so the release
# job's `./bin/**/*` glob covers it without depending on whether `**` matches
# zero path segments in the uploader's glob implementation.
mkdir -p "$ARTIFACT_DIR/minisign"
cp "$PUBLIC_KEY_SOURCE" "$ARTIFACT_DIR/minisign/minisign.pub"

echo "Signed ${#artifacts[@]} artifact(s); public key written to $ARTIFACT_DIR/minisign/minisign.pub"
