#!/usr/bin/env bash

# Verifies Hive JSON results, prints failing tests, copies related logs,
# and updates the GitHub summary to surface the failures in the workflow UI.

set -euo pipefail

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required to parse Hive results but was not found in PATH"
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required to process Hive client logs but was not found in PATH"
  exit 1
fi

slugify() {
  local input="${1:-}"
  local lowered trimmed
  lowered="$(printf '%s' "${input}" | tr '[:upper:]' '[:lower:]')"
  trimmed="$(printf '%s' "${lowered}" | sed -E 's/[^a-z0-9._-]+/-/g')"
  trimmed="${trimmed#-}"
  trimmed="${trimmed%-}"
  printf '%s' "${trimmed}"
}

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

# Known-flaky tests to ignore (substring match against test case name).
# These are hive framework issues, not ethrex bugs.
KNOWN_FLAKY_TESTS=(
  "Invalid Missing Ancestor Syncing ReOrg, Timestamp, EmptyTxs=False, CanonicalReOrg=False, Invalid P8"
  "Invalid Missing Ancestor Syncing ReOrg, Timestamp, EmptyTxs=False, CanonicalReOrg=True, Invalid P8"
)

# Build a jq filter that excludes known-flaky tests.
flaky_filter='true'
for pattern in "${KNOWN_FLAKY_TESTS[@]}"; do
  flaky_filter="${flaky_filter} and (.name | contains(\"${pattern}\") | not)"
done

for json_file in "${json_files[@]}"; do
  if [[ "${json_file}" == *"hive.json" ]]; then
    continue
  fi

  suite_name="$(jq -r '.name // empty' "${json_file}")"
  failed_cases="$(jq '[.testCases[]? | select(.summaryResult.pass != true) | select('"${flaky_filter}"')] | length' "${json_file}")"

  skipped_flaky="$(jq '[.testCases[]? | select(.summaryResult.pass != true) | select(('"${flaky_filter}"') | not)] | length' "${json_file}")"
  if [ "${skipped_flaky}" -gt 0 ]; then
    echo "Ignoring ${skipped_flaky} known-flaky test(s) in ${suite_name:-$(basename "${json_file}")}"
  fi

  if [ "${failed_cases}" -gt 0 ]; then
    echo "Detected ${failed_cases} failing test case(s) in ${suite_name:-$(basename "${json_file}")}"
    failure_list="$(
      jq -r '
        .testCases[]?
        | select(.summaryResult.pass != true)
        | select('"${flaky_filter}"')
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
        echo "Note: Hive scenarios may include multiple ethrex clients, so each failing case can have more than one log snippet."
        echo
      } >> "${GITHUB_STEP_SUMMARY}"
    fi

    suite_slug_raw="${suite_name:-$(basename "${json_file}" .json)}"
    suite_slug="$(slugify "${suite_slug_raw}")"
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
          (.testCases[]? | select(.summaryResult.pass != true) | select('"${flaky_filter}"') | .clientInfo? | to_entries? // [] | map(.value.logFile? // empty) | .[]),
          (.testCases[]? | select(.summaryResult.pass != true) | select('"${flaky_filter}"') | .summaryResult.logFile?),
          (.testCases[]? | select(.summaryResult.pass != true) | select('"${flaky_filter}"') | .logFile?)
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

    client_case_entries="$(
      jq -r '
        .testCases
        | to_entries[]
        | select(.value.summaryResult.pass != true)
        | select(.value | '"${flaky_filter}"')
        | . as $case_entry
        | ($case_entry.value.clientInfo? // {}) | to_entries[]
        | [
            .value.logFile // "",
            ($case_entry.value.name // ("case-" + $case_entry.key)),
            $case_entry.key,
            ($case_entry.value.start // ""),
            ($case_entry.value.end // ""),
            .key
          ]
        | @tsv
      ' "${json_file}" 2>/dev/null || true
    )"
    generated_client_snippets=0
    if [ -n "${client_case_entries}" ]; then
      client_logs_dir="${suite_dir}/client_logs"
      mkdir -p "${client_logs_dir}"

      while IFS= read -r client_entry; do
        [ -n "${client_entry}" ] || continue
        IFS=$'\t' read -r client_log_rel raw_case_name case_id case_start case_end client_id <<< "${client_entry}"

        if [ -z "${client_log_rel}" ] || [ -z "${case_start}" ] || [ -z "${case_end}" ]; then
          continue
        fi

        log_copy_path="${suite_dir}/${client_log_rel}"
        if [ ! -f "${log_copy_path}" ]; then
          continue
        fi

        case_slug="$(slugify "${raw_case_name}")"
        if [ -n "${case_slug}" ]; then
          case_slug="${case_slug}-case-${case_id}"
        else
          case_slug="case-${case_id}"
        fi

        client_slug="$(slugify "${client_id}")"
        if [ -z "${client_slug}" ]; then
          client_slug="client"
        fi

        case_dir="${client_logs_dir}/${case_slug}"
        mkdir -p "${case_dir}"
        snippet_path="${case_dir}/${client_slug}.log"

        python3 - "${log_copy_path}" "${snippet_path}" "${raw_case_name}" "${case_start}" "${case_end}" "${client_id}" "${client_log_rel}" <<'PY'
import sys
from datetime import datetime, timedelta
from pathlib import Path

FORMATS = ("%Y-%m-%dT%H:%M:%S.%fZ", "%Y-%m-%dT%H:%M:%SZ")
CONTEXT_SECONDS = 2
PREFETCH_LIMIT = 50

def normalise_timestamp_str(value):
    if not value or not value.endswith("Z"):
        return value
    prefix = value[:-1]
    if "." not in prefix:
        return value
    base, frac = prefix.split(".", 1)
    frac_digits = "".join(ch for ch in frac if ch.isdigit())
    if not frac_digits:
        return f"{base}.000000Z"
    frac_digits = (frac_digits + "000000")[:6]
    return f"{base}.{frac_digits}Z"

def parse_timestamp(value):
    if not value:
        return None
    value = normalise_timestamp_str(value)
    for fmt in FORMATS:
        try:
            return datetime.strptime(value, fmt)
        except ValueError:
            continue
    return None

def timestamp_from_line(line):
    if not line:
        return None
    token = line.split(" ", 1)[0]
    if not token or not token[0].isdigit():
        return None
    token = normalise_timestamp_str(token)
    for fmt in FORMATS:
        try:
            return datetime.strptime(token, fmt)
        except ValueError:
            continue
    return None

log_path = Path(sys.argv[1])
output_path = Path(sys.argv[2])
case_name = sys.argv[3]
case_start_raw = sys.argv[4]
case_end_raw = sys.argv[5]
client_id = sys.argv[6] or "unknown"
client_log_rel = sys.argv[7]

try:
    log_content = log_path.read_text(encoding="utf-8", errors="replace").splitlines(keepends=True)
except Exception as exc:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(f"# Failed to read log '{log_path}': {exc}\n", encoding="utf-8")
    sys.exit(0)

start_ts = parse_timestamp(case_start_raw)
end_ts = parse_timestamp(case_end_raw)

fallback_reason = None
if not start_ts or not end_ts or end_ts < start_ts:
    fallback_reason = "Unable to determine reliable time window from test metadata."
else:
    start_ts = start_ts - timedelta(seconds=CONTEXT_SECONDS)
    end_ts = end_ts + timedelta(seconds=CONTEXT_SECONDS)

captured_lines = []
prefetch = []
current_ts = None
capturing = False

if not fallback_reason:
    for line in log_content:
        ts = timestamp_from_line(line)
        if ts is not None:
            current_ts = ts

        if not capturing:
            prefetch.append(line)
            if len(prefetch) > PREFETCH_LIMIT:
                prefetch.pop(0)

        in_window = current_ts is not None and start_ts <= current_ts <= end_ts

        if in_window:
            if not capturing:
                captured_lines.extend(prefetch)
                capturing = True
            captured_lines.append(line)
        elif capturing and current_ts is not None and current_ts > end_ts:
            break
        elif capturing:
            captured_lines.append(line)

    if not captured_lines:
        fallback_reason = "No timestamped log lines matched the computed time window."

if fallback_reason:
    captured_lines = log_content

header_lines = [
    f"# Test: {case_name}\n",
    f"# Client ID: {client_id}\n",
    f"# Source log: {client_log_rel}\n",
]

if start_ts and end_ts and not fallback_reason:
    header_lines.append(
        f"# Time window (UTC): {case_start_raw} .. {case_end_raw} (with ±{CONTEXT_SECONDS}s context)\n"
    )
else:
    header_lines.append("# Time window (UTC): unavailable\n")

if fallback_reason:
    header_lines.append(f"# NOTE: {fallback_reason}\n")

header_lines.append("\n")

output_path.parent.mkdir(parents=True, exist_ok=True)
with output_path.open("w", encoding="utf-8") as dst:
    dst.writelines(header_lines)
    dst.writelines(captured_lines)
PY

        if [ -s "${snippet_path}" ]; then
          generated_client_snippets=$((generated_client_snippets + 1))
        fi
      done <<< "${client_case_entries}"
    fi

    if [ "${generated_client_snippets}" -gt 0 ]; then
      echo "Generated ${generated_client_snippets} client log snippet(s) in ${client_logs_dir}"
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
