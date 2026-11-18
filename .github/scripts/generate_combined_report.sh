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

  fmt_num() {
    # Add comma thousands separators (e.g. 84994 → 84,994) — pure bash, no sed
    local n out='' len i
    n=$(printf "%d" "$1")
    len=${#n}
    for ((i=0; i<len; i++)); do
      if [[ $i -gt 0 && $(( (len - i) % 3 )) -eq 0 ]]; then out+=","; fi
      out+="${n:$i:1}"
    done
    printf "%s" "$out"
  }

  fmt_loc_diff() {
    local new=$1 old=$2
    if [[ $new -gt $old ]];   then printf " (+%s)" "$(fmt_num $((new - old)))"
    elif [[ $new -lt $old ]]; then printf " (-%s)" "$(fmt_num $((old - new)))"
    fi
  }

  pct() {
    # Integer percentage: pct numerator denominator
    [[ "$2" -eq 0 ]] && { printf "0"; return; }
    printf "%d" $(( ($1 * 100 + $2 / 2) / $2 ))
  }

  # Max crate name length (for sub-crate alignment)
  max_name_len=$(jq -r '
    [.ethrex_crates[] | select(.[0] != "l2" and .[0] != "vm") | .[0] | length] | max
  ' "$LOC_JSON")

  num_width=7  # right-aligns formatted numbers; fits up to "999,999"

  fmt_top_row() {
    local label=$1 new=$2 old=$3 denom=$4
    # Use wc -m (char count, not byte count) to pad correctly despite multi-byte •
    local char_len pad padded
    char_len=$(printf "%s" "$label" | wc -m | tr -d ' ')
    pad=$(( 6 - char_len ))
    padded=$(printf "%s%${pad}s" "$label" "")
    printf "%s  %${num_width}s (%2d%%)%s" \
      "$padded" \
      "$(fmt_num "$new")" \
      "$(pct "$new" "$denom")" \
      "$(fmt_loc_diff "$new" "$old")"
  }

  fmt_sub_row() {
    local name=$1 new=$2 old=$3 denom=$4
    printf "  • %-${max_name_len}s  %${num_width}s (%2d%%)%s" \
      "$name" \
      "$(fmt_num "$new")" \
      "$(pct "$new" "$denom")" \
      "$(fmt_loc_diff "$new" "$old")"
  }

  # L1 sub-crates: everything except l2 and vm (those are L2 and LEVM)
  # Build as array of lines for proper newline handling
  l1_crates_text=""
  while IFS=$'\t' read -r crate_name crate_loc; do
    if [[ -f "$LOC_OLD_JSON" ]]; then
      old_crate_loc=$(jq -r --arg n "$crate_name" \
        '(.ethrex_crates[] | select(.[0] == $n) | .[1]) // 0' \
        "$LOC_OLD_JSON" 2>/dev/null || echo 0)
    else
      old_crate_loc="$crate_loc"
    fi
    [[ -z "$old_crate_loc" || "$old_crate_loc" == "null" ]] && old_crate_loc=0
    l1_crates_text+="$(fmt_sub_row "$crate_name" "$crate_loc" "$old_crate_loc" "$loc_l1")"$'\n'
  done < <(jq -r '
    .ethrex_crates[]
    | select(.[0] != "l2" and .[0] != "vm")
    | [.[0], (.[1] | tostring)] | @tsv
  ' "$LOC_JSON")

  loc_text="$(fmt_top_row "Total" "$loc_total" "$loc_old_total" "$loc_total")"$'\n'
  loc_text+="$(fmt_top_row "• L1" "$loc_l1" "$loc_old_l1" "$loc_total")"$'\n'
  loc_text+="${l1_crates_text}"
  loc_text+="$(fmt_top_row "• L2" "$loc_l2" "$loc_old_l2" "$loc_total")"$'\n'
  loc_text+="$(fmt_top_row "• LEVM" "$loc_levm" "$loc_old_levm" "$loc_total")"
fi

BASE_URL="${PERF_PROMETHEUS_URL:-${PROMETHEUS_URL:-}}"
if [[ -z "$BASE_URL" ]]; then
  echo "Set PERF_PROMETHEUS_URL to build the Prometheus query endpoint." >&2
  exit 1
fi
QUERY_URL="${BASE_URL%/}/api/v1/query"
RANGE="${PERF_PROMETHEUS_RANGE:-24h}"

# Build auth args
auth_args=()
_bearer="${PERF_PROMETHEUS_BEARER_TOKEN:-${PROMETHEUS_BEARER_TOKEN:-}}"
_basic="${PERF_PROMETHEUS_BASIC_AUTH:-${PROMETHEUS_BASIC_AUTH:-}}"
if [[ -n "$_bearer" ]]; then auth_args+=(-H "Authorization: Bearer $_bearer"); fi
if [[ -n "$_basic"  ]]; then auth_args+=(-u "$_basic"); fi

prometheus_query() {
  local query="$1"
  # POST: the generated queries run to tens of KB, past common URL length limits.
  curl -sS "$QUERY_URL" "${auth_args[@]}" --data-urlencode "query=$query"
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

# --- Fair per-block numbers ---
# Every node is fronted by its own Lighthouse, which times each block's engine_newPayload the same
# way for every client. Block time is that duration; throughput is the block's gas divided by it.
# No client measures itself, so the numbers are comparable across clients.
#
# PERF_NODES lists "display-name=host" pairs. Each host runs Lighthouse metrics on :5054.
PERF_NODES="${PERF_NODES:-ethrex=ethrex-mainnet-2 reth=reth-mainnet-1 nethermind=nethermind-mainnet-1}"
read -r -a nodes <<<"$PERF_NODES"
# Per-slot values use 12 s windows aligned to slot starts. Mainnet's genesis lands 11 s after a
# multiple of 12, so the windows end 1 s before Prometheus' 12 s-aligned evaluation points.
PERF_SLOT_OFFSET="${PERF_SLOT_OFFSET:-1}"
# The gas of each slot's block is a chain fact: the median of the fleet's last-block gas gauges
# (every node that follows the chain, reported or not), so a node one block behind in a slot cannot
# skew it. The list is separate from PERF_NODES because it reads each client's own gauge, set when
# the block is imported; the metrics exporter's gauge is polled and lags it by up to a slot.
PERF_GAS_SOURCES="${PERF_GAS_SOURCES:-$(cat <<EOF
last_over_time(gas_used{instance="ethrex-mainnet-2:3701"}[12s] offset ${PERF_SLOT_OFFSET}s)
or last_over_time(gas_used{instance="geth-mainnet-1:3701"}[12s] offset ${PERF_SLOT_OFFSET}s)
or last_over_time(reth_consensus_engine_beacon_new_payload_total_gas_last{instance="reth-mainnet-1:6060"}[12s] offset ${PERF_SLOT_OFFSET}s)
or last_over_time(nethermind_gas_used{instance="nethermind-mainnet-1:6060"}[12s] offset ${PERF_SLOT_OFFSET}s)
EOF
)}"
# A slot with fewer than three gauges has no gas: with two, the median would average a stale value in.
SLOT_GAS="(quantile(0.5, ${PERF_GAS_SOURCES}) and on() (count(${PERF_GAS_SOURCES}) >= 3))"
OFF_A="offset ${PERF_SLOT_OFFSET}s"
OFF_B="offset $((PERF_SLOT_OFFSET + 12))s"

slot_delta() { printf '(%s %s - %s %s)' "$1" "$OFF_A" "$1" "$OFF_B"; } # slot_delta SERIES: its increase over the slot

# Slot-aligned newPayload duration (s) of one node: Δsum/Δcount over the slot. A slot is kept only if
# every call in it returned VALID: Lighthouse also times calls that failed (refused at once, or cut
# at its 8 s timeout) and SYNCING/ACCEPTED/INVALID replies, none of which imported the block.
slot_time() {
  local sel="method=\"new_payload\",instance=\"$1:5054\"" ds dc dv
  ds=$(slot_delta "execution_layer_request_times_sum{${sel}}")
  dc=$(slot_delta "execution_layer_request_times_count{${sel}}")
  dv=$(slot_delta "execution_layer_payload_status{${sel},status=\"valid\"}")
  printf '((%s / %s) and (%s >= 1) and on(instance) (%s == ignoring(status) %s))' "$ds" "$dc" "$dc" "$dv" "$dc"
}

tag() { # tag EXPR NAME STAT: label the series with client=NAME and stat=STAT
  printf 'label_replace(label_replace(%s, "client", "%s", "", ""), "stat", "%s", "", "")' "$1" "$2" "$3"
}

bt_parts=()
tput_parts=()
for pair in "${nodes[@]}"; do
  name="${pair%%=*}"; host="${pair#*=}"
  t="$(slot_time "$host")"
  # Block time (ms) over the per-slot durations.
  bt_parts+=("$(tag "avg_over_time((1000 * ${t})[${RANGE}:12s])" "$name" mean)")
  bt_parts+=("$(tag "quantile_over_time(0.5, (1000 * ${t})[${RANGE}:12s])" "$name" p50)")
  bt_parts+=("$(tag "quantile_over_time(0.99, (1000 * ${t})[${RANGE}:12s])" "$name" p99)")
  # Throughput (Ggas/s) over the slots the node processed that have gas: gas-weighted = Σ block gas /
  # Σ newPayload time; median = median over slots of block gas / newPayload time. g is the slot's gas
  # matched by vector, not scalar(), so a slot without gas is dropped instead of making the sum NaN.
  g="((${t} > bool 0) * on() group_left() ${SLOT_GAS})"
  tput_parts+=("$(tag "sum_over_time(${g}[${RANGE}:12s]) / sum_over_time((${t} and on() ${SLOT_GAS})[${RANGE}:12s]) / 1e9" "$name" mean)")
  tput_parts+=("$(tag "quantile_over_time(0.5, (${g} / ${t} / 1e9)[${RANGE}:12s])" "$name" p50)")
done
join_or() { local IFS=; printf '%s' "${1}"; shift; for p in "$@"; do printf ' or %s' "$p"; done; }
BLOCK_TIME_QUERY="${BLOCK_TIME_PROMETHEUS_QUERY:-$(join_or "${bt_parts[@]}")}"
PERF_QUERY="${PERF_PROMETHEUS_QUERY:-$(join_or "${tput_parts[@]}")}"

block_time_response=$(prometheus_query "$BLOCK_TIME_QUERY")
check_response "$block_time_response" "block time"

perf_response=$(prometheus_query "$PERF_QUERY")
check_response "$perf_response" "throughput"

# --- Version queries ---
# ethrex exposes ethrex_info with version/branch/commit labels; every client answers web3_clientVersion.
version_response=$(prometheus_query "eth_exe_web3_client_version")
ethrex_info_response=$(prometheus_query 'ethrex_info')

node_version() {
  local host="$1" info ver branch commit
  info=$(jq -c --arg inst "${host}:3701" '.data.result[] | select(.metric.instance == $inst) | .metric' <<<"$ethrex_info_response" 2>/dev/null | head -1)
  if [[ -n "$info" ]]; then
    ver=$(jq -r '.version // ""' <<<"$info"); branch=$(jq -r '.branch // ""' <<<"$info"); commit=$(jq -r '.commit // ""' <<<"$info")
    if [[ -n "$ver" && -n "$commit" ]]; then
      # A detached build reports branch HEAD; the commit identifies it.
      if [[ -n "$branch" && "$branch" != "HEAD" ]]; then printf 'v%s-%s-%s' "$ver" "$branch" "${commit:0:8}"; else printf 'v%s-%s' "$ver" "${commit:0:8}"; fi
      return
    fi
  fi
  # Version portion of "Client/v1.2.3/platform/..."
  jq -r --arg prefix "${host}:" '
    .data.result[] | select(.metric.instance | startswith($prefix)) | .metric.version // "unknown" | split("/")[1] // "unknown"
  ' <<<"$version_response" 2>/dev/null | head -1
}

# --- Parse the responses into "client|stat|value" rows (bash 3.2 compatible, no associative arrays) ---
# A non-finite value counts as missing.
parse_rows() {
  jq -r '
    .data.result[]
    | select(.value[1] | IN("NaN", "+Inf", "-Inf") | not)
    | [(.metric.client // "series"), (.metric.stat // "mean"), .value[1]]
    | join("|")
  ' <<<"$1"
}
bt_rows=$(parse_rows "$block_time_response")
tput_rows=$(parse_rows "$perf_response")

stat_of() { # stat_of ROWS CLIENT STAT -> value or empty
  printf '%s\n' "$1" | awk -F'|' -v c="$2" -v s="$3" '$1 == c && $2 == s { print $3; exit }'
}

name_width=0
for pair in "${nodes[@]}"; do name="${pair%%=*}"; (( ${#name} > name_width )) && name_width=${#name}; done

# "name=host" pairs of the nodes with a mean, sorted by it; a node without one is reported on stderr.
order_nodes() { # order_nodes ROWS SORT_FLAG WHAT
  local pair v
  for pair in "${nodes[@]}"; do
    v=$(stat_of "$1" "${pair%%=*}" mean)
    if [[ -n "$v" ]]; then echo "$v $pair"; else echo "WARNING: no $3 for ${pair%%=*} (${pair#*=}), left out of the report" >&2; fi
  done | LC_ALL=C sort "$2"
}
# Block time ascending by mean, throughput descending by gas-weighted mean.
bt_order=()
while read -r _val pair; do bt_order+=("$pair"); done < <(order_nodes "$bt_rows" -n "block time")
tput_order=()
while read -r _val pair; do tput_order+=("$pair"); done < <(order_nodes "$tput_rows" -rn "throughput")

fmt_stat() { # fmt_stat WIDTH VALUE: VALUE with two decimals, or n/a when missing
  if [[ -n "$2" ]]; then printf "%${1}.2f" "$2"; else printf "%${1}s" "n/a"; fi
}
fmt_bt_row() {
  printf "%${name_width}s: %s ms (mean) | %s ms (p50) | %s ms (p99)\n" "$1" \
    "$(fmt_stat 7 "$(stat_of "$bt_rows" "$1" mean)")" \
    "$(fmt_stat 7 "$(stat_of "$bt_rows" "$1" p50)")" \
    "$(fmt_stat 8 "$(stat_of "$bt_rows" "$1" p99)")"
}
fmt_tput_row() {
  printf "%${name_width}s: %s Ggas/s (gas-weighted) | %s Ggas/s (median block)\n" "$1" \
    "$(fmt_stat 5 "$(stat_of "$tput_rows" "$1" mean)")" \
    "$(fmt_stat 5 "$(stat_of "$tput_rows" "$1" p50)")"
}

bt_table=""
for pair in "${bt_order[@]}"; do bt_table+="$(fmt_bt_row "${pair%%=*}")"$'\n'; done
tput_table=""
for pair in "${tput_order[@]}"; do tput_table+="$(fmt_tput_row "${pair%%=*}")"$'\n'; done

# "Comparing ..." line in block time order, with each node's version
comparing_line=""
for pair in "${bt_order[@]}"; do
  ver=$(node_version "${pair#*=}"); : "${ver:=unknown}"
  comparing_line+="${pair%%=*} (${ver}), "
done
comparing_line="${comparing_line%, }"

header_text="Daily ethrex report"
perf_title="Comparative performance report (${RANGE})"
perf_method="Measured by each node's Lighthouse: wall-clock of engine_newPayload per block, the same way for every client. Throughput = block gas / that time."

# --- Generate text report for GitHub ---
{
  echo "# ${header_text}"
  echo

  if [[ -n "$loc_text" ]]; then
    echo "## Lines of code"
    echo
    printf "%s\n" "$loc_text"
    echo
  fi

  echo "## ${perf_title}"
  echo
  echo "${perf_method}"
  echo
  echo "Comparing ${comparing_line}"
  echo

  echo "### Block Time"
  echo
  printf "%s" "$bt_table"
  echo

  echo "### Throughput"
  echo
  printf "%s" "$tput_table"
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

slack_text+="*${perf_title}*"$'\n'
slack_text+="${perf_method}"$'\n'
slack_text+="Comparing ${comparing_line}"$'\n\n'

slack_text+="*Block Time*"$'\n'
slack_text+='```'$'\n'
slack_text+="${bt_table}"
slack_text+='```'$'\n\n'

slack_text+="*Throughput*"$'\n'
slack_text+='```'$'\n'
slack_text+="${tput_table}"
slack_text+='```'$'\n'

jq -n --arg header "$header_text" --arg text "$slack_text" '{
  "blocks": [
    { "type": "header", "text": { "type": "plain_text", "text": $header } },
    { "type": "section", "text": { "type": "mrkdwn", "text": $text } }
  ]
}' >"${OUTPUT_DIR}/daily_report_slack.json"

# --- Generate Telegram HTML report ---
# Uses <pre> for code sections (monospace) and <b> for bold headers

# Escape HTML special characters for Telegram's HTML parser
escape_html() {
  local s="$1"
  s="${s//&/&amp;}"
  s="${s//</&lt;}"
  s="${s//>/&gt;}"
  printf "%s" "$s"
}

tg_text="<b>${header_text}</b>"$'\n\n'

if [[ -n "$loc_text" ]]; then
  tg_text+="<b>Lines of code</b>"$'\n'
  tg_text+="<pre>"$'\n'
  tg_text+="${loc_text}"$'\n'
  tg_text+="</pre>"$'\n\n'
fi

tg_text+="<b>${perf_title}</b>"$'\n'
tg_text+="$(escape_html "$perf_method")"$'\n'
tg_text+="Comparing $(escape_html "$comparing_line")"$'\n\n'

tg_text+="<b>Block Time</b>"$'\n'
tg_text+="<pre>"$'\n'
tg_text+="${bt_table}"
tg_text+="</pre>"$'\n\n'

tg_text+="<b>Throughput</b>"$'\n'
tg_text+="<pre>"$'\n'
tg_text+="${tput_table}"
tg_text+="</pre>"

printf "%s" "$tg_text" >"${OUTPUT_DIR}/daily_report_telegram.txt"
