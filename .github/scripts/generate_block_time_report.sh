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

# Query version information from eth_exe_web3_client_version metric
VERSION_QUERY="eth_exe_web3_client_version"
version_curl_args=(-sS -G "$QUERY_URL" --data-urlencode "query=$VERSION_QUERY")
if [[ -n "${BLOCK_TIME_PROMETHEUS_BEARER_TOKEN:-}" ]]; then
  version_curl_args+=(-H "Authorization: Bearer $BLOCK_TIME_PROMETHEUS_BEARER_TOKEN")
fi
if [[ -n "${BLOCK_TIME_PROMETHEUS_BASIC_AUTH:-}" ]]; then
  version_curl_args+=(-u "$BLOCK_TIME_PROMETHEUS_BASIC_AUTH")
fi

version_response=$(curl "${version_curl_args[@]}")

# Extract version for each client by instance pattern (bash 3.2 compatible)
# Extracts only the version portion (e.g., "v1.2.3" from "Client/v1.2.3/platform")
get_version_by_instance() {
  local instance_pattern="$1"
  jq -r --arg pattern "$instance_pattern" '
    .data.result[] | select(.metric.instance | test($pattern)) | .metric.version // "unknown" | split("/")[1] // "unknown"
  ' <<<"$version_response" 2>/dev/null | head -1
}

# Truncate version string if longer than max length
truncate_version() {
  local version="$1"
  local max_len="${2:-24}"
  if [[ ${#version} -gt $max_len ]]; then
    echo "${version:0:$((max_len-3))}..."
  else
    echo "$version"
  fi
}

version_ethrex=$(get_version_by_instance "^ethrex-mainnet-1:")
version_reth=$(get_version_by_instance "^reth-mainnet-1:")
version_geth=$(get_version_by_instance "^geth-mainnet-1:")
version_nethermind=$(get_version_by_instance "^nethermind-mainnet-1:")
: "${version_ethrex:=unknown}"
: "${version_reth:=unknown}"
: "${version_geth:=unknown}"
: "${version_nethermind:=unknown}"

# Truncated versions for Slack table display
version_ethrex_short=$(truncate_version "$version_ethrex")
version_reth_short=$(truncate_version "$version_reth")
version_geth_short=$(truncate_version "$version_geth")
version_nethermind_short=$(truncate_version "$version_nethermind")

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

header_text="Daily block time report (24-hour average)"

# Generate text report for GitHub/Telegram (full version on separate line)
# Sorted by block time (ascending - fastest first)
{
  echo "# ${header_text}"
  echo

  # Build sortable entries: "value client"
  sort_entries=()
  if [[ -n "$ethrex_value" ]]; then
    sort_entries+=("$ethrex_value ethrex")
  fi
  if [[ -n "$reth_p50" || -n "$reth_p999" ]]; then
    sort_entries+=("${reth_p50:-0} reth")
  fi
  if [[ -n "$geth_p50" || -n "$geth_p999" ]]; then
    sort_entries+=("${geth_p50:-0} geth")
  fi
  if [[ -n "$nether_value" ]]; then
    sort_entries+=("$nether_value nethermind")
  fi

  # Sort by value and print each client
  while read -r value client; do
    case "$client" in
      ethrex)
        printf "• ethrex: %.3fms (mean)\n  %s\n" "$ethrex_value" "$version_ethrex"
        ;;
      reth)
        printf "• reth: %.3fms (p50) | %.3fms (p99.9)\n  %s\n" "${reth_p50:-0}" "${reth_p999:-0}" "$version_reth"
        ;;
      geth)
        printf "• geth: %.3fms (p50) | %.3fms (p99.9)\n  %s\n" "${geth_p50:-0}" "${geth_p999:-0}" "$version_geth"
        ;;
      nethermind)
        printf "• nethermind: %.3fms (mean)\n  %s\n" "$nether_value" "$version_nethermind"
        ;;
    esac
  done < <(printf '%s\n' "${sort_entries[@]}" | LC_ALL=C sort -n)
} >"${OUTPUT_DIR}/block_time_report_github.txt"

# Generate Slack message (simple format, similar to Telegram)
# Sorted by block time (ascending - fastest first)
slack_text=""

# Build sortable entries: "value client"
slack_sort_entries=()
if [[ -n "$ethrex_value" ]]; then
  slack_sort_entries+=("$ethrex_value ethrex")
fi
if [[ -n "$reth_p50" || -n "$reth_p999" ]]; then
  slack_sort_entries+=("${reth_p50:-0} reth")
fi
if [[ -n "$geth_p50" || -n "$geth_p999" ]]; then
  slack_sort_entries+=("${geth_p50:-0} geth")
fi
if [[ -n "$nether_value" ]]; then
  slack_sort_entries+=("$nether_value nethermind")
fi

# Sort by value and append each client entry
while read -r value client; do
  case "$client" in
    ethrex)
      slack_text+=$(printf "• *ethrex*: %.3fms (mean)\n  %s" "$ethrex_value" "$version_ethrex")$'\n'
      ;;
    reth)
      slack_text+=$(printf "• *reth*: %.3fms (p50) | %.3fms (p99.9)\n  %s" "${reth_p50:-0}" "${reth_p999:-0}" "$version_reth")$'\n'
      ;;
    geth)
      slack_text+=$(printf "• *geth*: %.3fms (p50) | %.3fms (p99.9)\n  %s" "${geth_p50:-0}" "${geth_p999:-0}" "$version_geth")$'\n'
      ;;
    nethermind)
      slack_text+=$(printf "• *nethermind*: %.3fms (mean)\n  %s" "$nether_value" "$version_nethermind")$'\n'
      ;;
  esac
done < <(printf '%s\n' "${slack_sort_entries[@]}" | LC_ALL=C sort -n)

jq -n --arg header "$header_text" --arg text "$slack_text" '{
  "blocks": [
    { "type": "header", "text": { "type": "plain_text", "text": $header } },
    { "type": "section", "text": { "type": "mrkdwn", "text": $text } }
  ]
}' >"${OUTPUT_DIR}/block_time_report_slack.json"
