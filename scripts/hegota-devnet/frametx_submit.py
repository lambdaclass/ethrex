#!/usr/bin/env python3
"""Submit a current-format EIP-8141 self-verified transfer (type 0x06) to a node.

Self-verify layout (sender is a funded EOA):
  frame[0] = VERIFY (mode 1), target=sender, flags=0x03 (APPROVE scope EXECUTION+PAYMENT)
             -> default code checks the outer secp256k1 sig (signer==sender) and APPROVEs.
  frame[1] = SENDER (mode 2), target=recipient, value=amount -> transfer as sender.
  signatures = [secp256k1 sig over sig_hash, signer=sender]  (empty msg => elided from sig_hash)

Usage: frametx_submit.py <rpc_url> <sender_priv_hex> <recipient_hex> <amount_wei>
"""
import sys, json, time, urllib.request
from eth_keys import keys
from frametx import Frame, FrameSig, FrameTx, addr20

def rpc(url, method, params):
    req = urllib.request.Request(
        url, headers={"content-type": "application/json"},
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode())
    r = json.loads(urllib.request.urlopen(req, timeout=15).read())
    if "error" in r:
        raise RuntimeError(f"{method} -> {r['error']}")
    return r["result"]

def main():
    url, priv_hex, recip_hex, amount = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
    pk = keys.PrivateKey(bytes.fromhex(priv_hex.removeprefix("0x")))
    sender = int.from_bytes(pk.public_key.to_canonical_address(), "big")
    recipient = int(recip_hex, 16)

    chain_id = int(rpc(url, "eth_chainId", []), 16)
    nonce = int(rpc(url, "eth_getTransactionCount", [pk.public_key.to_checksum_address(), "latest"]), 16)
    blk = rpc(url, "eth_getBlockByNumber", ["latest", False])
    base_fee = int(blk.get("baseFeePerGas", "0x0"), 16)
    bal = int(rpc(url, "eth_getBalance", [pk.public_key.to_checksum_address(), "latest"]), 16)
    print(f"sender={pk.public_key.to_checksum_address()} bal={bal/1e18:.4f}ETH nonce={nonce} "
          f"chain_id={chain_id} base_fee={base_fee}")

    max_priority = 10**9          # 1 gwei
    max_fee = base_fee * 2 + max_priority

    def build(nonce_seq):
        tx = FrameTx(
            chain_id=chain_id, nonce_keys=[0], nonce_seq=nonce_seq, sender=sender,
            frames=[
                Frame(mode=1, flags=0x03, target=sender, gas_limit=80_000, value=0, data=b""),
                Frame(mode=2, flags=0, target=recipient, gas_limit=30_000, value=amount, data=b""),
            ],
            signatures=[FrameSig(FrameSig.SECP256K1, sender, b"", b"")],
            max_priority_fee=max_priority, max_fee=max_fee)
        sh = tx.sig_hash()
        s = pk.sign_msg_hash(sh)
        sig = bytes([s.v + 27]) + s.r.to_bytes(32, "big") + s.s.to_bytes(32, "big")
        tx.signatures = [FrameSig(FrameSig.SECP256K1, sender, b"", sig)]
        return tx

    # nonce_seq for key 0: try the account's regular nonce first; on mismatch the
    # node error reports the expected value.
    nonce_seq = nonce
    tx = build(nonce_seq)
    raw = "0x" + tx.raw().hex()
    print(f"sig_hash={tx.sig_hash().hex()}  raw_len={len(tx.raw())}")
    try:
        txhash = rpc(url, "eth_sendRawTransaction", [raw])
    except RuntimeError as e:
        print("SUBMIT REJECTED:", e)
        return 1
    print("submitted:", txhash)
    for _ in range(30):
        rcpt = rpc(url, "eth_getTransactionReceipt", [txhash])
        if rcpt:
            print(f"MINED block={int(rcpt['blockNumber'],16)} type={rcpt.get('type')} "
                  f"status={rcpt.get('status')}")
            print(json.dumps({k: rcpt.get(k) for k in ("type","status","from","payer","frameReceipts","gasUsed")}, indent=2)[:800])
            return 0
        time.sleep(2)
    print("not mined within timeout"); return 1

if __name__ == "__main__":
    sys.exit(main())
