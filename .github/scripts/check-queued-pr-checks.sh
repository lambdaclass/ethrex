#!/usr/bin/env bash
#
# Assert that every check run matching one of the given name prefixes concluded
# successfully on the head commit of each pull request in the current merge group.
#
# Usage: check-queued-pr-checks.sh "Hive - " "Assertoor - " ...
#
# Why this exists: the expensive suites (hive, assertoor) are deliberately not
# re-run inside the merge queue, so the required gate job has nothing of its own
# to inspect there. Skipping the gate instead is not a safe substitute — GitHub
# counts a skipped check run as satisfying a required status check, so the queue
# would merge a pull request whose suites were red or still running. Reading the
# pull request's own results here keeps the queue cheap while making the
# requirement real, and because it runs at merge time it also catches a result
# that turned red (or was re-triggered) after the pull request was queued.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <check-name-prefix> [<check-name-prefix> ...]" >&2
  exit 2
fi

: "${GITHUB_REPOSITORY:?}"
: "${GITHUB_EVENT_PATH:?}"

base_sha=$(jq -r '.merge_group.base_sha' "$GITHUB_EVENT_PATH")
head_sha=$(jq -r '.merge_group.head_sha' "$GITHUB_EVENT_PATH")

if [[ -z "$base_sha" || "$base_sha" == "null" || -z "$head_sha" || "$head_sha" == "null" ]]; then
  echo "No merge_group payload found; this script only runs on merge_group events." >&2
  exit 2
fi

# One squashed commit per queued pull request, each titled "... (#1234)". Read
# them over the API rather than from a checkout so the job needs no clone.
mapfile -t pr_numbers < <(
  gh api "repos/${GITHUB_REPOSITORY}/compare/${base_sha}...${head_sha}" \
    --jq '.commits[].commit.message | split("\n")[0]' |
    grep -oE '\(#[0-9]+\)$' |
    tr -d '(#)' |
    sort -u
)

if [[ ${#pr_numbers[@]} -eq 0 ]]; then
  echo "Could not identify any pull request in merge group ${base_sha}..${head_sha}." >&2
  echo "Refusing to pass: a gate that cannot find what to verify must not report success." >&2
  exit 1
fi

echo "Merge group covers pull request(s): ${pr_numbers[*]}"

failed=0
for pr in "${pr_numbers[@]}"; do
  pr_head=$(gh api "repos/${GITHUB_REPOSITORY}/pulls/${pr}" --jq '.head.sha')
  echo "::group::PR #${pr} (head ${pr_head})"

  # One row per check name, tab separated, keeping only the most recently started
  # run of that name. A single commit can carry several check suites (a re-trigger
  # creates a new suite rather than updating the old one), and without this a
  # superseded red run would keep blocking a head that is now green.
  all_checks=$(
    gh api --paginate --slurp \
      "repos/${GITHUB_REPOSITORY}/commits/${pr_head}/check-runs?per_page=100" |
      jq -r '[.[].check_runs[]]
             | group_by(.name)
             | map(max_by(.started_at // ""))
             | .[]
             | [.name, .status, (.conclusion // "")]
             | @tsv'
  )

  matched=0
  while IFS=$'\t' read -r name status conclusion; do
    [[ -z "$name" ]] && continue
    for prefix in "$@"; do
      if [[ "$name" == "$prefix"* ]]; then
        matched=$((matched + 1))
        if [[ "$status" != "completed" ]]; then
          echo "PENDING  ${name} (status=${status}) is still running, so it cannot be merged yet"
          failed=1
        elif [[ "$conclusion" != "success" && "$conclusion" != "skipped" && "$conclusion" != "neutral" ]]; then
          echo "FAILED   ${name} (conclusion=${conclusion})"
          failed=1
        else
          echo "ok       ${name} (${conclusion})"
        fi
        break
      fi
    done
  done <<<"$all_checks"

  if [[ $matched -eq 0 ]]; then
    echo "No check runs matching [$*] on PR #${pr}."
    echo "Refusing to pass: the suites this gate exists to enforce never reported."
    failed=1
  fi
  echo "::endgroup::"
done

if [[ $failed -ne 0 ]]; then
  echo "Required suites did not pass on the queued pull request head(s)." >&2
  exit 1
fi

echo "All required suites passed on every queued pull request head."
