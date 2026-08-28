#!/usr/bin/env bash
# Keep the frames testnet's beacon nodes peered with each other.
#
# Why this exists: the nodes advertise `port_publisher.nat_exit_ip`, which on a
# host behind NAT is the router's address. If the router does not forward the
# CL ports back in, nothing can reach them there -- including the sibling nodes
# on this same host. Discovery then fails, and the deployment silently splits
# into one chain per node, each finalising only if it happens to hold 2/3 of
# the validators.
#
# The fix is to peer them over the docker network by address instead. Prysm's
# trusted-peer list is in-memory, so a container restart drops it; this runs on
# a timer and is idempotent, which makes a restart self-healing.
#
# Once the router forwards the CL ports (INSTALL.md section 10) this stops
# being load-bearing, but it stays correct and costs nothing.
set -uo pipefail

declare -A REST=()   # container -> host REST port
declare -A ADDR=()   # container -> /ip4/<container ip>/tcp/<p2p tcp>/p2p/<peer id>

for c in $(docker ps --format '{{.Names}}' | grep -E '^cl-[0-9]+-prysm' | sort); do
    ip=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$c")
    port=$(docker port "$c" 3500/tcp 2>/dev/null | head -1 | cut -d: -f2)
    tcp=$(docker inspect -f '{{join .Args " "}}' "$c" | tr ' ' '\n' \
          | grep -oE 'p2p-tcp-port=[0-9]+' | cut -d= -f2)
    [ -n "$ip" ] && [ -n "$port" ] && [ -n "$tcp" ] || continue
    id=$(curl -s --max-time 5 "http://127.0.0.1:${port}/eth/v1/node/identity" \
         | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["peer_id"])' 2>/dev/null)
    [ -n "$id" ] || continue
    REST["$c"]="$port"
    ADDR["$c"]="/ip4/${ip}/tcp/${tcp}/p2p/${id}"
done

[ "${#ADDR[@]}" -ge 2 ] || { echo "fewer than two beacon nodes resolved; nothing to do"; exit 0; }

for target in "${!REST[@]}"; do
    for peer in "${!ADDR[@]}"; do
        [ "$peer" = "$target" ] && continue
        curl -s -o /dev/null --max-time 5 -X POST -H 'Content-Type: application/json' \
            --data "{\"addr\":\"${ADDR[$peer]}\"}" \
            "http://127.0.0.1:${REST[$target]}/prysm/node/trusted_peers"
    done
done
