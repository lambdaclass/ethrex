#!/usr/bin/env bash
# Download <url> to <dest>, tolerating the transient failures that routinely
# break CI on GitHub release and CDN downloads.
#
# Usage: download.sh <url> <dest>
#
# Every guard below maps to a failure we have actually hit:
#
#   --fail        Plain `curl -o` exits 0 on an HTTP error and writes the error
#                 body to <dest>. A 107-byte 503 page landed in
#                 amsterdam-tests.tar.gz and only surfaced two steps later as
#                 "gzip: stdin: not in gzip format", which reads like corrupt
#                 fixtures rather than a failed download.
#   --retry ...   github.com release downloads serve bursts of 503s and drop
#                 connections mid-transfer (curl exit 56); both are transient.
#                 --retry-all-errors is needed because plain --retry ignores
#                 4xx/connection errors.
#   outer loop    curl's own backoff caps out around 30s, but release-CDN
#                 outages have lasted minutes, so retry the whole transfer a few
#                 times with a longer pause between rounds.
#   .part + mv    A failed attempt must not leave a partial file behind: make
#                 treats any existing <dest> newer than its prerequisites as up
#                 to date, so a truncated archive would be handed to `tar` on
#                 every later run and each re-run would fail identically.
#   magic bytes   An error page served with a 200, or a response truncated by a
#                 proxy, still yields a non-gzip body. Cheaper to catch at the
#                 download than as a confusing `tar` error further downstream.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $(basename "$0") <url> <dest>" >&2
  exit 2
fi

url=$1
dest=$2
tmp="$dest.part"

ATTEMPTS=5
RETRY_DELAY=30

trap 'rm -f "$tmp"' EXIT

# Fetch into $tmp and sanity-check it. Returns non-zero for anything the caller
# should retry, leaving $tmp for the caller to discard.
fetch() {
  curl "$url" \
    --location \
    --fail \
    --show-error \
    --no-progress-meter \
    --connect-timeout 30 \
    --retry 5 \
    --retry-all-errors \
    --retry-connrefused \
    --output "$tmp" || return 1

  case "$dest" in
  *.tar.gz | *.tgz | *.gz)
    # \x1f\x8b is the gzip magic number.
    if [ "$(head -c 2 "$tmp" | od -An -tx1 | tr -d ' \n')" != "1f8b" ]; then
      echo "$url: response is not gzip data (got $(wc -c <"$tmp") bytes)" >&2
      return 1
    fi
    ;;
  esac
}

mkdir -p "$(dirname "$dest")"

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "Downloading $url -> $dest (attempt $attempt/$ATTEMPTS)"
  if fetch; then
    mv "$tmp" "$dest"
    exit 0
  fi
  rm -f "$tmp"
  if [ "$attempt" -lt "$ATTEMPTS" ]; then
    echo "Retrying in ${RETRY_DELAY}s..."
    sleep "$RETRY_DELAY"
  fi
done

echo "Failed to download $url after $ATTEMPTS attempts" >&2
exit 1
