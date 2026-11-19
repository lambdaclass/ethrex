"""Export daisugi-devnet-0 blocks from a JSON-RPC endpoint as RLP, one hex block per line.

Usage:
    python3 -I scripts/daisugi-devnet-0/export_blocks.py OUT_FILE [FIRST] [LAST] [RPC_URL]

Defaults: blocks 1 to the endpoint's head, from https://rpc.daisugi-devnet-0.ethpandaops.io/.
The public RPC does not serve `debug_getRawBlock`, so each block is rebuilt from
`eth_getBlockByNumber` with full transactions: the header (21 Prague fields, or 25 after
the Daisugi fork), the transactions (legacy, EIP-2930, EIP-1559 and EIP-8141 frame
transactions), the withdrawals and, after the fork, the block proof. Every rebuilt
transaction must hash to its published hash and every header to its block hash, or the
export stops. The replay test in `test/tests/blockchain/daisugi_devnet_replay_tests.rs`
imports the result.

Known quirk: the reference client's JSON reports a keyed frame transaction's account
nonce in `nonce`, not always its `nonce_seq`, so a mismatching frame transaction is
retried with the sequence values 0 to 3 before giving up.
"""
import json, subprocess, sys, urllib.request

RPC = sys.argv[4] if len(sys.argv) > 4 else "https://rpc.daisugi-devnet-0.ethpandaops.io/"


def call(body):
    req = urllib.request.Request(RPC, data=json.dumps(body).encode(), headers={"content-type": "application/json", "User-Agent": "Mozilla/5.0"})
    return json.load(urllib.request.urlopen(req, timeout=120))


def b(h):
    h = h[2:] if h.startswith("0x") else h
    return bytes.fromhex(h if len(h) % 2 == 0 else "0" + h)


def uint(h):
    return b(h).lstrip(b"\x00")


def enc_bytes(x):
    if len(x) == 1 and x[0] < 0x80:
        return x
    if len(x) <= 55:
        return bytes([0x80 + len(x)]) + x
    l = len(x).to_bytes((len(x).bit_length() + 7) // 8, "big")
    return bytes([0xB7 + len(l)]) + l + x


def enc_list(items):
    p = b"".join(items)
    if len(p) <= 55:
        return bytes([0xC0 + len(p)]) + p
    l = len(p).to_bytes((len(p).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(l)]) + l + p


def keccak(data):
    return subprocess.run(["cast", "keccak", "0x" + data.hex()], capture_output=True, text=True).stdout.strip()


def access_list(tx):
    return enc_list([enc_list([enc_bytes(b(e["address"])), enc_list([enc_bytes(b(k)) for k in e["storageKeys"]])]) for e in tx.get("accessList") or []])


def to_field(tx):
    return enc_bytes(b(tx["to"]) if tx.get("to") else b"")


def frame_tx(tx, nonce):
    frames = [enc_list([enc_bytes(uint(f["mode"])), enc_bytes(uint(f["flags"])), enc_bytes(b(f["target"]) if f.get("target") else b""),
                        enc_list([enc_bytes(uint(f["executionGas"])), enc_bytes(uint(f["stateGas"]))]),
                        enc_bytes(uint(f["value"])), enc_bytes(b(f["data"]))]) for f in tx["frames"]]
    sigs = [enc_list([enc_bytes(uint(s["scheme"])), enc_bytes(b(s["signer"]) if s.get("signer") else b""), enc_bytes(b(s["msg"])), enc_bytes(b(s["signature"]))]) for s in tx["signatures"]]
    items = [enc_bytes(uint(tx["chainId"]))]
    if tx.get("nonceKeys") is not None:
        items.append(enc_list([enc_bytes(uint(k)) for k in tx["nonceKeys"]]))
    items += [enc_bytes(uint(nonce)), enc_bytes(b(tx["from"])), enc_list(frames), enc_list(sigs),
              enc_list([enc_bytes(uint(tx["maxFee"])), enc_bytes(uint(tx["maxPriorityFeePerGas"]))]),
              enc_list([enc_bytes(b(h)) for h in tx["blobVersionedHashes"]])]
    return b"\x06" + enc_list(items)


def typed_or_legacy(tx):
    """The transaction's EIP-2718 bytes, and how it sits in a block body."""
    t = int(tx["type"], 16)
    if t == 0:
        raw = enc_list([enc_bytes(uint(tx["nonce"])), enc_bytes(uint(tx["gasPrice"])), enc_bytes(uint(tx["gas"])), to_field(tx),
                        enc_bytes(uint(tx["value"])), enc_bytes(b(tx["input"])), enc_bytes(uint(tx["v"])), enc_bytes(uint(tx["r"])), enc_bytes(uint(tx["s"]))])
        return raw, raw
    if t == 1:
        raw = b"\x01" + enc_list([enc_bytes(uint(tx["chainId"])), enc_bytes(uint(tx["nonce"])), enc_bytes(uint(tx["gasPrice"])), enc_bytes(uint(tx["gas"])), to_field(tx),
                                  enc_bytes(uint(tx["value"])), enc_bytes(b(tx["input"])), access_list(tx), enc_bytes(uint(tx.get("yParity", tx["v"]))), enc_bytes(uint(tx["r"])), enc_bytes(uint(tx["s"]))])
    elif t == 2:
        raw = b"\x02" + enc_list([enc_bytes(uint(tx["chainId"])), enc_bytes(uint(tx["nonce"])), enc_bytes(uint(tx["maxPriorityFeePerGas"])), enc_bytes(uint(tx["maxFeePerGas"])),
                                  enc_bytes(uint(tx["gas"])), to_field(tx), enc_bytes(uint(tx["value"])), enc_bytes(b(tx["input"])), access_list(tx),
                                  enc_bytes(uint(tx.get("yParity", tx["v"]))), enc_bytes(uint(tx["r"])), enc_bytes(uint(tx["s"]))])
    elif t == 6:
        raw = None
        for nonce in [tx["nonce"]] + [hex(n) for n in range(4)]:
            candidate = frame_tx(tx, nonce)
            if keccak(candidate) == tx["hash"]:
                raw = candidate
                break
        if raw is None:
            raise SystemExit(f"frame transaction {tx['hash']} does not rebuild to its hash")
    else:
        raise SystemExit(f"transaction type {tx['type']} ({tx['hash']}) is not supported by this exporter")
    return raw, enc_bytes(raw)


HEADER = [("parentHash", 0), ("sha3Uncles", 0), ("miner", 0), ("stateRoot", 0), ("transactionsRoot", 0), ("receiptsRoot", 0), ("logsBloom", 0),
          ("difficulty", 1), ("number", 1), ("gasLimit", 1), ("gasUsed", 1), ("timestamp", 1), ("extraData", 0), ("mixHash", 0), ("nonce", 0),
          ("baseFeePerGas", 1), ("withdrawalsRoot", 0), ("blobGasUsed", 1), ("excessBlobGas", 1), ("parentBeaconBlockRoot", 0), ("requestsHash", 0)]


def block_rlp(blk):
    header_items = [enc_bytes(uint(blk[k]) if as_int else b(blk[k])) for k, as_int in HEADER]
    forked = "starkProofHash" in blk
    if forked:
        header_items += [b"\x80", b"\x80", enc_bytes(b(blk["starkProofHash"])), enc_bytes(b(blk["blockDepsHash"]))]
    header = enc_list(header_items)
    if keccak(header) != blk["hash"]:
        raise SystemExit(f"block {int(blk['number'], 16)} header does not rebuild to its hash")
    txs = []
    for tx in blk["transactions"]:
        raw, in_block = typed_or_legacy(tx)
        if keccak(raw) != tx["hash"]:
            raise SystemExit(f"transaction {tx['hash']} in block {int(blk['number'], 16)} does not rebuild to its hash")
        txs.append(in_block)
    withdrawals = [enc_list([enc_bytes(uint(w["index"])), enc_bytes(uint(w["validatorIndex"])), enc_bytes(b(w["address"])), enc_bytes(uint(w["amount"]))])
                   for w in blk.get("withdrawals") or []]
    items = [header, enc_list(txs), enc_list([]), enc_list(withdrawals)]
    if forked:
        items.append(enc_bytes(b(blk.get("starkProof") or "0x")))
    return enc_list(items)


BATCH = 10


def fetch(numbers, attempts=3):
    """Fetch a batch of blocks, retrying a response that fails to parse."""
    for attempt in range(attempts):
        try:
            return call([{"jsonrpc": "2.0", "id": n, "method": "eth_getBlockByNumber", "params": [hex(n), True]} for n in numbers])
        except (json.JSONDecodeError, OSError) as err:
            if attempt == attempts - 1:
                raise
            print(f"retrying blocks {numbers[0]}..{numbers[-1]}: {err}", file=sys.stderr)


def main():
    out = sys.argv[1]
    head = int(call({"jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": []})["result"], 16)
    first = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    last = int(sys.argv[3]) if len(sys.argv) > 3 else head
    with open(out, "w") as f:
        # Small batches: a block can carry a proof of about 250 KB, and a large batch
        # of them makes responses big enough to arrive truncated.
        for start in range(first, last + 1, BATCH):
            numbers = range(start, min(start + BATCH, last + 1))
            replies = fetch(numbers)
            for reply in sorted(replies, key=lambda r: r["id"]):
                f.write(block_rlp(reply["result"]).hex() + "\n")
            print(f"exported up to block {numbers[-1]}", file=sys.stderr)


if __name__ == "__main__":
    main()
