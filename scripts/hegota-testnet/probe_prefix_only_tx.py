#!/usr/bin/env python3
"""Reproducer: a PREFIX-ONLY frame transaction is accepted by RPC and silently dropped.

A frame transaction whose only frame is its VERIFY prefix — it approves payment and does
nothing else — simulates valid (`ethrex_simulateFrameTransaction` returns valid: true),
`eth_sendRawTransaction` returns its hash with no error, and the transaction then never
appears in the pool and never mines. `eth_getTransactionByHash` reports it unknown within
the same second, and the sender's balance is untouched, so it did not execute under some
other hash.

Deterministic: observed on every attempt, including on a chain freshly launched seconds
earlier. Whether the shape *should* be admitted is a separate question — a body-less
transaction does no work — but returning a hash for a transaction the node discards is
wrong either way: a caller has no way to learn it was dropped.

Usage: HEGOTA_SENDER_KEY=<hex> probe_prefix_only_tx.py <rpc> <authrpc> <jwt>
"""
import pathlib, sys, time
src = open(str(pathlib.Path(__file__).with_name("verify_devnet.py"))).read()
src = src.replace("sys.exit(main())", "pass")
sys.argv = ["v", sys.argv[1], sys.argv[2], sys.argv[3]]
m = {}
exec(compile(src, "verify", "exec"), m)
rpc, RPC = m["rpc"], m["RPC"]
chain_id = int(rpc(RPC, "eth_chainId", []), 16)
m["drain_pool"]("probe")

contract = m["deploy"](m["SENDER_CONTRACT_RUNTIME"], 10**18)
raw = m["build_frame_tx"](chain_id, contract, 0x7777_0001, 0,
                          [m["frame"](1, 0x03, contract, 80_000, 0, 0, b"")],
                          *m["fees"](), None, sign=False)
h = rpc(RPC, "eth_sendRawTransaction", [raw])
print("contract", contract, "\nsingle tx", h)
for t in range(20):
    time.sleep(3)
    known = bool(rpc(RPC, "eth_getTransactionByHash", [h]))
    r = rpc(RPC, "eth_getTransactionReceipt", [h])
    print(f"  t+{t*3:3}s known={known} mined={bool(r)} pool={rpc(RPC,'txpool_status',[])}")
    if r or (not known and t > 1):
        break
