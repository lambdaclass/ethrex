#!/usr/bin/env bash
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
      (.metric.instance // "unknown-instance"),
      (.metric.quantile // "")
    ]
  | @tsv
  ' <<<"$response")
ethrex_value=""
nether_value=""
reth_p50=""
reth_p999=""
geth_p50=""
geth_p999=""
reth_host=""
geth_host=""
ethrex_host=""
nether_host=""
extra_lines=()
slack_extra_lines=()
for row in "${raw_series[@]}"; do
  IFS=$'\t' read -r series_name series_value series_instance series_quantile <<<"$row"
  host="${series_instance%%:*}"
  if [[ -n "$series_quantile" ]]; then
    case "$series_quantile" in
      0.5) qualifier="p50" ;;
      0.999) qualifier="p99.9" ;;
      *) qualifier="p${series_quantile}" ;;
    esac
  else
    qualifier="mean"
  fi
  case "${series_name}:${qualifier}" in
    ethrex:mean)
      ethrex_value="$series_value"
      ethrex_host="$host"
      ;;
    reth:p50)
      reth_p50="$series_value"
      reth_host="$host"
      ;;
    reth:p99.9)
      reth_p999="$series_value"
      reth_host="$host"
      ;;
    geth:p50)
      geth_p50="$series_value"
      geth_host="$host"
      ;;
    geth:p99.9)
      geth_p999="$series_value"
      geth_host="$host"
      ;;
    nethermind:mean)
      nether_value="$series_value"
      nether_host="$host"
      ;;
    *)
      line=$(printf "* %s: %.3f Ggas/s (%s)\n  %s" "$series_name" "$series_value" "$qualifier" "$host")
      slack_line=$(printf "• *%s*: %.3f Ggas/s (%s)\n    %s" "$series_name" "$series_value" "$qualifier" "$host")
      extra_lines+=("$line")
      slack_extra_lines+=("$slack_line")
      ;;
  esac
done

ordered_lines=()
if [[ -n "$ethrex_value" ]]; then
  ordered_lines+=("$(printf "* ethrex: %.3f Ggas/s (mean)\n  %s" "$ethrex_value" "$ethrex_host")")
fi
if [[ -n "$reth_p50" || -n "$reth_p999" ]]; then
  ordered_lines+=("$(printf "* reth: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)\n  %s" "${reth_p50:-0}" "${reth_p999:-0}" "$reth_host")")
fi
if [[ -n "$geth_p50" || -n "$geth_p999" ]]; then
  ordered_lines+=("$(printf "* geth: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)\n  %s" "${geth_p50:-0}" "${geth_p999:-0}" "$geth_host")")
fi
if [[ -n "$nether_value" ]]; then
  ordered_lines+=("$(printf "* nethermind: %.3f Ggas/s (mean)\n  %s" "$nether_value" "$nether_host")")
fi
ordered_lines+=("${extra_lines[@]:-}")

ordered_slack_lines=()
if [[ -n "$ethrex_value" ]]; then
  ordered_slack_lines+=("$(printf "• *ethrex*: %.3f Ggas/s (mean)\n    %s" "$ethrex_value" "$ethrex_host")")
fi
if [[ -n "$reth_p50" || -n "$reth_p999" ]]; then
  ordered_slack_lines+=("$(printf "• *reth*: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)\n    %s" "${reth_p50:-0}" "${reth_p999:-0}" "$reth_host")")
fi
if [[ -n "$geth_p50" || -n "$geth_p999" ]]; then
  ordered_slack_lines+=("$(printf "• *geth*: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)\n    %s" "${geth_p50:-0}" "${geth_p999:-0}" "$geth_host")")
fi
if [[ -n "$nether_value" ]]; then
  ordered_slack_lines+=("$(printf "• *nethermind*: %.3f Ggas/s (mean)\n    %s" "$nether_value" "$nether_host")")
fi
ordered_slack_lines+=("${slack_extra_lines[@]:-}")

header_text="Daily performance report (24-hour average)"
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
