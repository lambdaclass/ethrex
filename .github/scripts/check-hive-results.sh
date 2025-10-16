#!/usr/bin/env bash

# Verifies Hive JSON results, prints failing tests, copies related logs,
# and updates the GitHub summary to surface the failures in the workflow UI.

set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required to parse Hive results but was not found in PATH"
  exit 1
fi

results_dir="${1:-src/results}"

if [ ! -d "$results_dir" ]; then
  echo "Hive results directory '${results_dir}' not found"
  exit 1
fi

if ! results_dir="$(cd "${results_dir}" >/dev/null 2>&1 && pwd -P)"; then
  echo "Failed to resolve absolute path for Hive results directory"
  exit 1
fi

results_parent="$(dirname "${results_dir}")"
workspace_logs_dir=""
if [ -d "${results_parent}/workspace/logs" ]; then
  workspace_logs_dir="$(cd "${results_parent}/workspace/logs" >/dev/null 2>&1 && pwd -P)"
fi

shopt -s nullglob
json_files=("${results_dir}"/*.json)
shopt -u nullglob

if [ ${#json_files[@]} -eq 0 ]; then
  echo "No Hive JSON result files found in ${results_dir}"
  exit 1
fi

failures=0
failed_logs_root="${results_dir}/failed_logs"
rm -rf "${failed_logs_root}"
mkdir -p "${failed_logs_root}"

for json_file in "${json_files[@]}"; do
  if [[ "${json_file}" == *"hive.json" ]]; then
    continue
  fi

  suite_name="$(jq -r '.name // empty' "${json_file}")"
  failed_cases="$(jq '[.testCases[]? | select(.summaryResult.pass != true)] | length' "${json_file}")"

  if [ "${failed_cases}" -gt 0 ]; then
    echo "Detected ${failed_cases} failing test case(s) in ${suite_name:-$(basename "${json_file}")}"
    failure_list="$(
      jq -r '
        .testCases[]?
        | select(.summaryResult.pass != true)
        | . as $case
        | ($case.summaryResult // {}) as $summary
        | ($summary.message // $summary.reason // $summary.error // "") as $message
        | (if $summary.log?
           then "log lines "
             + (($summary.log.begin // "?") | tostring)
             + "-"
             + (($summary.log.end // "?") | tostring)
           else ""
           end) as $log_hint
        | (if $message != "" then $message else $log_hint end) as $detail
        | (if $case.clientInfo?
           then ($case.clientInfo
                 | to_entries
                 | map((.value.name // .key) + ": " + (.value.logFile // "unknown log"))
                 | join("; "))
           else ""
           end) as $clients
        | "- " + ($case.name // "unknown test")
          + (if $detail != "" then ": " + $detail else "" end)
          + (if $clients != "" then " (client logs: " + $clients + ")" else "" end)
      ' "${json_file}"
    )"

    printf '%s\n' "${failure_list}"

    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
      {
        echo "### Hive failures: ${suite_name:-$(basename "${json_file}" .json)}"
        printf '%s\n' "${failure_list}"
        echo
      } >> "${GITHUB_STEP_SUMMARY}"
    fi

    suite_slug_raw="${suite_name:-$(basename "${json_file}" .json)}"
    suite_slug="$(printf '%s' "${suite_slug_raw}" | tr '[:upper:]' '[:lower:]')"
    suite_slug="$(printf '%s' "${suite_slug}" | sed -E 's/[^a-z0-9._-]+/-/g')"
    suite_slug="${suite_slug#-}"
    suite_slug="${suite_slug%-}"
    suite_dir="${failed_logs_root}/${suite_slug:-suite}"
    mkdir -p "${suite_dir}"

    {
      printf '%s\n' "Detected ${failed_cases} failing test case(s) in ${suite_name:-$(basename "${json_file}")}"
      printf '%s\n' "${failure_list}"
      echo
    } >> "${suite_dir}/failed-tests.txt"

    cp "${json_file}" "${suite_dir}/"

    suite_logs_output="$(
      jq -r '
        [
          .simLog?,
          .testDetailsLog?,
          (.testCases[]? | select(.summaryResult.pass != true) | .clientInfo? | to_entries? // [] | map(.value.logFile? // empty) | .[]),
          (.testCases[]? | select(.summaryResult.pass != true) | .summaryResult.logFile?),
          (.testCases[]? | select(.summaryResult.pass != true) | .logFile?)
        ]
        | map(select(. != null and . != ""))
        | unique
        | .[]
      ' "${json_file}" 2>/dev/null || true
    )"

    if [ -n "${suite_logs_output}" ]; then
      while IFS= read -r log_rel; do
        [ -z "${log_rel}" ] && continue

        log_path=""
        if [[ "${log_rel}" == /* ]]; then
          if [ -f "${log_rel}" ]; then
            log_path="${log_rel}"
          fi
        else
          candidate_paths=(
            "${results_dir}/${log_rel}"
            "${results_dir}/logs/${log_rel}"
          )
          if [ -n "${workspace_logs_dir}" ]; then
            candidate_paths+=("${workspace_logs_dir}/${log_rel}")
          fi

          for candidate in "${candidate_paths[@]}"; do
            if [ -f "${candidate}" ]; then
              log_path="${candidate}"
              break
            fi
          done
        fi

        if [ -z "${log_path}" ] && [[ "${log_rel}" != /* ]]; then
          search_roots=("${results_dir}")
          if [ -d "${results_dir}/logs" ]; then
            search_roots+=("${results_dir}/logs")
          fi
          if [ -n "${workspace_logs_dir}" ]; then
            search_roots+=("${workspace_logs_dir}")
          fi

          for search_root in "${search_roots[@]}"; do
            [ -d "${search_root}" ] || continue
            found_log="$(find "${search_root}" -type f -name "$(basename "${log_rel}")" -print -quit 2>/dev/null || true)"
            if [ -n "${found_log}" ]; then
              log_path="${found_log}"
              break
            fi
          done
        fi

        if [ -n "${log_path}" ]; then
          target_path="${suite_dir}/${log_rel}"
          mkdir -p "$(dirname "${target_path}")"
          if [ ! -f "${target_path}" ]; then
            cp "${log_path}" "${target_path}"
          fi
        else
          echo "Referenced log '${log_rel}' not found for suite ${suite_name:-$(basename "${json_file}")}"
        fi
      done <<< "${suite_logs_output}"
    fi

    echo "Saved Hive failure artifacts to ${suite_dir}"

    failures=$((failures + failed_cases))
  fi
done

if [ "${failures}" -gt 0 ]; then
  echo "Hive reported ${failures} failing test cases in total"
  exit 1
fi

echo "Hive reported no failing test cases."
