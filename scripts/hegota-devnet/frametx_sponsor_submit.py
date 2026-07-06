#!/usr/bin/env python3
"""Submit a *sponsored* EIP-8141 transfer (type 0x06) whose gas is paid by a
distinct paymaster contract (payer != sender) — the canonical-paymaster
`[only_verify, pay]` shape.

Frame layout:
  frame[0] = VERIFY (mode 1), target=sender,  flags=0x02 (APPROVE_EXECUTION)
             -> the sender is a funded EOA; its default VERIFY code checks the
                outer secp256k1 sig (signer==sender) and APPROVEs EXECUTION.
  frame[1] = VERIFY (mode 1), target=sponsor, flags=0x01 (APPROVE_PAYMENT),
             data = OpenSponsor.verify() selector (0xfc735e99)
             -> the sponsor contract runs and calls APPROVE(scope=1), so the
                sponsor (P != sender) becomes the transaction's payer.
  frame[2] = SENDER (mode 2), target=recipient, value=amount -> the transfer.
  signatures = [secp256k1 over sig_hash, signer=sender]  (empty msg => elided)

The sponsor needs NO signature: it authorizes payment via its own code. Only
the sender signs. Requires the distinct-paymaster mempool grammar (the pay
frame may target P != sender); on a node without it the submit is rejected with
`VerifyTargetNotSender` and the tx must instead be included builder-direct.

The sender only needs enough ETH for the transferred `value` (not gas) — a
successful run with a gas-starved sender proves the sponsor paid.

Usage: frametx_sponsor_submit.py <rpc_url> <sender_priv_hex> <sponsor_hex> <recipient_hex> <amount_wei>
"""
import sys, json, time, urllib.request
from eth_keys import keys
from frametx import Frame, FrameSig, FrameTx

# OpenSponsor.verify() — keccak256("verify()")[:4]
SPONSOR_VERIFY_SELECTOR = bytes.fromhex("fc735e99")

# EIP-8141 APPROVE scope bits (flags bits 0-1).
APPROVE_PAYMENT = 0x01
APPROVE_EXECUTION = 0x02

def rpc(url, method, params):
    req = urllib.request.Request(
        url, headers={"content-type": "application/json"},
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode())
    r = json.loads(urllib.request.urlopen(req, timeout=15).read())
    if "error" in r:
        raise RuntimeError(f"{method} -> {r['error']}")
    return r["result"]

def main():
    url, priv_hex, sponsor_hex, recip_hex, amount = (
        sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5]))
    pk = keys.PrivateKey(bytes.fromhex(priv_hex.removeprefix("0x")))
    sender = int.from_bytes(pk.public_key.to_canonical_address(), "big")
    sponsor = int(sponsor_hex, 16)
    recipient = int(recip_hex, 16)

    chain_id = int(rpc(url, "eth_chainId", []), 16)
    nonce = int(rpc(url, "eth_getTransactionCount", [pk.public_key.to_checksum_address(), "latest"]), 16)
    blk = rpc(url, "eth_getBlockByNumber", ["latest", False])
    base_fee = int(blk.get("baseFeePerGas", "0x0"), 16)
    sender_bal = int(rpc(url, "eth_getBalance", [pk.public_key.to_checksum_address(), "latest"]), 16)
    sponsor_bal = int(rpc(url, "eth_getBalance", [sponsor_hex, "latest"]), 16)
    print(f"sender={pk.public_key.to_checksum_address()} bal={sender_bal/1e18:.6f}ETH nonce={nonce}")
    print(f"sponsor=0x{sponsor:040x} bal={sponsor_bal/1e18:.6f}ETH")
    print(f"chain_id={chain_id} base_fee={base_fee}")

    max_priority = 10**9  # 1 gwei
    max_fee = base_fee * 2 + max_priority

    def build(nonce_seq):
        tx = FrameTx(
            chain_id=chain_id, nonce_keys=[0], nonce_seq=nonce_seq, sender=sender,
            frames=[
                # exec approval by the sender (auto-approved via the outer sig).
                # The two VERIFY (prefix) frames' gas_limits plus the signature
                # verification cost must stay under MAX_VERIFY_GAS (100_000), so
                # keep them small — the exec auto-approve and the sponsor's
                # APPROVE both cost only a few thousand gas.
                Frame(mode=1, flags=APPROVE_EXECUTION, target=sender, gas_limit=20_000, value=0, data=b""),
                # payment approval by the sponsor contract (runs verify()).
                Frame(mode=1, flags=APPROVE_PAYMENT, target=sponsor, gas_limit=40_000, value=0,
                      data=SPONSOR_VERIFY_SELECTOR),
                # the actual transfer, executed as the sender.
                Frame(mode=2, flags=0, target=recipient, gas_limit=30_000, value=amount, data=b""),
            ],
            signatures=[FrameSig(FrameSig.SECP256K1, sender, b"", b"")],
            max_priority_fee=max_priority, max_fee=max_fee)
        sh = tx.sig_hash()
        s = pk.sign_msg_hash(sh)
        sig = bytes([s.v + 27]) + s.r.to_bytes(32, "big") + s.s.to_bytes(32, "big")
        tx.signatures = [FrameSig(FrameSig.SECP256K1, sender, b"", sig)]
        return tx

    # key-0 nonce_seq is the sender's regular account nonce.
    tx = build(nonce)
    raw = "0x" + tx.raw().hex()
    print(f"sig_hash={tx.sig_hash().hex()}  raw_len={len(tx.raw())}")
    try:
        txhash = rpc(url, "eth_sendRawTransaction", [raw])
    except RuntimeError as e:
        print("SUBMIT REJECTED:", e)
        print("  (a node without the distinct-paymaster grammar rejects the pay "
              "frame with VerifyTargetNotSender — deploy the item-#1 binary first.)")
        return 1
    print("submitted:", txhash)
    for _ in range(30):
        rcpt = rpc(url, "eth_getTransactionReceipt", [txhash])
        if rcpt:
            payer = rcpt.get("payer")
            print(f"MINED block={int(rcpt['blockNumber'],16)} type={rcpt.get('type')} "
                  f"status={rcpt.get('status')}")
            print(json.dumps({k: rcpt.get(k) for k in
                              ("type", "status", "from", "payer", "gasUsed")}, indent=2))
            sender_hex = pk.public_key.to_checksum_address().lower()
            if payer and payer.lower() == sponsor_hex.lower() and payer.lower() != sender_hex:
                print(f"OK: payer ({payer}) is the sponsor, not the sender — gas was sponsored.")
                return 0
            print(f"WARNING: payer={payer} is not the sponsor {sponsor_hex}")
            return 1
        time.sleep(2)
    print("not mined within timeout"); return 1

if __name__ == "__main__":
    sys.exit(main())
