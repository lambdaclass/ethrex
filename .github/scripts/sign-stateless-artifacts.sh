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
#   MINISIGN_PUBLIC_KEY  the matching public key
#   MINISIGN_PASSWORD    password for the private key (empty for a `-W` key)
#
# Optional environment:
#   TRUSTED_COMMENT_SUFFIX  appended to each signed trusted comment, e.g. the tag
#                           and commit. Signed, so it is a provenance claim.
#   COMMITTED_PUBLIC_KEY    path to the in-repo public key (default
#                           .github/minisign.pub). Must exist and must match
#                           MINISIGN_PUBLIC_KEY — see "Why the committed key is
#                           mandatory" below.
set -euo pipefail

ARTIFACT_DIR="${1:?usage: sign-stateless-artifacts.sh <artifact-dir>}"
COMMITTED_PUBLIC_KEY="${COMMITTED_PUBLIC_KEY:-.github/minisign.pub}"
TRUSTED_COMMENT_SUFFIX="${TRUSTED_COMMENT_SUFFIX:-}"

: "${MINISIGN_SECRET_KEY:?MINISIGN_SECRET_KEY is not set}"
: "${MINISIGN_PUBLIC_KEY:?MINISIGN_PUBLIC_KEY is not set}"
MINISIGN_PASSWORD="${MINISIGN_PASSWORD:-}"

if [ ! -d "$ARTIFACT_DIR" ]; then
  echo "error: artifact directory '$ARTIFACT_DIR' does not exist" >&2
  exit 1
fi

# ── Key material ──────────────────────────────────────────────────────────────
#
# Why the committed key is mandatory: a public key shipped inside the same
# release it authenticates proves nothing, because anyone able to replace the
# artifacts can replace the key beside them. The signature is only meaningful
# against a key published out-of-band, so the in-repo copy is the source of
# truth and the released copy is a convenience. Asserting they match is what
# stops a rotated or mistyped secret from producing a release full of signatures
# that verify against nothing anyone has.

if [ ! -f "$COMMITTED_PUBLIC_KEY" ]; then
  cat >&2 <<EOF
error: no committed public key at '$COMMITTED_PUBLIC_KEY'.

Signing is configured (MINISIGN_SECRET_KEY is set) but the public key is not in
the repository, so a consumer would have no out-of-band copy to verify against.
Commit the public key of the signing keypair to that path, then re-run.
EOF
  exit 1
fi

# minisign key files are a comment line followed by the base64 key. Compare the
# key itself so an differing comment line is not treated as a mismatch.
key_line() {
  grep -v '^untrusted comment:' "$1" | tr -d '[:space:]'
}

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
umask 077

printf '%s\n' "$MINISIGN_SECRET_KEY" > "$WORK_DIR/minisign.key"
printf '%s\n' "$MINISIGN_PUBLIC_KEY" > "$WORK_DIR/minisign.pub"

if [ "$(key_line "$WORK_DIR/minisign.pub")" != "$(key_line "$COMMITTED_PUBLIC_KEY")" ]; then
  cat >&2 <<EOF
error: MINISIGN_PUBLIC_KEY does not match '$COMMITTED_PUBLIC_KEY'.

The secret and the committed key are different keypairs, so the released
signatures would not verify against the key consumers have. Either update the
committed key (if the signing key was rotated on purpose) or fix the secret.
EOF
  exit 1
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

  # Verify what was just produced, against the committed key rather than the
  # secret's copy — this is the check a consumer will run, so running it here
  # means a broken keypair fails the release instead of shipping.
  #
  # Overlaps with the key-match check above by design: that one fails fast with a
  # precise diagnostic, this one is the last line of defence on the real release
  # path, where nothing else verifies the output.
  minisign -V -m "$file" -p "$COMMITTED_PUBLIC_KEY" -x "$file.minisig" > /dev/null

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
cp "$COMMITTED_PUBLIC_KEY" "$ARTIFACT_DIR/minisign/minisign.pub"

echo "Signed ${#artifacts[@]} artifact(s); public key written to $ARTIFACT_DIR/minisign/minisign.pub"
