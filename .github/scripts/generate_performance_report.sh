#!/usr/bin/env bash
set -ex
set -euo pipefail

OUTPUT_DIR="${PERF_REPORT_OUTPUT_DIR:-tooling/performance_report}"
mkdir -p "$OUTPUT_DIR"

BASE_URL="${PERF_PROMETHEUS_URL:-${PROMETHEUS_URL:-}}"
RANGE="${PERF_PROMETHEUS_RANGE:-24h}"
SELECTOR="${PERF_PROMETHEUS_SELECTOR:-job=\"ethrex-l1\"}"
DEFAULT_QUERY="avg(avg_over_time(gigagas{${SELECTOR}}[${RANGE}]))"
QUERY="${PERF_PROMETHEUS_QUERY:-$DEFAULT_QUERY}"

if [[ -z "$BASE_URL" ]]; then
  echo "Set PERF_PROMETHEUS_URL to build the Prometheus query endpoint." >&2
  exit 1
fi
QUERY_URL="${BASE_URL%/}/api/v1/query"

curl_args=(-sS -G "$QUERY_URL" --data-urlencode "query=$QUERY")
if [[ -n "${PERF_PROMETHEUS_BEARER_TOKEN:-}" ]]; then
  curl_args+=(-H "Authorization: Bearer $PERF_PROMETHEUS_BEARER_TOKEN")
fi
if [[ -n "${PERF_PROMETHEUS_BASIC_AUTH:-}" ]]; then
  curl_args+=(-u "$PERF_PROMETHEUS_BASIC_AUTH")
fi

response=$(curl "${curl_args[@]}")
status=$(jq -r '.status // empty' <<<"$response")
if [[ "$status" != "success" ]]; then
  echo "Prometheus query failed: $(jq -r '.error // .errorType // "unknown error"' <<<"$response")" >&2
  exit 1
fi

result_count=$(jq '.data.result | length' <<<"$response")
if [[ "$result_count" -eq 0 ]]; then
  echo "Prometheus query returned no data" >&2
  exit 1
fi

raw_series=()
while IFS= read -r line; do
  raw_series+=("$line")
done < <(jq -r '
  .data.result[]
  | [
      (.metric.client // .metric.instance // .metric.job // "series"),
      (.value[1]),
      (.metric.instance // "unknown-instance")
    ]
  | @tsv
  ' <<<"$response")
nether_line=""
ethrex_line=""
reth_line=""
extra_lines=()
slack_nether_line=""
slack_ethrex_line=""
slack_reth_line=""
slack_extra_lines=()
for row in "${raw_series[@]}"; do
  IFS=$'\t' read -r series_name series_value series_instance <<<"$row"
  host="${series_instance%%:*}"
  qualifier="mean"
  if [[ "$series_name" == "reth" ]]; then
    qualifier="p50"
  fi
  line=$(printf "* %s (%s, 24-hour): %.3f Ggas/s\n  %s" "$series_name" "$qualifier" "$series_value" "$host")
  slack_line=$(printf "• *%s* (%s, 24-hour): %.3f Ggas/s\n    %s" "$series_name" "$qualifier" "$series_value" "$host")
  case "$series_name" in
  nethermind)
    nether_line="$line"
    slack_nether_line="$slack_line"
    ;;
  ethrex)
    ethrex_line="$line"
    slack_ethrex_line="$slack_line"
    ;;
  reth)
    reth_line="$line"
    slack_reth_line="$slack_line"
    ;;
  *)
    extra_lines+=("$line")
    slack_extra_lines+=("$slack_line")
    ;;
  esac
done

ordered_lines=()
[[ -n "$ethrex_line" ]] && ordered_lines+=("$ethrex_line")
[[ -n "$reth_line" ]] && ordered_lines+=("$reth_line")
[[ -n "$nether_line" ]] && ordered_lines+=("$nether_line")
ordered_lines+=("${extra_lines[@]:-}")

ordered_slack_lines=()
[[ -n "$slack_ethrex_line" ]] && ordered_slack_lines+=("$slack_ethrex_line")
[[ -n "$slack_reth_line" ]] && ordered_slack_lines+=("$slack_reth_line")
[[ -n "$slack_nether_line" ]] && ordered_slack_lines+=("$slack_nether_line")
ordered_slack_lines+=("${slack_extra_lines[@]:-}")

header_text="Daily performance report"
{
  echo "# ${header_text}"
  echo
  printf '%s\n' "${ordered_lines[@]}"
} >"${OUTPUT_DIR}/performance_report_github.txt"

series_text=""
for entry in "${ordered_slack_lines[@]}"; do
  if [[ -n "$series_text" ]]; then
    series_text+=$'\n\n'
  fi
  series_text+="$entry"
done
jq -n --arg header "$header_text" --arg series "$series_text" '{
  "blocks": [
    { "type": "header", "text": { "type": "plain_text", "text": $header } },
    { "type": "section", "text": { "type": "mrkdwn", "text": $series } }
  ]
}' >"${OUTPUT_DIR}/performance_report_slack.json"
