#!/usr/bin/env python3
"""Reproducer: two keyed frame transactions from a contract sender, admitted then dropped.

This is EIP-8250 concurrency's happy path — two frame transactions from one contract sender
on disjoint nonce keys — and it usually works: both are admitted and mine in the SAME block.
Intermittently (roughly one run in three, seen on a chain minutes old) both are admitted,
`eth_sendRawTransaction` returns their hashes, and neither ever enters the pool. The probe
distinguishes the two outcomes by polling `eth_getTransactionByHash` alongside
`txpool_status`, so a slow builder does not read as a drop.

The transactions do not execute: their recipients stay unfunded and the sender contract's
balance is unchanged. No log line accompanies the drop.

Usage: HEGOTA_SENDER_KEY=<hex> probe_contract_sender_tx.py <rpc> <authrpc> <jwt>
"""
import json, pathlib, sys, time

spec_path = str(pathlib.Path(__file__).with_name("verify_devnet.py"))
src = open(spec_path).read().replace("sys.exit(main())", "pass")
mod = {}
sys.argv = ["v", sys.argv[1], sys.argv[2], sys.argv[3]]
exec(compile(src, "verify", "exec"), mod)

rpc, fees, deploy, frame, build = (mod["rpc"], mod["fees"], mod["deploy"],
                                   mod["frame"], mod["build_frame_tx"])
RPC = mod["RPC"]
chain_id = int(rpc(RPC, "eth_chainId", []), 16)
mod["drain_pool"]("the probe")

contract = deploy(mod["SENDER_CONTRACT_RUNTIME"], 10**18)
print("contract", contract, "balance",
      int(rpc(RPC, "eth_getBalance", [contract, "latest"]), 16) / 1e18, "ETH",
      "nonce", int(rpc(RPC, "eth_getTransactionCount", [contract, "latest"]), 16))

hashes = []
for i in (0, 1):
    raw = build(chain_id, contract, 0x9999_0000 + i, 0,
                [frame(1, 0x03, contract, 80_000, 0, 0, b""),
                 frame(2, 0x00, mod["derived_address"]("f00d", i), 30_000,
                       mod["NEW_ACCOUNT_STATE_GAS"], 100, b"")],
                *fees(), None, sign=False)
    h = rpc(RPC, "eth_sendRawTransaction", [raw])
    print(f"  key {hex(0x9999_0000 + i)} admitted {h}")
    hashes.append(h)

for t in range(40):
    time.sleep(3)
    pool = rpc(RPC, "txpool_status", [])
    known = [bool(rpc(RPC, "eth_getTransactionByHash", [h])) for h in hashes]
    mined = [bool(rpc(RPC, "eth_getTransactionReceipt", [h])) for h in hashes]
    if t % 3 == 0 or all(mined):
        print(f"  t+{t*3:3}s pool={pool} known={known} mined={mined}")
    if all(mined) or not any(known):
        break
for h in hashes:
    r = rpc(RPC, "eth_getTransactionReceipt", [h])
    print(h, "->", f"block {int(r['blockNumber'],16)} status {r['status']}" if r
          else ("STILL KNOWN, unmined" if rpc(RPC, "eth_getTransactionByHash", [h]) else "GONE"))
