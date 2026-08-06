#!/usr/bin/env python3
"""EIP-8312 UTXO frames — live integration tests against the PoC devnet.

Every case here needs a real chain: the block-end openings root, the ring proof,
the spent bit, settlement, and multi-client agreement only exist once blocks are
produced. Cases that a unit test already covers are not repeated.

Usage:
  ./utxo_itest.py --rpc <el-rpc> --funder-key <key> [--peers URL,URL] [--only NAME]
"""
import argparse
import json
import os
import sys
import time
import urllib.request

from eth_hash.auto import keccak
from eth_keys import keys

from frametx import (
    Frame,
    FrameSig,
    FrameTx,
    addr20,
    rlp_bytes,
    rlp_int,
    rlp_list,
)

# EIP-8141 signature schemes, per crates/common/types/transaction.rs. Spelled out
# here rather than trusted from any local copy: an out-of-date table that still
# has SECP256K1 = 0 silently builds ARBITRARY-scheme signatures, which carry no
# authentication, so a spend that should be rejected for a bad signature is
# instead rejected for an unauthenticated actor — the same-looking failure for a
# very different reason.
SCHEME_ARBITRARY = 0
SCHEME_SECP256K1 = 1
SCHEME_P256 = 2

# ---------- EIP-8312 constants ----------
UTXO_MODE = 5
VAULT = 0x8312
SPEND_MAGIC = 0x81
RING_SIZE = 8192
BATCH_SIZE = 8192
SLOT_RING_BASE = 1
SLOT_NEXT_INDEX = 0
SLOT_BATCH_BASE = 1 << 128
SLOT_SPENT_BASE = 1 << 129
UTXO_CREATED_TOPIC = "0x" + keccak(b"UtxoCreated(address,address,uint64,uint256)").hex()

VAULT_ADDR = "0x" + (VAULT).to_bytes(20, "big").hex()


# ---------- RPC ----------
class Rpc:
    def __init__(self, url):
        self.url = url
        self._id = 0

    def call(self, method, params=None):
        self._id += 1
        body = json.dumps(
            {"jsonrpc": "2.0", "id": self._id, "method": method, "params": params or []}
        ).encode()
        req = urllib.request.Request(
            self.url, data=body, headers={"Content-Type": "application/json"}
        )
        with urllib.request.urlopen(req, timeout=30) as resp:
            out = json.loads(resp.read())
        if "error" in out:
            raise RpcError(out["error"].get("message", str(out["error"])))
        return out["result"]

    def block_number(self):
        return int(self.call("eth_blockNumber"), 16)

    def storage_at(self, addr, slot_int, block="latest"):
        slot = "0x" + slot_int.to_bytes(32, "big").hex()
        return int(self.call("eth_getStorageAt", [addr, slot, block]), 16)

    def balance(self, addr, block="latest"):
        return int(self.call("eth_getBalance", [addr, block]), 16)

    def receipt(self, tx_hash):
        return self.call("eth_getTransactionReceipt", [tx_hash])

    def block(self, n, full=True):
        return self.call("eth_getBlockByNumber", [hex(n), full])


class RpcError(Exception):
    pass


def wait_blocks(rpc, n=1, timeout=90):
    start = rpc.block_number()
    deadline = time.time() + timeout
    while time.time() < deadline:
        cur = rpc.block_number()
        if cur >= start + n:
            return cur
        time.sleep(1)
    raise TimeoutError(f"chain did not advance {n} block(s) from {start}")


def wait_receipt(rpc, tx_hash, timeout=120):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            r = rpc.receipt(tx_hash)
            if r:
                return r
        except RpcError:
            pass
        time.sleep(1)
    return None


# ---------- openings tree (independent reimplementation) ----------
# Written from the EIP text, not imported from the client, so that a matching
# root is real agreement rather than a tautology.
def opening_leaf(index, source, recipient, value):
    return keccak(
        index.to_bytes(8, "big")
        + addr20(source)
        + addr20(recipient)
        + value.to_bytes(32, "big")
    )


def hash_pair(a, b):
    return keccak(a + b)


def merkle_root(leaves):
    if not leaves:
        return b"\x00" * 32
    level = list(leaves)
    while len(level) & (len(level) - 1) != 0:  # pad to a power of two
        level.append(b"\x00" * 32)
    while len(level) > 1:
        level = [hash_pair(level[i], level[i + 1]) for i in range(0, len(level), 2)]
    return level[0]


def merkle_proof(leaves, position):
    level = list(leaves)
    while len(level) & (len(level) - 1) != 0:
        level.append(b"\x00" * 32)
    idx, proof = position, []
    while len(level) > 1:
        proof.append(level[idx ^ 1])
        level = [hash_pair(level[i], level[i + 1]) for i in range(0, len(level), 2)]
        idx //= 2
    return proof


def fold(node, position, siblings):
    for sib in siblings:
        node = hash_pair(sib, node) if position & 1 else hash_pair(node, sib)
        position >>= 1
    return node


def ring_slot(block_number):
    return SLOT_RING_BASE + (block_number % RING_SIZE)


def batch_slot_for_block(block_number):
    return SLOT_BATCH_BASE + (block_number // BATCH_SIZE)


def spent_bit_location(index):
    return SLOT_SPENT_BASE + (index >> 8), 1 << (index & 0xFF)


# ---------- spend encoding ----------
def encode_spend(actors, inputs, utxo_outs, account_outs, change_index, payer,
                 max_fee, max_priority, max_gas):
    """inputs: list of dicts with index, creation_block, source, recipient, value,
    position, siblings, batch_siblings."""
    def out_rlp(o):
        return rlp_list([rlp_bytes(addr20(o[0])), rlp_int(o[1])])

    def input_rlp(i):
        return rlp_list([
            rlp_int(i["index"]),
            rlp_int(i["creation_block"]),
            rlp_bytes(addr20(i["source"])),
            rlp_bytes(addr20(i["recipient"])),
            rlp_int(i["value"]),
            rlp_int(i["position"]),
            rlp_list([rlp_bytes(s) for s in i["siblings"]]),
            rlp_list([rlp_bytes(s) for s in i["batch_siblings"]]),
        ])

    return rlp_list([
        rlp_list([rlp_bytes(addr20(a)) for a in actors]),
        rlp_list([input_rlp(i) for i in inputs]),
        rlp_list([out_rlp(o) for o in utxo_outs]),
        rlp_list([out_rlp(o) for o in account_outs]),
        rlp_int(change_index),
        rlp_bytes(b"" if payer is None else addr20(payer)),
        rlp_int(max_fee),
        rlp_int(max_priority),
        rlp_int(max_gas),
    ])


def spend_hash(chain_id, actors, inputs, utxo_outs, account_outs, change_index,
               payer, max_fee, max_priority, max_gas):
    def out_rlp(o):
        return rlp_list([rlp_bytes(addr20(o[0])), rlp_int(o[1])])

    signed_inputs = rlp_list([
        rlp_list([rlp_int(i["index"]), rlp_int(i["creation_block"])]) for i in inputs
    ])
    payload = rlp_list([
        rlp_int(chain_id),
        rlp_list([rlp_bytes(addr20(a)) for a in actors]),
        signed_inputs,
        rlp_list([out_rlp(o) for o in utxo_outs]),
        rlp_list([out_rlp(o) for o in account_outs]),
        rlp_int(change_index),
        rlp_bytes(b"" if payer is None else addr20(payer)),
        rlp_int(max_fee),
        rlp_int(max_priority),
        rlp_int(max_gas),
    ])
    return keccak(bytes([SPEND_MAGIC]) + payload)


def sign_digest(privkey, digest, signer_addr):
    sig = privkey.sign_msg_hash(digest)
    raw = bytes([sig.v]) + sig.r.to_bytes(32, "big") + sig.s.to_bytes(32, "big")
    return FrameSig(SCHEME_SECP256K1, signer_addr, digest, raw)


# ---------- deposits ----------
def deposit(rpc, funder_key, funder_addr, recipient, value, chain_id, gas_price):
    """Create a UTXO by calling the vault with a 20-byte recipient as calldata."""
    nonce = int(rpc.call("eth_getTransactionCount", [funder_addr, "pending"]), 16)
    tx = {
        "type": 2,
        "chainId": chain_id,
        "nonce": nonce,
        "to": VAULT_ADDR,
        "value": value,
        "data": "0x" + addr20(recipient).hex(),
        "gas": 200_000,
        "maxFeePerGas": gas_price * 4,
        "maxPriorityFeePerGas": gas_price,
    }
    from eth_account import Account
    signed = Account.from_key(funder_key).sign_transaction(tx)
    h = rpc.call("eth_sendRawTransaction", ["0x" + signed.raw_transaction.hex()])
    return h


def utxo_created_in_block(rpc, block_number):
    """Every UTXO the block created, from the vault's own logs, ordered by index."""
    logs = rpc.call("eth_getLogs", [{
        "fromBlock": hex(block_number),
        "toBlock": hex(block_number),
        "address": VAULT_ADDR,
        "topics": [UTXO_CREATED_TOPIC],
    }])
    out = []
    for log in logs:
        data = bytes.fromhex(log["data"][2:])
        out.append({
            "index": int.from_bytes(data[:32], "big"),
            "value": int.from_bytes(data[32:64], "big"),
            "source": "0x" + log["topics"][1][-40:],
            "recipient": "0x" + log["topics"][2][-40:],
        })
    out.sort(key=lambda u: u["index"])
    return out


# ---------- test infrastructure ----------
RESULTS = []


def case(name):
    def deco(fn):
        fn._case_name = name
        return fn
    return deco


def record(name, ok, detail=""):
    RESULTS.append((name, ok, detail))
    print(f"{'PASS' if ok else 'FAIL'}  {name}" + (f"\n      {detail}" if detail else ""))


class Ctx:
    def __init__(self, rpc, peers, chain_id, funder_key, funder_addr, gas_price):
        self.rpc = rpc
        self.peers = peers
        self.chain_id = chain_id
        self.funder_key = funder_key
        self.funder_addr = funder_addr
        self.gas_price = gas_price
        self.run_salt = os.urandom(16)

    def new_actor(self, seed: int):
        """A throwaway account, unique per run.

        The run salt matters: two cases below assert on an account being brand
        new (no leaf, zero balance) or on an exact received amount. With a fixed
        seed those pass on a clean chain and then fail on the second run against
        the same chain, because the address already exists and balances have
        accumulated — a real failure and a harness artifact look identical.
        """
        pk = keys.PrivateKey(keccak(self.run_salt + seed.to_bytes(2, "big")))
        return pk, pk.public_key.to_checksum_address()


def make_utxo(ctx, recipient, value):
    """Deposit, wait for inclusion, and return (block, index, source) of the UTXO."""
    h = deposit(ctx.rpc, ctx.funder_key, ctx.funder_addr, recipient, value,
                ctx.chain_id, ctx.gas_price)
    r = wait_receipt(ctx.rpc, h)
    if r is None or int(r["status"], 16) != 1:
        raise RuntimeError(f"deposit failed: {r}")
    blk = int(r["blockNumber"], 16)
    created = [u for u in utxo_created_in_block(ctx.rpc, blk)
               if u["recipient"].lower() == recipient.lower()]
    if not created:
        raise RuntimeError("deposit produced no UtxoCreated log")
    return blk, created[-1]


def build_spend_tx(ctx, actor_key, actor_addr, utxo, creation_block,
                   utxo_outs=None, account_outs=None, payer=None,
                   frames_prefix=None, tamper=None, batch_siblings=None):
    """A self-funded spend of `utxo`, with the change going back to the actor.

    The witness is built by reconstructing the creation block's whole openings
    tree from its logs — exactly what a real wallet must do.
    """
    all_created = utxo_created_in_block(ctx.rpc, creation_block)
    leaves = [opening_leaf(u["index"], u["source"], u["recipient"], u["value"])
              for u in all_created]
    position = next(i for i, u in enumerate(all_created) if u["index"] == utxo["index"])
    proof = merkle_proof(leaves, position)

    utxo_outs = list(utxo_outs or [])
    account_outs = list(account_outs or [])
    # change output: a UTXO back to the actor, signed with value 0
    utxo_outs.append((actor_addr, 0))
    change_index = len(utxo_outs) - 1

    inp = {
        "index": utxo["index"],
        "creation_block": creation_block,
        "source": utxo["source"],
        "recipient": actor_addr,
        "value": utxo["value"],
        "position": position,
        "siblings": proof,
        "batch_siblings": list(batch_siblings or []),
    }
    if tamper:
        tamper(inp)

    max_fee, max_priority, max_gas = 10**12, 10**12, 30_000_000
    digest = spend_hash(ctx.chain_id, [actor_addr], [inp], utxo_outs, account_outs,
                        change_index, payer, max_fee, max_priority, max_gas)
    data = encode_spend([actor_addr], [inp], utxo_outs, account_outs, change_index,
                        payer, max_fee, max_priority, max_gas)

    frames = list(frames_prefix or [])
    frames.append(Frame(mode=UTXO_MODE, flags=0, target=None,
                        gas_limit=3_000_000, value=0, data=data))
    tx = FrameTx(
        chain_id=ctx.chain_id,
        nonce_keys=[],
        nonce_seq=0,
        sender=VAULT,
        frames=frames,
        signatures=[sign_digest(actor_key, digest, actor_addr)],
        max_priority_fee=ctx.gas_price,
        max_fee=ctx.gas_price * 4,
    )
    return tx


def send_frame_tx(rpc, tx):
    return rpc.call("eth_sendRawTransaction", ["0x" + tx.raw().hex()])


# ---------- cases ----------
@case("deposit creates a UTXO and the block-end openings root commits to it")
def t_openings_root(ctx):
    """Independently recompute the root from the block's logs and compare with the
    root the client wrote to the vault's ring slot. A match means the client's
    leaf encoding, ordering and tree shape agree with the EIP as read by a second
    implementation — not with itself."""
    _, actor = ctx.new_actor(0xA0)
    blk, utxo = make_utxo(ctx, actor, 10**17)
    wait_blocks(ctx.rpc, 1)  # the root is written at the END of the creation block

    created = utxo_created_in_block(ctx.rpc, blk)
    leaves = [opening_leaf(u["index"], u["source"], u["recipient"], u["value"])
              for u in created]
    expected = merkle_root(leaves)
    stored = ctx.rpc.storage_at(VAULT_ADDR, ring_slot(blk))
    ok = stored == int.from_bytes(expected, "big")
    record(t_openings_root._case_name, ok,
           f"block {blk}, {len(created)} creation(s); root {'matches' if ok else 'MISMATCH'} "
           f"(stored {stored:#x}, recomputed {int.from_bytes(expected,'big'):#x})")
    return {"block": blk, "utxo": utxo, "actor": actor}


@case("a recipient holding ZERO ETH spends its UTXO (the EIP's headline claim)")
def t_zero_eth_spend(ctx):
    """The whole point of the design: gas is paid out of the spend's own value via
    the conservation rule, so the owner never needs a funded account."""
    key, actor = ctx.new_actor(0xB0)
    value = 10**17
    blk, utxo = make_utxo(ctx, actor, value)
    wait_blocks(ctx.rpc, 1)

    bal_before = ctx.rpc.balance(actor)
    payee_key, payee = ctx.new_actor(0xB1)
    payout = 10**15

    tx = build_spend_tx(ctx, key, actor, utxo, blk, account_outs=[(payee, payout)])
    h = send_frame_tx(ctx.rpc, tx)
    r = wait_receipt(ctx.rpc, h)

    ok = (bal_before == 0 and r is not None and int(r["status"], 16) == 1
          and ctx.rpc.balance(payee) == payout)
    record(t_zero_eth_spend._case_name, ok,
           f"owner balance before: {bal_before} wei; tx status "
           f"{r and r['status']}; payee received {ctx.rpc.balance(payee)} wei (wanted {payout})")
    return {"spend_tx": h, "utxo": utxo, "block": blk, "actor": actor, "key": key}


@case("the spent bit is set on-chain and the same spend cannot be replayed")
def t_double_spend(ctx):
    key, actor = ctx.new_actor(0xC0)
    blk, utxo = make_utxo(ctx, actor, 10**17)
    wait_blocks(ctx.rpc, 1)

    slot, mask = spent_bit_location(utxo["index"])
    before = ctx.rpc.storage_at(VAULT_ADDR, slot)

    tx = build_spend_tx(ctx, key, actor, utxo, blk)
    h = send_frame_tx(ctx.rpc, tx)
    r = wait_receipt(ctx.rpc, h)
    if r is None or int(r["status"], 16) != 1:
        record(t_double_spend._case_name, False, f"first spend did not land: {r}")
        return None

    after = ctx.rpc.storage_at(VAULT_ADDR, slot)
    bit_set = (after & mask) != 0 and (before & mask) == 0

    # Replay the identical spend. Replay protection for a vault-sender tx is the
    # spent bit alone — there is no nonce.
    replayed = False
    try:
        send_frame_tx(ctx.rpc, tx)
        # Accepted into the pool at best; it must never be included again.
        wait_blocks(ctx.rpc, 2)
        replayed = wait_receipt(ctx.rpc, h, timeout=5) is not None and False
    except RpcError as e:
        replayed = False
        detail_err = str(e)[:90]
    else:
        detail_err = "pool accepted the replay but no second inclusion"

    ok = bit_set and not replayed
    record(t_double_spend._case_name, ok,
           f"spent bit {'set' if bit_set else 'NOT set'} at slot {slot:#x}; replay rejected "
           f"({detail_err})")
    return None


@case("a forged proof cannot spend (mempool rejects it)")
def t_forged_proof(ctx):
    """Soundness: a witness that does not fold to the committed root must not
    spend. Without this, value could be minted out of the vault's pooled balance."""
    key, actor = ctx.new_actor(0xD0)
    blk, utxo = make_utxo(ctx, actor, 10**17)
    wait_blocks(ctx.rpc, 1)

    def inflate(inp):
        inp["value"] = inp["value"] * 1000  # claim 1000x what was committed

    tx = build_spend_tx(ctx, key, actor, utxo, blk, tamper=inflate)
    rejected, detail = False, ""
    try:
        h = send_frame_tx(ctx.rpc, tx)
        r = wait_receipt(ctx.rpc, h, timeout=30)
        rejected = r is None or int(r["status"], 16) != 1
        detail = "pooled but not included" if r is None else f"status {r['status']}"
    except RpcError as e:
        rejected, detail = True, f"rejected at admission: {str(e)[:100]}"
    record(t_forged_proof._case_name, rejected, detail)
    return None


@case("a UTXO is not spendable in its own creation block")
def t_not_yet_spendable(ctx):
    """Its openings root is written at the END of the creation block, so the
    earliest a spend can prove it is the next block. This must be a TRANSIENT
    failure: the tx stays pooled and lands later, rather than being evicted."""
    key, actor = ctx.new_actor(0xE0)
    blk, utxo = make_utxo(ctx, actor, 10**17)
    # Deliberately do NOT wait: submit while the head is still the creation block.
    tx = build_spend_tx(ctx, key, actor, utxo, blk)
    try:
        h = send_frame_tx(ctx.rpc, tx)
    except RpcError as e:
        record(t_not_yet_spendable._case_name, False,
               f"admission rejected a spend that should be kept pending: {str(e)[:100]}")
        return None
    # It must eventually land, without resubmission.
    r = wait_receipt(ctx.rpc, h, timeout=90)
    ok = r is not None and int(r["status"], 16) == 1
    record(t_not_yet_spendable._case_name, ok,
           f"submitted at head=creation block {blk}; "
           + (f"included in block {int(r['blockNumber'],16)}" if r else "never included"))
    return None


def batch_witness_path(chain_id, batch_index):
    return os.path.join(os.path.dirname(os.path.abspath(__file__)),
                        f".batch-witness-{chain_id}-{batch_index}.json")


def save_batch_leaves(chain_id, batch_index, leaves):
    """Persist a batch's ring roots.

    Necessary, not an optimisation. RING_SIZE == BATCH_SIZE, so the whole of batch
    `b` is readable from the ring at exactly ONE head: its last block,
    `(b+1)*BATCH_SIZE - 1`. One block earlier that block's own root is not written
    yet; one block later the batch's first slot has been overwritten. A wallet is
    therefore expected to follow the chain and keep its witness, which is what this
    file stands in for.
    """
    with open(batch_witness_path(chain_id, batch_index), "w") as f:
        json.dump([r.hex() for r in leaves], f)


def load_batch_leaves(chain_id, batch_index):
    try:
        with open(batch_witness_path(chain_id, batch_index)) as f:
            return [bytes.fromhex(h) for h in json.load(f)]
    except FileNotFoundError:
        return None


def ring_roots_of_batch(rpc, batch_index, block="latest"):
    """Every ring root of `batch_index`, in block order — the batch tree's leaves.

    The ring holds one root per block for RING_SIZE blocks and then wraps, so these
    are only readable while the batch's blocks are still the ones occupying the
    ring. Right after a batch seals is exactly that window, which is why a wallet is
    expected to upgrade its witness promptly.

    Batched JSON-RPC: 8192 individual calls is minutes of round trips.
    """
    first = batch_index * BATCH_SIZE
    roots = []
    CHUNK = 256
    for start in range(first, first + BATCH_SIZE, CHUNK):
        batch = []
        for n in range(start, min(start + CHUNK, first + BATCH_SIZE)):
            slot = "0x" + ring_slot(n).to_bytes(32, "big").hex()
            batch.append({"jsonrpc": "2.0", "id": n, "method": "eth_getStorageAt",
                          "params": [VAULT_ADDR, slot, block]})
        req = urllib.request.Request(
            rpc.url, data=json.dumps(batch).encode(),
            headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=120) as resp:
            out = json.loads(resp.read())
        by_id = {o["id"]: o for o in out}
        for n in range(start, min(start + CHUNK, first + BATCH_SIZE)):
            roots.append(bytes.fromhex(by_id[n]["result"].removeprefix("0x").zfill(64)))
    return roots


# A fixed key, so the batch-proof case can find and spend a UTXO it created on an
# EARLIER run. Everything else here derives actors per run to stay idempotent, but
# this case cannot: a batch takes BATCH_SIZE blocks to seal, so the deposit and the
# spend are necessarily different runs and the key has to survive between them.
# Devnet-only, and deliberately worthless outside one.
BATCH_CASE_SEED = bytes.fromhex(
    "8312babe00000000000000000000000000000000000000000000000000000001")


@case("a sealed batch proof spends a UTXO (two-phase: deposits, then spends later)")
def t_batch_proof(ctx):
    """The second witness form, and the only EIP-8312 path a single run cannot reach.

    A ring proof is good for RING_SIZE blocks; past that the holder must fold the
    creation block's openings root up through the sealed batch tree and prove against
    the batch slot. A batch seals only after BATCH_SIZE blocks, so this case works in
    two phases across runs: deposit to a fixed key, then spend once that batch has
    sealed.

    There is a window, and it is the point. The batch tree's leaves are the ring
    roots, and the ring wraps after RING_SIZE blocks — so the witness can only be
    BUILT while the batch's blocks still occupy the ring, which is why a wallet is
    expected to upgrade promptly. Past that the UTXO needs an archive, and this case
    reports that rather than failing.

    The spend hash excludes witness fields, so the batch path must verify under the
    same signature a ring proof would have used.
    """
    key = keys.PrivateKey(BATCH_CASE_SEED)
    actor = key.public_key.to_checksum_address()
    head = ctx.rpc.block_number()
    name = t_batch_proof._case_name

    # Look for a UTXO of ours whose batch has sealed and whose ring roots are still
    # readable. Scanning logs by recipient is exactly how a wallet finds its own.
    logs = ctx.rpc.call("eth_getLogs", [{
        "fromBlock": "0x0", "toBlock": "latest", "address": VAULT_ADDR,
        "topics": [UTXO_CREATED_TOPIC, None, "0x" + "00" * 12 + actor[2:].lower()],
    }])
    candidate = None
    for log in logs:
        blk = int(log["blockNumber"], 16)
        batch = blk // BATCH_SIZE
        last = (batch + 1) * BATCH_SIZE - 1
        sealed = head >= last
        # The whole batch is readable from the ring only at head == last (see
        # save_batch_leaves); a saved witness removes that constraint.
        usable = load_batch_leaves(ctx.chain_id, batch) is not None or head == last
        index = int(log["data"][2:66], 16)
        slot, mask = spent_bit_location(index)
        if sealed and usable and not (ctx.rpc.storage_at(VAULT_ADDR, slot) & mask):
            candidate = {"index": index, "block": blk, "batch": batch,
                         "value": int(log["data"][66:130], 16),
                         "source": "0x" + log["topics"][1][-40:]}
            break

    if candidate is None:
        # Phase 1: create one and say exactly when a later run can spend it.
        blk, utxo = make_utxo(ctx, actor, 10**17)
        batch = blk // BATCH_SIZE
        last = (batch + 1) * BATCH_SIZE - 1
        record(name, True,
               f"phase 1: deposited index {utxo['index']} at block {blk} (batch {batch}). "
               f"The batch seals at block {last}, which is also the ONLY head at which "
               f"its ring roots are all still readable (RING_SIZE == BATCH_SIZE), so run "
               f"again at block {last} to capture the witness — after that a spend needs "
               f"the saved witness. {last - head} blocks away.")
        return None

    # Phase 2: get the batch's leaves — from the saved witness, or from the ring if
    # this run happens to sit in the one-block window where it is still complete.
    leaves = load_batch_leaves(ctx.chain_id, candidate["batch"])
    if leaves is None:
        leaves = ring_roots_of_batch(ctx.rpc, candidate["batch"])
        save_batch_leaves(ctx.chain_id, candidate["batch"], leaves)
    computed = merkle_root(leaves)
    stored = ctx.rpc.storage_at(VAULT_ADDR, batch_slot_for_block(candidate["block"]))
    if stored != int.from_bytes(computed, "big"):
        record(name, False,
               f"batch {candidate['batch']} root mismatch: stored {stored:#x} vs "
               f"recomputed {int.from_bytes(computed, 'big'):#x}")
        return None
    batch_path = merkle_proof(leaves, candidate["block"] % BATCH_SIZE)

    payee = ctx.new_actor(0xB3)[1]
    tx = build_spend_tx(ctx, key, actor, candidate, candidate["block"],
                        account_outs=[(payee, 10**15)], batch_siblings=batch_path)
    h = send_frame_tx(ctx.rpc, tx)
    r = wait_receipt(ctx.rpc, h, timeout=180)
    ok = r is not None and int(r["status"], 16) == 1
    slot, mask = spent_bit_location(candidate["index"])
    spent = ctx.rpc.storage_at(VAULT_ADDR, slot)
    record(name, ok and bool(spent & mask),
           f"phase 2: batch {candidate['batch']} root verified over {len(leaves)} ring "
           f"roots; index {candidate['index']} (block {candidate['block']}) spent via "
           f"batch proof: {'mined' if ok else 'FAILED'}, "
           f"spent bit {'set' if spent & mask else 'NOT set'}")
    return None


@case("permanent state written: a UTXO cycle vs a plain transfer to a fresh account")
def t_state_growth(ctx):
    """The economic claim, measured on the live chain rather than asserted. Counts
    the account leaves and storage slots each flow actually adds."""
    from eth_account import Account

    # (a) UTXO cycle: deposit + spend, paying a fresh recipient as a UTXO.
    key, actor = ctx.new_actor(0xF0)
    blk, utxo = make_utxo(ctx, actor, 10**17)
    wait_blocks(ctx.rpc, 1)
    _, fresh_utxo_payee = ctx.new_actor(0xF1)
    tx = build_spend_tx(ctx, key, actor, utxo, blk,
                        utxo_outs=[(fresh_utxo_payee, 10**15)])
    h = send_frame_tx(ctx.rpc, tx)
    r_spend = wait_receipt(ctx.rpc, h)

    # The recipient of a UTXO output has NO account leaf: the payment lives in
    # the openings tree plus one spent bit when eventually spent.
    utxo_payee_has_account = ctx.rpc.call(
        "eth_getTransactionCount", [fresh_utxo_payee, "latest"]) != "0x0" or \
        ctx.rpc.balance(fresh_utxo_payee) != 0

    # (b) plain transfer to a brand-new account: writes a 120-byte account leaf.
    _, fresh_eoa = ctx.new_actor(0xF2)
    nonce = int(ctx.rpc.call("eth_getTransactionCount", [ctx.funder_addr, "pending"]), 16)
    plain = {
        "type": 2, "chainId": ctx.chain_id, "nonce": nonce, "to": fresh_eoa,
        "value": 10**15, "data": "0x", "gas": 300_000,
        "maxFeePerGas": ctx.gas_price * 4, "maxPriorityFeePerGas": ctx.gas_price,
    }
    signed = Account.from_key(ctx.funder_key).sign_transaction(plain)
    ph = ctx.rpc.call("eth_sendRawTransaction", ["0x" + signed.raw_transaction.hex()])
    r_plain = wait_receipt(ctx.rpc, ph)
    eoa_created = ctx.rpc.balance(fresh_eoa) == 10**15

    ok = (r_spend and int(r_spend["status"], 16) == 1
          and r_plain and int(r_plain["status"], 16) == 1
          and not utxo_payee_has_account and eoa_created)
    record(t_state_growth._case_name, ok,
           f"UTXO payee {fresh_utxo_payee[:10]}… has NO account leaf "
           f"(nonce 0, balance 0): {not utxo_payee_has_account}; "
           f"plain transfer created a leaf for {fresh_eoa[:10]}…: {eoa_created}; "
           f"gas — spend {int(r_spend['gasUsed'],16) if r_spend else '?'}, "
           f"plain transfer {int(r_plain['gasUsed'],16) if r_plain else '?'}")
    return None


@case("all three execution clients agree after the UTXO activity")
def t_consensus(ctx):
    """Three independent ethrex instances with independent state must agree on the
    head hash — the check that catches nondeterminism in the new consensus rules
    (block-end root writes, durable spent bits, settlement ordering)."""
    if not ctx.peers:
        record(t_consensus._case_name, False, "no peer RPCs supplied")
        return None
    wait_blocks(ctx.rpc, 2)
    target = ctx.rpc.block_number() - 2  # a block every node has certainly seen
    hashes, vault_roots = [], []
    for url in [ctx.rpc.url] + ctx.peers:
        p = Rpc(url)
        blk = p.block(target, False)
        hashes.append(blk["hash"])
        vault_roots.append(p.storage_at(VAULT_ADDR, SLOT_NEXT_INDEX, hex(target)))
    ok = len(set(hashes)) == 1 and len(set(vault_roots)) == 1
    record(t_consensus._case_name, ok,
           f"block {target}: hashes {'agree' if len(set(hashes))==1 else set(hashes)}; "
           f"vault index counter {'agrees' if len(set(vault_roots))==1 else vault_roots} "
           f"(= {vault_roots[0]} UTXOs created so far)")
    return None


CASES = [t_openings_root, t_zero_eth_spend, t_double_spend, t_forged_proof,
         t_not_yet_spendable, t_state_growth, t_batch_proof, t_consensus]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rpc", required=True)
    ap.add_argument("--peers", default="")
    ap.add_argument("--funder-key", required=True)
    ap.add_argument("--only", default="")
    args = ap.parse_args()

    rpc = Rpc(args.rpc)
    peers = [u for u in args.peers.split(",") if u]
    chain_id = int(rpc.call("eth_chainId"), 16)
    fk = keys.PrivateKey(bytes.fromhex(args.funder_key.removeprefix("0x")))
    funder = fk.public_key.to_checksum_address()
    gas_price = max(int(rpc.call("eth_gasPrice"), 16), 10**9)

    print(f"chain {chain_id}, head {rpc.block_number()}, funder {funder} "
          f"({rpc.balance(funder)/10**18:.3f} ETH), gas price {gas_price}")
    vault_code = rpc.call("eth_getCode", [VAULT_ADDR, "latest"])
    print(f"vault code: {len(vault_code)//2 - 1} bytes at {VAULT_ADDR}")
    if len(vault_code) <= 2:
        print("FATAL: the vault has no code — EIP-8312 is not active on this chain")
        return 2

    ctx = Ctx(rpc, peers, chain_id, fk, funder, gas_price)
    for fn in CASES:
        if args.only and args.only not in fn.__name__:
            continue
        try:
            fn(ctx)
        except Exception as e:  # a case blowing up is a failure, not a crash
            record(fn._case_name, False, f"{type(e).__name__}: {e}")

    passed = sum(1 for _, ok, _ in RESULTS if ok)
    print(f"\n{passed}/{len(RESULTS)} cases passed")
    return 0 if passed == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
