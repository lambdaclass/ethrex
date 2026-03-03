#!/usr/bin/env bash
set -euo pipefail

OUTPUT_DIR="${DAILY_REPORT_OUTPUT_DIR:-tooling/daily_report}"
mkdir -p "$OUTPUT_DIR"

# --- LOC section (read from JSON produced by the loc tool) ---
LOC_JSON="tooling/loc/loc_report.json"
LOC_OLD_JSON="tooling/loc/loc_report.json.old"

loc_text=""
if [[ -f "$LOC_JSON" ]]; then
  read -r loc_total loc_l1 loc_l2 loc_levm \
    < <(jq -r '[.ethrex, .ethrex_l1, .ethrex_l2, .levm] | @tsv' "$LOC_JSON")

  if [[ -f "$LOC_OLD_JSON" ]]; then
    read -r loc_old_total loc_old_l1 loc_old_l2 loc_old_levm \
      < <(jq -r '[.ethrex, .ethrex_l1, .ethrex_l2, .levm] | @tsv' "$LOC_OLD_JSON")
  else
    loc_old_total=$loc_total; loc_old_l1=$loc_l1
    loc_old_l2=$loc_l2;       loc_old_levm=$loc_levm
  fi

  fmt_loc_diff() {
    local new=$1 old=$2
    if [[ $new -gt $old ]];   then printf " (+%d)" $((new - old))
    elif [[ $new -lt $old ]]; then printf " (-%d)" $((old - new))
    fi
  }

  # L1 sub-crates: everything except l2 and vm (those are L2 and LEVM)
  l1_crates=$(jq -r '
    .ethrex_crates[]
    | select(.[0] != "l2" and .[0] != "vm")
    | "  • \(.[0]): \(.[1])"
  ' "$LOC_JSON")

  loc_text="Total: ${loc_total}$(fmt_loc_diff "$loc_total" "$loc_old_total")"$'\n'
  loc_text+="• L1: ${loc_l1}$(fmt_loc_diff "$loc_l1" "$loc_old_l1")"$'\n'
  loc_text+="${l1_crates}"$'\n'
  loc_text+="• L2: ${loc_l2}$(fmt_loc_diff "$loc_l2" "$loc_old_l2")"$'\n'
  loc_text+="• LEVM: ${loc_levm}$(fmt_loc_diff "$loc_levm" "$loc_old_levm")"
fi

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

header_text="Daily ethrex report"

# Sort entries for block time (ascending) and throughput (descending)
bt_sort_entries=()
[[ -n "$ethrex_bt" ]]                          && bt_sort_entries+=("$ethrex_bt ethrex")
[[ -n "$reth_bt_p50"   || -n "$reth_bt_p999"  ]] && bt_sort_entries+=("${reth_bt_p50:-0} reth")
[[ -n "$geth_bt_p50"   || -n "$geth_bt_p999"  ]] && bt_sort_entries+=("${geth_bt_p50:-0} geth")
[[ -n "$nether_bt" ]]                          && bt_sort_entries+=("$nether_bt nethermind")

tput_sort_entries=()
[[ -n "$ethrex_tput" ]]                            && tput_sort_entries+=("$ethrex_tput ethrex")
[[ -n "$reth_tput" ]]                              && tput_sort_entries+=("$reth_tput reth")
[[ -n "$geth_tput_p50" || -n "$geth_tput_p999" ]] && tput_sort_entries+=("${geth_tput_p50:-0} geth")
[[ -n "$nether_tput" ]]                            && tput_sort_entries+=("$nether_tput nethermind")

# "Comparing ..." line, listed in block time order
comparing_line=""
while read -r _val client; do
  case "$client" in
    ethrex)     comparing_line+="ethrex (${version_ethrex}), " ;;
    reth)       comparing_line+="reth (${version_reth}), " ;;
    geth)       comparing_line+="geth (${version_geth}), " ;;
    nethermind) comparing_line+="nethermind (${version_nethermind}), " ;;
  esac
done < <(printf '%s\n' "${bt_sort_entries[@]}" | LC_ALL=C sort -n)
comparing_line="${comparing_line%, }"  # strip trailing ", "

# Per-row formatters — right-align name to 10 chars so ":" lines up
fmt_bt_row() {
  case "$1" in
    ethrex)     printf "%10s: %.3fms (mean)\n"                    "ethrex"     "${ethrex_bt:-0}" ;;
    reth)       printf "%10s: %.3fms (p50) | %.3fms (p99.9)\n"   "reth"       "${reth_bt_p50:-0}" "${reth_bt_p999:-0}" ;;
    geth)       printf "%10s: %.3fms (p50) | %.3fms (p99.9)\n"   "geth"       "${geth_bt_p50:-0}" "${geth_bt_p999:-0}" ;;
    nethermind) printf "%10s: %.3fms (mean)\n"                    "nethermind" "${nether_bt:-0}" ;;
  esac
}

fmt_tput_row() {
  case "$1" in
    ethrex)     printf "%10s: %.3f Ggas/s (mean)\n"                       "ethrex"     "${ethrex_tput:-0}" ;;
    reth)       printf "%10s: %.3f Ggas/s (mean)\n"                       "reth"       "${reth_tput:-0}" ;;
    geth)       printf "%10s: %.3f Ggas/s (p50) | %.3f Ggas/s (p99.9)\n" "geth"       "${geth_tput_p50:-0}" "${geth_tput_p999:-0}" ;;
    nethermind) printf "%10s: %.3f Ggas/s (mean)\n"                       "nethermind" "${nether_tput:-0}" ;;
  esac
}

# --- Generate text report for GitHub/Telegram ---
{
  echo "# ${header_text}"
  echo

  if [[ -n "$loc_text" ]]; then
    echo "## Lines of code"
    echo
    printf "%s\n" "$loc_text"
    echo
  fi

  echo "## Comparative performance report (24h average)"
  echo
  echo "Comparing ${comparing_line}"
  echo

  echo "### Block Time"
  echo
  while read -r _val client; do fmt_bt_row "$client"; done \
    < <(printf '%s\n' "${bt_sort_entries[@]}" | LC_ALL=C sort -n)
  echo

  echo "### Throughput"
  echo
  while read -r _val client; do fmt_tput_row "$client"; done \
    < <(printf '%s\n' "${tput_sort_entries[@]}" | LC_ALL=C sort -rn)
} >"${OUTPUT_DIR}/daily_report_github.txt"

# --- Generate Slack JSON ---
# Use code blocks for the aligned tables so monospace rendering preserves column alignment
slack_text=""
if [[ -n "$loc_text" ]]; then
  slack_text="*Lines of code*"$'\n'
  slack_text+='```'$'\n'
  slack_text+="${loc_text}"$'\n'
  slack_text+='```'$'\n\n'
fi

slack_text+="*Comparative performance report (24h average)*"$'\n'
slack_text+="Comparing ${comparing_line}"$'\n\n'

slack_text+="*Block Time*"$'\n'
slack_text+='```'$'\n'
while read -r _val client; do
  slack_text+="$(fmt_bt_row "$client")"$'\n'
done < <(printf '%s\n' "${bt_sort_entries[@]}" | LC_ALL=C sort -n)
slack_text+='```'$'\n\n'

slack_text+="*Throughput*"$'\n'
slack_text+='```'$'\n'
while read -r _val client; do
  slack_text+="$(fmt_tput_row "$client")"$'\n'
done < <(printf '%s\n' "${tput_sort_entries[@]}" | LC_ALL=C sort -rn)
slack_text+='```'$'\n'

jq -n --arg header "$header_text" --arg text "$slack_text" '{
  "blocks": [
    { "type": "header", "text": { "type": "plain_text", "text": $header } },
    { "type": "section", "text": { "type": "mrkdwn", "text": $text } }
  ]
}' >"${OUTPUT_DIR}/daily_report_slack.json"
