#!/usr/bin/env bash
# server-status.sh — Full status report of all ethrex servers
# with fair percentile-based performance comparisons using block intersection
set -euo pipefail
export LC_ALL=C

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

SSH_OPTS="-o ConnectTimeout=5 -o BatchMode=yes -o StrictHostKeyChecking=accept-new"

# ── Data collection ─────────────────────────────────────────────────
# Single SSH per server: git info, PIDs, BLOCK metrics
fetch_server() {
    local name=$1 host=$2

    if ! ssh $SSH_OPTS "$host" bash <<'REMOTE' > "$WORK/${name}_all.txt" 2>/dev/null
cd ~/ethrex 2>/dev/null && {
    echo "BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
    echo "COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
} || {
    echo "BRANCH=unknown"
    echo "COMMIT=unknown"
}
echo "ETHREX_PID=$(pgrep -f 'target.*ethrex' | head -1 || true)"
echo "LIGHTHOUSE_PID=$(pgrep -f lighthouse | head -1 || true)"
echo "---METRICS---"
grep -P 'BLOCK \d+ \|.*Ggas/s' ~/ethrex.log 2>/dev/null || true
REMOTE
    then
        echo "STATUS=unreachable" > "$WORK/${name}_info.txt"
        touch "$WORK/${name}_raw.txt"
        return
    fi

    # Extract info fields
    grep -E '^(BRANCH|COMMIT|ETHREX_PID|LIGHTHOUSE_PID)=' "$WORK/${name}_all.txt" \
        > "$WORK/${name}_info.txt" 2>/dev/null || true

    # Parse BLOCK metrics → "epoch block_num ggas ms"
    sed -n '/^---METRICS---$/,$ p' "$WORK/${name}_all.txt" | tail -n +2 \
        | awk -F'|' '{
            ts = $1; sub(/[[:space:]].*/, "", ts)
            split(ts, dt, "T")
            split(dt[2], hms, ":")
            split(hms[3], sf, ".")
            epoch = hms[1] * 3600 + hms[2] * 60 + sf[1]

            split($1, a, "BLOCK ")
            bn = a[2] + 0

            gsub(" ", "", $2); split($2, g, "G")
            gsub(" ", "", $3); split($3, m, "m")

            print epoch, bn, g[1], m[1]
        }' > "$WORK/${name}_raw.txt" 2>/dev/null || touch "$WORK/${name}_raw.txt"
}

# ── Status detection ────────────────────────────────────────────────
detect_status() {
    local name=$1

    if grep -q "STATUS=unreachable" "$WORK/${name}_info.txt" 2>/dev/null; then
        echo "unreachable"; return
    fi

    local ethrex_pid lighthouse_pid block_count
    ethrex_pid=$(grep 'ETHREX_PID=' "$WORK/${name}_info.txt" | cut -d= -f2 || true)
    lighthouse_pid=$(grep 'LIGHTHOUSE_PID=' "$WORK/${name}_info.txt" | cut -d= -f2 || true)
    block_count=$(wc -l < "$WORK/${name}_raw.txt" 2>/dev/null | tr -d ' ' || echo 0)

    if [ -z "$ethrex_pid" ]; then
        echo "stopped"
    elif [ -z "$lighthouse_pid" ]; then
        echo "ethrex only"
    elif [ "$block_count" -eq 0 ]; then
        echo "syncing"
    else
        echo "running"
    fi
}

# ── Steady-state detection ──────────────────────────────────────────
# Finds first inter-block gap >= 5s (end of catch-up burst)
find_steady_start() {
    local name=$1

    local result
    result=$(awk '
    NR == 1 { prev_t = $1; next }
    ($1 - prev_t) >= 5 { print $2; exit }
    { prev_t = $1 }
    ' "$WORK/${name}_raw.txt")

    if [ -z "$result" ]; then
        head -1 "$WORK/${name}_raw.txt" 2>/dev/null | awk '{print $2}'
    else
        echo "$result"
    fi
}

# ── Block intersection ──────────────────────────────────────────────
# Computes the set of blocks common to ALL running servers in a group
# Args: group_label entry1 entry2 ... (entries are "name:role")
compute_common_blocks() {
    local group=$1; shift
    local common="$WORK/common_${group}.txt"
    local first=true

    for entry in "$@"; do
        local name="${entry%%:*}"
        local status
        status=$(detect_status "$name")
        [ "$status" != "running" ] && continue

        local steady
        steady=$(find_steady_start "$name")
        [ -z "$steady" ] && continue

        # Extract unique block numbers from steady state onward
        awk -v sb="$steady" '$2 >= sb {print $2}' "$WORK/${name}_raw.txt" \
            | sort -n -u > "$WORK/${name}_blocks.txt"

        if [ "$first" = true ]; then
            cp "$WORK/${name}_blocks.txt" "$common"
            first=false
        else
            # Intersect: keep only blocks present in both files
            awk 'NR==FNR {a[$1]=1; next} $1 in a' \
                "$WORK/${name}_blocks.txt" "$common" > "$WORK/_tmp_common.txt"
            mv "$WORK/_tmp_common.txt" "$common"
        fi
    done

    if [ "$first" = true ]; then
        touch "$common"
    fi
}

# ── Stats computation ───────────────────────────────────────────────
# Output: count avg_g p50_g p95_g p99_g avg_m p50_m p95_m p99_m
compute_stats() {
    local name=$1 common_file=$2

    # Join raw data with common blocks (first occurrence per block only)
    awk 'NR==FNR {b[$1]=1; next} $2 in b && !seen[$2]++ {print $3, $4}' \
        "$common_file" "$WORK/${name}_raw.txt" > "$WORK/${name}_joined.txt"

    local n
    n=$(wc -l < "$WORK/${name}_joined.txt" | tr -d ' ')
    if [ "$n" -eq 0 ]; then
        echo "0 0 0 0 0 0 0 0 0"
        return
    fi

    # Throughput percentiles (sort by ggas ascending)
    local gstats
    gstats=$(sort -g -k1,1 "$WORK/${name}_joined.txt" | awk '{
        g[NR] = $1; s += $1
    } END {
        n = NR; avg = s / n
        i = int(n * 0.5 + 0.999999); if (i<1) i=1; if (i>n) i=n; p50 = g[i]
        i = int(n * 0.95 + 0.999999); if (i<1) i=1; if (i>n) i=n; p95 = g[i]
        i = int(n * 0.99 + 0.999999); if (i<1) i=1; if (i>n) i=n; p99 = g[i]
        printf "%.3f %.3f %.3f %.3f", avg, p50, p95, p99
    }')

    # Block time percentiles (sort by ms ascending)
    local mstats
    mstats=$(sort -g -k2,2 "$WORK/${name}_joined.txt" | awk '{
        m[NR] = $2; s += $2
    } END {
        n = NR; avg = s / n
        i = int(n * 0.5 + 0.999999); if (i<1) i=1; if (i>n) i=n; p50 = m[i]
        i = int(n * 0.95 + 0.999999); if (i<1) i=1; if (i>n) i=n; p95 = m[i]
        i = int(n * 0.99 + 0.999999); if (i<1) i=1; if (i>n) i=n; p99 = m[i]
        printf "%.0f %.0f %.0f %.0f", avg, p50, p95, p99
    }')

    echo "$n $gstats $mstats"
}

# ── Format value with delta vs baseline ─────────────────────────────
format_val() {
    local val=$1 base=$2
    if [ "$base" = "0" ] || [ -z "$base" ]; then
        echo "$val"
        return
    fi
    awk -v v="$val" -v b="$base" 'BEGIN {
        if (b + 0 == 0) { print v; exit }
        d = (v - b) / b * 100
        if (d >= 0) printf "%s (+%.1f%%)", v, d
        else printf "%s (%.1f%%)", v, d
    }'
}

# ── Print overview table ────────────────────────────────────────────
print_overview() {
    local label=$1; shift

    echo "### ${label}"
    echo ""
    echo "| Server | Branch | Commit | Status |"
    echo "|--------|--------|--------|--------|"

    for entry in "$@"; do
        local name="${entry%%:*}"
        local role="${entry##*:}"

        local status
        status=$(detect_status "$name")

        local branch="—" commit="—"
        if [ "$status" != "unreachable" ]; then
            branch=$(grep 'BRANCH=' "$WORK/${name}_info.txt" 2>/dev/null | cut -d= -f2 || echo "—")
            commit=$(grep 'COMMIT=' "$WORK/${name}_info.txt" 2>/dev/null | cut -d= -f2 || echo "—")
        fi

        local srv_label="$name"
        [ "$role" = "baseline" ] && srv_label="${name} (baseline)"

        echo "| ${srv_label} | ${branch} | \`${commit}\` | ${status} |"
    done
    echo ""
}

# ── Print performance table ─────────────────────────────────────────
print_perf() {
    local label=$1; shift
    local common="$WORK/common_${label}.txt"
    local common_count
    common_count=$(wc -l < "$common" 2>/dev/null | tr -d ' ' || true)
    common_count=${common_count:-0}

    if [ "$common_count" -eq 0 ]; then
        echo "*No common blocks for performance comparison.*"
        echo ""
        return
    fi

    local first_block last_block
    first_block=$(head -1 "$common")
    last_block=$(tail -1 "$common")

    echo "**Performance** (${common_count} common blocks, ${first_block}..${last_block})"
    echo ""
    echo "| Server | Avg Ggas/s | p50 | p95 | p99 | Avg ms | p50 | p95 | p99 |"
    echo "|--------|-----------|-----|-----|-----|--------|-----|-----|-----|"

    # Find baseline stats (if group has one)
    local base_avg_g=0 base_p50_g=0 base_p95_g=0 base_p99_g=0
    local base_avg_m=0 base_p50_m=0 base_p95_m=0 base_p99_m=0
    for entry in "$@"; do
        local name="${entry%%:*}"
        local role="${entry##*:}"
        if [ "$role" = "baseline" ]; then
            local status
            status=$(detect_status "$name")
            if [ "$status" = "running" ]; then
                local bstats
                bstats=$(compute_stats "$name" "$common")
                read -r _ base_avg_g base_p50_g base_p95_g base_p99_g \
                    base_avg_m base_p50_m base_p95_m base_p99_m <<< "$bstats"
            fi
            break
        fi
    done

    # Print rows
    for entry in "$@"; do
        local name="${entry%%:*}"
        local role="${entry##*:}"
        local status
        status=$(detect_status "$name")
        [ "$status" != "running" ] && continue

        local stats
        stats=$(compute_stats "$name" "$common")
        local n avg_g p50_g p95_g p99_g avg_m p50_m p95_m p99_m
        read -r n avg_g p50_g p95_g p99_g avg_m p50_m p95_m p99_m <<< "$stats"
        [ "$n" -eq 0 ] && continue

        if [ "$role" = "baseline" ] || [ "$role" = "peer" ]; then
            local srv_label="$name"
            [ "$role" = "baseline" ] && srv_label="${name} (baseline)"
            echo "| ${srv_label} | ${avg_g} | ${p50_g} | ${p95_g} | ${p99_g} | ${avg_m} | ${p50_m} | ${p95_m} | ${p99_m} |"
        else
            # Test server: show deltas vs baseline
            echo "| ${name} | $(format_val "$avg_g" "$base_avg_g") | $(format_val "$p50_g" "$base_p50_g") | $(format_val "$p95_g" "$base_p95_g") | $(format_val "$p99_g" "$base_p99_g") | $(format_val "$avg_m" "$base_avg_m") | $(format_val "$p50_m" "$base_p50_m") | $(format_val "$p95_m" "$base_p95_m") | $(format_val "$p99_m" "$base_p99_m") |"
        fi
    done
    echo ""
}

# ── Process one group ───────────────────────────────────────────────
process_group() {
    local label=$1; shift
    print_overview "$label" "$@"
    compute_common_blocks "$label" "$@"
    print_perf "$label" "$@"
}

# ── Main ────────────────────────────────────────────────────────────

# Fetch all servers in parallel
for i in $(seq 1 10); do
    fetch_server "srv${i}" "admin@ethrex-mainnet-${i}" &
done
for i in $(seq 1 5); do
    fetch_server "office${i}" "admin@ethrex-office-${i}" &
done
wait

echo "## Server Status"
echo ""

process_group "Instance A" \
    "srv1:baseline" "srv2:test" "srv3:test" "srv4:test" "srv5:test"

process_group "Instance B" \
    "srv6:baseline" "srv7:test" "srv8:test" "srv9:test" "srv10:test"

process_group "Office" \
    "office1:peer" "office2:peer" "office3:peer" "office4:peer" "office5:peer"
