#!/usr/bin/env python3
"""
Generates docker-compose.yaml, docker-compose.override.yaml, and .env
for N ethrex nodes on an isolated bridge network.

Usage: python3 generate.py [num_nodes]   (default: 50)
"""
import subprocess, sys, os, pathlib, textwrap

NUM_NODES = int(sys.argv[1]) if len(sys.argv) > 1 else 50
BASE_IP = "10.55.0"
IP_OFFSET = 10  # node1 = .10, node2 = .11, ...

if NUM_NODES + IP_OFFSET > 254:
    print(f"Error: max {254 - IP_OFFSET} nodes in a /24 subnet")
    sys.exit(1)

HERE = pathlib.Path(__file__).parent
NODEKEYS = HERE / "nodekeys"
NODEKEYS.mkdir(exist_ok=True)

# ── 1. Generate keys ──────────────────────────────────────────────
pubkeys = {}
for i in range(1, NUM_NODES + 1):
    pem = NODEKEYS / f"node{i}.pem"
    raw = NODEKEYS / f"node{i}.key"

    if not pem.exists():
        subprocess.run(
            ["openssl", "ecparam", "-name", "secp256k1", "-genkey", "-noout", "-out", str(pem)],
            check=True, capture_output=True,
        )

    if not raw.exists():
        # Extract raw 32-byte private key
        text = subprocess.run(
            ["openssl", "ec", "-in", str(pem), "-text", "-noout"],
            check=True, capture_output=True, text=True,
        ).stdout
        lines = []
        capture = False
        for line in text.splitlines():
            if line.strip().startswith("priv:"):
                capture = True
                continue
            if line.strip().startswith("pub:"):
                break
            if capture:
                lines.append(line.strip().replace(":", ""))
        hex_key = "".join(lines)
        raw.write_bytes(bytes.fromhex(hex_key))

    # Derive public key
    der = subprocess.run(
        ["openssl", "ec", "-in", str(pem), "-pubout", "-outform", "DER"],
        check=True, capture_output=True,
    ).stdout
    # Last 65 bytes = 04 || x || y; strip 04
    uncompressed = der[-65:]
    assert uncompressed[0] == 0x04, f"Expected 04 prefix, got {uncompressed[0]:02x}"
    pubkeys[i] = uncompressed[1:].hex()
    assert len(pubkeys[i]) == 128, f"node{i}: got {len(pubkeys[i])} hex chars"

print(f"Generated keys for {NUM_NODES} nodes")

# ── 2. Build enode URLs and bootnodes ─────────────────────────────
def ip(n):
    return f"{BASE_IP}.{IP_OFFSET + n - 1}"

def enode(n):
    return f"enode://{pubkeys[n]}@{ip(n)}:30303"

# Each node gets up to 10 bootnodes (a random subset of others).
# Using all others would make the .env enormous for 50 nodes.
import random
random.seed(42)  # reproducible

def bootnodes_for(n):
    others = [i for i in range(1, NUM_NODES + 1) if i != n]
    # Give each node min(10, len(others)) bootnodes
    selected = random.sample(others, min(10, len(others)))
    return ",".join(enode(i) for i in sorted(selected))

# ── 3. Write .env ─────────────────────────────────────────────────
env_lines = []
for i in range(1, NUM_NODES + 1):
    env_lines.append(f"BOOTNODES_FOR_NODE{i}={bootnodes_for(i)}")
(HERE / ".env").write_text("\n".join(env_lines) + "\n")

# ── 4. Write docker-compose.yaml ──────────────────────────────────
services = []

for i in range(1, NUM_NODES + 1):
    http_port = 8544 + i
    # node1 gets a healthcheck so the monitor can depend on it being up.
    # Use pidof since the minimal image has no curl/ss/bash.
    healthcheck = ""
    if i == 1:
        healthcheck = """
    healthcheck:
      test: ["CMD", "pidof", "ethrex"]
      interval: 2s
      timeout: 2s
      retries: 30
      start_period: 3s"""

    svc = f"""  node{i}:
    build:
      context: ../..
      dockerfile: Dockerfile
    entrypoint: ["/bin/sh", "/entrypoint.sh"]
    command:
      - --network
      - /genesis.json
      - --p2p.addr
      - "0.0.0.0"
      - --p2p.port
      - "30303"
      - --discovery.port
      - "30303"
      - --http.port
      - "{http_port}"
      - --datadir
      - /data
      - --bootnodes
      - "${{BOOTNODES_FOR_NODE{i}}}"
    volumes:
      - ./genesis.json:/genesis.json:ro
      - ./entrypoint.sh:/entrypoint.sh:ro
      - ./nodekeys/node{i}.key:/node.key:ro
    networks:
      discovery-net:
        ipv4_address: {ip(i)}{healthcheck}
    deploy:
      resources:
        limits:
          memory: 256M"""
    services.append(svc)

# Monitor sidecar shares node1's network namespace so it can sniff node1's
# traffic. A healthcheck on node1 prevents the race where monitor starts
# before node1's netns exists.
services.append("""  monitor:
    image: nicolaka/netshoot:latest
    cap_add:
      - NET_ADMIN
      - NET_RAW
    network_mode: "service:node1"
    volumes:
      - ./monitor.sh:/monitor.sh:ro
    entrypoint: ["sleep", "infinity"]
    depends_on:
      node1:
        condition: service_healthy
    restart: on-failure""")

# Flooder: sends crafted Neighbors packets with fake nodes to node1,
# triggering the 1:16 ping amplification. Uses the host-built binary
# mounted into the ethrex image (avoids Docker DNS issues for apt-get).
# Build first: bash run-flooder.sh --build-only
# Start with: docker compose --profile flood up -d flooder
services.append(f"""  flooder:
    profiles: ["flood"]
    image: ubuntu:24.04
    entrypoint: ["/flooder"]
    command: ["{ip(1)}", "30303", "200"]
    volumes:
      - ../../target/release/packet-storm-flooder:/flooder:ro
    networks:
      discovery-net:
        ipv4_address: {BASE_IP}.253
    depends_on:
      node1:
        condition: service_healthy
    deploy:
      resources:
        limits:
          memory: 128M""")

compose = f"""## Auto-generated: {NUM_NODES} ethrex nodes for packet-storm reproduction.
## Re-generate with: python3 generate.py {NUM_NODES}

services:
{chr(10).join(services)}

networks:
  discovery-net:
    driver: bridge
    ipam:
      config:
        - subnet: {BASE_IP}.0/24
"""

(HERE / "docker-compose.yaml").write_text(compose)

# Remove override file (no longer needed, volumes are inline)
override = HERE / "docker-compose.override.yaml"
if override.exists():
    override.unlink()

print(f"Written: docker-compose.yaml ({NUM_NODES} nodes), .env")
print(f"Node IPs: {ip(1)} .. {ip(NUM_NODES)}")
print(f"Monitor attached to node1 ({ip(1)})")
