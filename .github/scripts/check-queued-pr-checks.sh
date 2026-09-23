#!/usr/bin/env bash
#
# Assert that this workflow's required integration gate was genuinely green on
# the head of every pull request in the current merge group.
#
# Usage: check-queued-pr-checks.sh "<gate job name>"
#
# Why this exists: the expensive suites (hive, assertoor, the L2 integration
# tests) are deliberately not re-run inside the merge queue, so the required
# gate job has nothing of its own to inspect there. Skipping the gate instead is
# not a safe substitute — GitHub counts a skipped check run as satisfying a
# required status check, so a gate that skips is a gate that always passes, and
# the queue would merge a pull request whose suites were red or still running.
# GitHub also decides queue eligibility when the pull request is enqueued and
# never re-evaluates it afterwards, so a result that turns red later cannot
# evict it.
#
# What it reads: the latest `pull_request` run of *this* workflow on each queued
# pull request's head, and within that run the verdict of this same gate job.
# Reading the gate's own verdict rather than matching suite check-run names
# keeps one definition of what is required — the workflow's own
# `Check if any job failed` step — and cannot be confused by an unrelated
# workflow that happens to name a job the same way.
#
# How the three outcomes are read:
#   - `success` passes, obviously.
#   - `skipped` passes. That is what the gate looks like on a pull request whose
#     changes did not require these suites at all, for example the L2 gate on an
#     L1-only pull request. It is not the same as the suites being skipped for
#     the wrong reason: when a dependency such as the docker build fails, the
#     suites skip but the gate itself runs and fails.
#   - A run that is not yet `completed` fails. That is the case this exists for:
#     re-running a suite bumps the run attempt, so a re-run in flight blocks the
#     merge group instead of being bypassed.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <gate-job-name>" >&2
  exit 2
fi

gate_name=$1

: "${GITHUB_REPOSITORY:?}"
: "${GITHUB_EVENT_PATH:?}"
: "${GITHUB_WORKFLOW_REF:?}"

# "owner/repo/.github/workflows/pr-main_l1.yaml@refs/heads/..." — the middle is
# what the runs API reports as `.path`. Derived from the environment rather than
# passed in so it cannot drift if the workflow file is renamed.
workflow_path=${GITHUB_WORKFLOW_REF#"${GITHUB_REPOSITORY}/"}
workflow_path=${workflow_path%%@*}

base_sha=$(jq -r '.merge_group.base_sha' "$GITHUB_EVENT_PATH")
head_sha=$(jq -r '.merge_group.head_sha' "$GITHUB_EVENT_PATH")

if [[ -z "$base_sha" || "$base_sha" == "null" || -z "$head_sha" || "$head_sha" == "null" ]]; then
  echo "No merge_group payload found; this script only runs on merge_group events." >&2
  exit 2
fi

# One squashed commit per queued pull request, each titled "... (#1234)". Read
# them over the API rather than from a checkout so the job needs no deep clone,
# and from the commit subjects rather than the queue branch name because a
# batched group's ref names only its last pull request.
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
echo "Gate: '${gate_name}' in ${workflow_path}"

failed=0
for pr in "${pr_numbers[@]}"; do
  # The pull request's live head, deliberately, rather than the commit that was
  # squashed into the merge group: the merge_group payload carries no pull
  # request head to compare against, and reading the newest head is what lets
  # this catch a result that turned red after the pull request was queued. Every
  # way the two can disagree is fail-closed — a head that moved after queueing
  # has either no run at all or one still in flight, and both fail below.
  pr_head=$(gh api "repos/${GITHUB_REPOSITORY}/pulls/${pr}" --jq '.head.sha')
  echo "::group::PR #${pr} (head ${pr_head})"

  run=$(
    gh api --paginate --slurp \
      "repos/${GITHUB_REPOSITORY}/actions/runs?head_sha=${pr_head}&event=pull_request&per_page=100" |
      jq -r --arg path "$workflow_path" \
        '[.[].workflow_runs[] | select(.path == $path)]
         | if length == 0 then empty
           else max_by([.run_number, .id]) | [.id, .status, .html_url] | @tsv
           end'
  )

  if [[ -z "$run" ]]; then
    echo "No ${workflow_path} pull_request run on head ${pr_head}."
    echo "Refusing to pass: a gate that cannot see what it is verifying must not report success."
    failed=1
    echo "::endgroup::"
    continue
  fi

  IFS=$'\t' read -r run_id run_status run_url <<<"$run"

  if [[ "$run_status" != "completed" ]]; then
    echo "PENDING  the run is still '${run_status}', so this cannot be merged yet: ${run_url}"
    failed=1
    echo "::endgroup::"
    continue
  fi

  # `/jobs` defaults to the latest run attempt, which is the one whose verdict
  # counts. Every job carrying the gate's name is read, so a duplicate cannot
  # hide behind a green sibling.
  gate_jobs=$(
    gh api --paginate --slurp \
      "repos/${GITHUB_REPOSITORY}/actions/runs/${run_id}/jobs?per_page=100" |
      jq -r --arg name "$gate_name" \
        '.[].jobs[] | select(.name == $name) | [.name, .status, (.conclusion // "")] | @tsv'
  )

  matched=0
  while IFS=$'\t' read -r name status conclusion; do
    [[ -z "$name" ]] && continue
    matched=$((matched + 1))
    if [[ "$status" != "completed" ]]; then
      echo "PENDING  ${name} is '${status}', so this cannot be merged yet: ${run_url}"
      failed=1
    elif [[ "$conclusion" == "success" || "$conclusion" == "skipped" ]]; then
      echo "ok       ${name} (${conclusion}): ${run_url}"
    else
      echo "FAILED   ${name} concluded '${conclusion}': ${run_url}"
      failed=1
    fi
  done <<<"$gate_jobs"

  if [[ $matched -eq 0 ]]; then
    echo "No job named '${gate_name}' in ${run_url}."
    echo "Refusing to pass: a gate that cannot see what it is verifying must not report success."
    failed=1
  fi
  echo "::endgroup::"
done

if [[ $failed -ne 0 ]]; then
  echo "The required gate was not green on the queued pull request head(s)." >&2
  exit 1
fi

echo "'${gate_name}' was green on every queued pull request head."
