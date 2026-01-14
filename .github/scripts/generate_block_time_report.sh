#!/usr/bin/env bash
set -euo pipefail

OUTPUT_DIR="${BLOCK_TIME_REPORT_OUTPUT_DIR:-tooling/block_time_report}"
mkdir -p "$OUTPUT_DIR"

BASE_URL="${BLOCK_TIME_PROMETHEUS_URL:-${PROMETHEUS_URL:-}}"
RANGE="${BLOCK_TIME_PROMETHEUS_RANGE:-24h}"
SELECTOR="${BLOCK_TIME_PROMETHEUS_SELECTOR:-job=\"ethrex-l1\"}"
DEFAULT_QUERY="avg(avg_over_time(block_time{${SELECTOR}}[${RANGE}]))"
QUERY="${BLOCK_TIME_PROMETHEUS_QUERY:-$DEFAULT_QUERY}"

if [[ -z "$BASE_URL" ]]; then
  echo "Set BLOCK_TIME_PROMETHEUS_URL to build the Prometheus query endpoint." >&2
  exit 1
fi
QUERY_URL="${BASE_URL%/}/api/v1/query"

curl_args=(-sS -G "$QUERY_URL" --data-urlencode "query=$QUERY")
if [[ -n "${BLOCK_TIME_PROMETHEUS_BEARER_TOKEN:-}" ]]; then
  curl_args+=(-H "Authorization: Bearer $BLOCK_TIME_PROMETHEUS_BEARER_TOKEN")
fi
if [[ -n "${BLOCK_TIME_PROMETHEUS_BASIC_AUTH:-}" ]]; then
  curl_args+=(-u "$BLOCK_TIME_PROMETHEUS_BASIC_AUTH")
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
      line=$(printf "* %s: %.3fms (%s)\n  %s" "$series_name" "$series_value" "$qualifier" "$host")
      slack_line=$(printf "• *%s*: %.3fms (%s)\n    %s" "$series_name" "$series_value" "$qualifier" "$host")
      extra_lines+=("$line")
      slack_extra_lines+=("$slack_line")
      ;;
  esac
done

ordered_lines=()
if [[ -n "$ethrex_value" ]]; then
  ordered_lines+=("$(printf "* ethrex: %.3fms (mean)\n  %s" "$ethrex_value" "$ethrex_host")")
fi
if [[ -n "$reth_p50" || -n "$reth_p999" ]]; then
  ordered_lines+=("$(printf "* reth: %.3fms (p50) | %.3fms (p99.9)\n  %s" "${reth_p50:-0}" "${reth_p999:-0}" "$reth_host")")
fi
if [[ -n "$geth_p50" || -n "$geth_p999" ]]; then
  ordered_lines+=("$(printf "* geth: %.3fms (p50) | %.3fms (p99.9)\n  %s" "${geth_p50:-0}" "${geth_p999:-0}" "$geth_host")")
fi
if [[ -n "$nether_value" ]]; then
  ordered_lines+=("$(printf "* nethermind: %.3fms (mean)\n  %s" "$nether_value" "$nether_host")")
fi
ordered_lines+=("${extra_lines[@]:-}")

ordered_slack_lines=()
if [[ -n "$ethrex_value" ]]; then
  ordered_slack_lines+=("$(printf "• *ethrex*: %.3fms (mean)\n    %s" "$ethrex_value" "$ethrex_host")")
fi
if [[ -n "$reth_p50" || -n "$reth_p999" ]]; then
  ordered_slack_lines+=("$(printf "• *reth*: %.3fms (p50) | %.3fms (p99.9)\n    %s" "${reth_p50:-0}" "${reth_p999:-0}" "$reth_host")")
fi
if [[ -n "$geth_p50" || -n "$geth_p999" ]]; then
  ordered_slack_lines+=("$(printf "• *geth*: %.3fms (p50) | %.3fms (p99.9)\n    %s" "${geth_p50:-0}" "${geth_p999:-0}" "$geth_host")")
fi
if [[ -n "$nether_value" ]]; then
  ordered_slack_lines+=("$(printf "• *nethermind*: %.3fms (mean)\n    %s" "$nether_value" "$nether_host")")
fi
ordered_slack_lines+=("${slack_extra_lines[@]:-}")

header_text="Daily block time report (24-hour average)"
{
  echo "# ${header_text}"
  echo
  printf '%s\n' "${ordered_lines[@]}"
} >"${OUTPUT_DIR}/block_time_report_github.txt"

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
}' >"${OUTPUT_DIR}/block_time_report_slack.json"
