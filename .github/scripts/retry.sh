#!/usr/bin/env bash
# Retry a command with exponential backoff.
# Usage: source this file, then call `retry <command> [args...]`
# Retries up to 5 times with delays of 10s, 20s, 40s, 80s.

retry() {
  local retries=5
  local delay=10
  for i in $(seq 1 $retries); do
    if "$@"; then
      return 0
    fi
    if [ $i -lt $retries ]; then
      echo "Attempt $i/$retries failed. Retrying in ${delay}s..."
      sleep $delay
      delay=$((delay * 2))
    else
      echo "All $retries attempts failed."
    fi
  done
  return 1
}
