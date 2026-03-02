#!/usr/bin/env bash
set -euo pipefail

OUTPUT_DIR="${DAILY_REPORT_OUTPUT_DIR:-tooling/daily_report}"
mkdir -p "$OUTPUT_DIR"

BASE_URL="${PERF_PROMETHEUS_URL:-${BLOCK_TIME_PROMETHEUS_URL:-${PROMETHEUS_URL:-}}}"
if [[ -z "$BASE_URL" ]]; then
  echo "Set PERF_PROMETHEUS_URL to build the Prometheus query endpoint." >&2
  exit 1
fi
QUERY_URL="${BASE_URL%/}/api/v1/query"

PERF_RANGE="${PERF_PROMETHEUS_RANGE:-24h}"
BLOCK_TIME_RANGE="${BLOCK_TIME_PROMETHEUS_RANGE:-24h}"

# Build auth args (same Prometheus instance for both queries)
auth_args=()
if [[ -n "${PERF_PROMETHEUS_BEARER_TOKEN:-}" ]]; then
  auth_args+=(-H "Authorization: Bearer $PERF_PROMETHEUS_BEARER_TOKEN")
fi
if [[ -n "${PERF_PROMETHEUS_BASIC_AUTH:-}" ]]; then
  auth_args+=(-u "$PERF_PROMETHEUS_BASIC_AUTH")
fi

prometheus_query() {
  local query="$1"
  curl -sS -G "$QUERY_URL" "${auth_args[@]}" --data-urlencode "query=$query"
}

check_response() {
  local response="$1"
  local context="$2"
  local status
  status=$(jq -r '.status // empty' <<<"$response")
  if [[ "$status" != "success" ]]; then
    echo "Prometheus query failed ($context): $(jq -r '.error // .errorType // "unknown error"' <<<"$response")" >&2
    exit 1
  fi
  local count
  count=$(jq '.data.result | length' <<<"$response")
  if [[ "$count" -eq 0 ]]; then
    echo "Prometheus query returned no data ($context)" >&2
    exit 1
  fi
}

# --- Throughput query ---
PERF_SELECTOR="${PERF_PROMETHEUS_SELECTOR:-job=\"ethrex-l1\"}"
DEFAULT_PERF_QUERY="avg(avg_over_time(gigagas{${PERF_SELECTOR}}[${PERF_RANGE}]))"
PERF_QUERY="${PERF_PROMETHEUS_QUERY:-$DEFAULT_PERF_QUERY}"

perf_response=$(prometheus_query "$PERF_QUERY")
check_response "$perf_response" "throughput"

# --- Block time query ---
BLOCK_TIME_SELECTOR="${BLOCK_TIME_PROMETHEUS_SELECTOR:-job=\"ethrex-l1\"}"
DEFAULT_BLOCK_TIME_QUERY="avg(avg_over_time(block_time{${BLOCK_TIME_SELECTOR}}[${BLOCK_TIME_RANGE}]))"
BLOCK_TIME_QUERY="${BLOCK_TIME_PROMETHEUS_QUERY:-$DEFAULT_BLOCK_TIME_QUERY}"

block_time_response=$(prometheus_query "$BLOCK_TIME_QUERY")
check_response "$block_time_response" "block time"

# --- Version queries ---
# For reth/geth/nethermind: use eth_exe_web3_client_version (version string already contains short commit)
version_response=$(prometheus_query "eth_exe_web3_client_version")

# For ethrex: use ethrex_info with separate version/branch/commit labels
ethrex_info_response=$(prometheus_query 'ethrex_info{instance=~"ethrex-mainnet-1:.*"}')

# Extract version for reth/geth/nethermind by instance pattern (bash 3.2 compatible)
# Extracts only the version portion (e.g., "v1.2.3" from "Client/v1.2.3/platform")
get_version_by_instance() {
  local instance_pattern="$1"
  jq -r --arg pattern "$instance_pattern" '
    .data.result[] | select(.metric.instance | test($pattern)) | .metric.version // "unknown" | split("/")[1] // "unknown"
  ' <<<"$version_response" 2>/dev/null | head -1
}

# Compose ethrex version from ethrex_info labels: v{version}-{branch}-{commit[:8]}
ethrex_info_version=$(jq -r '.data.result[0].metric.version // ""' <<<"$ethrex_info_response" 2>/dev/null)
ethrex_info_branch=$(jq -r '.data.result[0].metric.branch // ""' <<<"$ethrex_info_response" 2>/dev/null)
ethrex_info_commit=$(jq -r '.data.result[0].metric.commit // ""' <<<"$ethrex_info_response" 2>/dev/null)

if [[ -n "$ethrex_info_version" && -n "$ethrex_info_branch" && -n "$ethrex_info_commit" ]]; then
  version_ethrex="v${ethrex_info_version}-${ethrex_info_branch}-${ethrex_info_commit:0:8}"
else
  version_ethrex=$(get_version_by_instance "^ethrex-mainnet-1:")
fi
: "${version_ethrex:=unknown}"

version_reth=$(get_version_by_instance "^reth-mainnet-1:")
version_geth=$(get_version_by_instance "^geth-mainnet-1:")
version_nethermind=$(get_version_by_instance "^nethermind-mainnet-1:")
: "${version_reth:=unknown}"
: "${version_geth:=unknown}"
: "${version_nethermind:=unknown}"

# --- Parse throughput data ---
ethrex_tput=""
nether_tput=""
reth_tput=""
geth_tput_p50=""
geth_tput_p999=""

raw_perf=()
while IFS= read -r line; do
  raw_perf+=("$line")
done < <(jq -r '
  .data.result[]
  | [
      (.metric.client // .metric.instance // .metric.job // "series"),
      (.value[1]),
      (.metric.instance // "unknown-instance"),
      (.metric.quantile // "")
    ]
  | @tsv
  ' <<<"$perf_response")

for row in "${raw_perf[@]}"; do
  IFS=$'\t' read -r series_name series_value series_instance series_quantile <<<"$row"
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
    ethrex:mean)     ethrex_tput="$series_value" ;;
    reth:mean)       reth_tput="$series_value" ;;
    nethermind:mean) nether_tput="$series_value" ;;
    geth:p50)        geth_tput_p50="$series_value" ;;
    geth:p99.9)      geth_tput_p999="$series_value" ;;
  esac
done

# --- Parse block time data ---
ethrex_bt=""
nether_bt=""
reth_bt_p50=""
reth_bt_p999=""
geth_bt_p50=""
geth_bt_p999=""

raw_bt=()
while IFS= read -r line; do
  raw_bt+=("$line")
done < <(jq -r '
  .data.result[]
  | [
      (.metric.client // .metric.instance // .metric.job // "series"),
      (.value[1]),
      (.metric.instance // "unknown-instance"),
      (.metric.quantile // "")
    ]
  | @tsv
  ' <<<"$block_time_response")

for row in "${raw_bt[@]}"; do
  IFS=$'\t' read -r series_name series_value series_instance series_quantile <<<"$row"
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
    ethrex:mean)     ethrex_bt="$series_value" ;;
    reth:p50)        reth_bt_p50="$series_value" ;;
    reth:p99.9)      reth_bt_p999="$series_value" ;;
    geth:p50)        geth_bt_p50="$series_value" ;;
    geth:p99.9)      geth_bt_p999="$series_value" ;;
    nethermind:mean) nether_bt="$series_value" ;;
  esac
done

header_text="Daily performance report (24-hour average)"

# Build sort keys per client: block time ascending (p50 for reth/geth, mean for ethrex/nethermind)
sort_entries=()
[[ -n "$ethrex_bt" ]] && sort_entries+=("$ethrex_bt ethrex")
[[ -n "$reth_bt_p50" || -n "$reth_bt_p999" ]] && sort_entries+=("${reth_bt_p50:-0} reth")
[[ -n "$geth_bt_p50" || -n "$geth_bt_p999" ]] && sort_entries+=("${geth_bt_p50:-0} geth")
[[ -n "$nether_bt" ]] && sort_entries+=("$nether_bt nethermind")

# --- Generate text report for GitHub/Telegram ---
{
  echo "# ${header_text}"
  echo

  while read -r _sort_val client; do
    case "$client" in
      ethrex)
        printf "• ethrex (%s)\n" "$version_ethrex"
        printf "  Block time: %.3fms (mean) | Throughput: %.3f Ggas/s (mean)\n" \
          "${ethrex_bt:-0}" "${ethrex_tput:-0}"
        ;;
      reth)
        printf "• reth (%s)\n" "$version_reth"
        printf "  Block time: %.3fms (p50) | %.3fms (p99.9) | Throughput: %.3f Ggas/s (mean)\n" \
          "${reth_bt_p50:-0}" "${reth_bt_p999:-0}" "${reth_tput:-0}"
        ;;
      geth)
        printf "• geth (%s)\n" "$version_geth"
        printf "  Block time: %.3fms (p50) | %.3fms (p99.9) | Throughput: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)\n" \
          "${geth_bt_p50:-0}" "${geth_bt_p999:-0}" "${geth_tput_p50:-0}" "${geth_tput_p999:-0}"
        ;;
      nethermind)
        printf "• nethermind (%s)\n" "$version_nethermind"
        printf "  Block time: %.3fms (mean) | Throughput: %.3f Ggas/s (mean)\n" \
          "${nether_bt:-0}" "${nether_tput:-0}"
        ;;
    esac
  done < <(printf '%s\n' "${sort_entries[@]}" | LC_ALL=C sort -n)
} >"${OUTPUT_DIR}/daily_report_github.txt"

# --- Generate Slack JSON ---
slack_text=""

while read -r _sort_val client; do
  case "$client" in
    ethrex)
      slack_text+=$(printf "• *ethrex* (%s)\n  Block time: %.3fms (mean) | Throughput: %.3f Ggas/s (mean)" \
        "$version_ethrex" "${ethrex_bt:-0}" "${ethrex_tput:-0}")$'\n'
      ;;
    reth)
      slack_text+=$(printf "• *reth* (%s)\n  Block time: %.3fms (p50) | %.3fms (p99.9) | Throughput: %.3f Ggas/s (mean)" \
        "$version_reth" "${reth_bt_p50:-0}" "${reth_bt_p999:-0}" "${reth_tput:-0}")$'\n'
      ;;
    geth)
      slack_text+=$(printf "• *geth* (%s)\n  Block time: %.3fms (p50) | %.3fms (p99.9) | Throughput: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)" \
        "$version_geth" "${geth_bt_p50:-0}" "${geth_bt_p999:-0}" "${geth_tput_p50:-0}" "${geth_tput_p999:-0}")$'\n'
      ;;
    nethermind)
      slack_text+=$(printf "• *nethermind* (%s)\n  Block time: %.3fms (mean) | Throughput: %.3f Ggas/s (mean)" \
        "$version_nethermind" "${nether_bt:-0}" "${nether_tput:-0}")$'\n'
      ;;
  esac
done < <(printf '%s\n' "${sort_entries[@]}" | LC_ALL=C sort -n)

jq -n --arg header "$header_text" --arg text "$slack_text" '{
  "blocks": [
    { "type": "header", "text": { "type": "plain_text", "text": $header } },
    { "type": "section", "text": { "type": "mrkdwn", "text": $text } }
  ]
}' >"${OUTPUT_DIR}/daily_report_slack.json"
