#!/usr/bin/env python3
"""P5 — the price moved between the quote and execution.

The victim is shown a quote, signs a transaction, and by the time it executes the oracle
has moved — so the trade fills far worse than what was displayed. The move lands in an
EARLIER transaction, which is what makes the assertion form matter.

This scenario is careful about a naming collision worth keeping straight: "oracle
manipulation" as a *hack class* (a flash-loaned pool skewed to drain a lending market) is
attacker-signed and outside EIP-7906's reach entirely. What is defensible here is the
different thing wearing the same name — the victim's OWN trade filling at a price they did
not agree to.

Two guard variants, mirroring P6:
  differential  "the oracle slot did not change"  -> PASSES, the bad fill goes through
  absolute      "the oracle slot is within the band I committed to" -> REVERTS

A net-effect bound on the realized output is also demonstrated, since it defends the same
divergence without needing to name the oracle's storage layout at all.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403

ORACLE_PRICE_SLOT = 0


def main():
    b = bank()

    print("\n  [setup] deploying")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    g_unchanged = b.deploy(compile_sol("guards/Guards.sol", "AssertSlotUnchanged"), "AssertSlotUnchanged")
    g_equals = b.deploy(compile_sol("guards/Guards.sol", "AssertSlotEquals"), "AssertSlotEquals")
    g_net = b.deploy(compile_sol("guards/Guards.sol", "AssertNetEffect"), "AssertNetEffect")
    token = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "MockERC20")
    oracle = b.deploy(compile_sol("targets/Market.sol", "MockOracle") + (10**18).to_bytes(32, "big"),
                      "MockOracle(price=1e18)")
    desk = b.deploy(compile_sol("targets/Market.sol", "OracleDesk") +
                    bytes(12) + bytes.fromhex(token[2:]) + bytes(12) + bytes.fromhex(oracle[2:]),
                    "OracleDesk")
    b.send_tx(token, 0, encode_call("mint(address,uint256)", desk, 10**9), gas=GAS_WRITE_CALL)

    quoted_price = storage_at(oracle, ORACLE_PRICE_SLOT)
    print(f"    quoted price: {quoted_price}  (victim expects ~1:1)")

    ev = Evidence(
        scenario="poc5_oracle_toctou",
        title="Price moved between the quote and execution",
        models="A victim's own trade filling at a price that changed after they were shown a "
               "quote. Distinct from attacker-signed oracle-manipulation exploits, which a "
               "sender-authored assertion cannot reach.",
        defense_kind="reverts the attack",
        addresses={"shim": shim, "token": token, "oracle": oracle, "desk": desk,
                   "guard_differential": g_unchanged, "guard_absolute": g_equals,
                   "guard_net_effect": g_net},
    )

    def bal(who):
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("balanceOf(address)", who).hex()}, "latest"]), 16)

    def new_victim(label):
        v = fresh_account(b, 25 * 10**16, label)
        b.send_tx(token, 0, encode_call("mint(address,uint256)", v.address, 1000), gas=GAS_WRITE_CALL)
        return v

    def buy_with(victim, guards, label, **kw):
        return submit(frame_tx(victim, victim, [
            verify_frame(victim.address),
            sender_frame(token, data=encode_call("approve(address,uint256)", desk, 100)),
            sender_frame(desk, data=encode_call("buy(uint256)", 100)),
        ] + guards), label, **kw)

    # ---- the price moves in its own, earlier transaction ----
    print("\n  [move] oracle price doubled in a prior block (victim's quote is now stale)")
    b.send_tx(oracle, 0, encode_call("set(uint256)", 2 * 10**18), gas=GAS_WRITE_CALL)
    now_price = storage_at(oracle, ORACLE_PRICE_SLOT)
    print(f"    oracle price now: {now_price}")
    if now_price == quoted_price:
        raise RuntimeError("the price move did not take effect")
    ev.extra["price_move"] = {"quoted": quoted_price, "at_execution": now_price,
                             "slot": ORACLE_PRICE_SLOT}

    # ---- phase A: unguarded victim eats the bad fill ----
    print("\n  [phase A] unguarded trade fills at the moved price")
    v0 = new_victim("victim A")
    before = bal(v0.address)
    o = buy_with(v0, [], "A unguarded buy(100)", expect_mine=True)
    if not o.mined:
        raise RuntimeError(f"phase A should have mined: {o.simulation}")
    got = bal(v0.address) - (before - 100)
    print(f"    paid 100, received {got} (expected ~100 at the quoted price)")
    if got >= 100:
        raise RuntimeError("phase A did not demonstrate a worse-than-quoted fill")
    ev.phase_a = {"victim": v0.address, "tx": o.txhash, "paid": 100, "received": got,
                  "expected_at_quote": 100}
    ev.note(f"Phase A: the victim paid 100 and received {got}, against ~100 at the price "
            f"they were shown.")

    # ---- variant 1: differential assertion silently passes ----
    print("\n  [variant 1] differential guard: 'oracle slot did not change'")
    v1 = new_victim("victim V1")
    before = bal(v1.address)
    o1 = buy_with(v1, [guard_frame(g_unchanged, encode_call(
        "assertUnchanged(address,address,uint256)", shim, oracle, ORACLE_PRICE_SLOT))],
        "V1 differential guard (expect PASS = trap)", expect_mine=True)
    got1 = bal(v1.address) - (before - 100)
    trap = o1.mined and got1 < 100
    print(f"    mined={o1.mined} received={got1} -> {'TRAP CONFIRMED' if trap else 'unexpected'}")
    if not trap:
        raise RuntimeError("expected the differential guard to pass and the bad fill to land")

    # ---- variant 2: absolute price-band assertion catches it ----
    print("\n  [variant 2] absolute guard: 'oracle slot equals the quoted price'")
    v2 = new_victim("victim V2")
    before = bal(v2.address)
    o2 = buy_with(v2, [guard_frame(g_equals, encode_call(
        "assertSlotEquals(address,address,uint256,uint256)",
        shim, oracle, ORACLE_PRICE_SLOT, quoted_price))],
        "V2 absolute price assertion (expect REVERT)")
    got2 = bal(v2.address) - (before - 100)
    print(f"    mined={o2.mined} token delta={bal(v2.address) - before}")
    if o2.mined:
        raise RuntimeError("the absolute price assertion failed to stop the bad fill")

    # ---- variant 3: net-effect bound, without naming the oracle layout ----
    print("\n  [variant 3] net-effect bound: 'I must receive at least 95'")
    v3 = new_victim("victim V3")
    before = bal(v3.address)
    o3 = buy_with(v3, [guard_frame(g_net, encode_call(
        "assertTokenDelta(address,address,address,uint256,uint256,uint256)",
        shim, token, v3.address, 1, 95, 2**255))],
        "V3 minReceived=95 (expect REVERT)")
    print(f"    mined={o3.mined} token delta={bal(v3.address) - before}")
    if o3.mined:
        raise RuntimeError("the net-effect bound failed to stop the bad fill")

    ev.phase_b = {
        "variant_1_differential": {"mined": o1.mined, "received": got1,
            "verdict": "TRAP — the oracle moved in a PRIOR transaction, so before == after "
                       "inside the guarded transaction and the assertion passes"},
        "variant_2_absolute_price": {"mined": o2.mined,
            "verdict": "CORRECT — compares the live oracle slot against the price quoted at "
                       "signing time"},
        "variant_3_net_effect": {"mined": o3.mined,
            "verdict": "CORRECT — bounds the realized output directly, so it needs no "
                       "knowledge of the oracle's storage layout"},
    }
    ev.note("The differential form is invalid for this scenario for the same reason as in the "
            "implementation-swap scenario: the adverse change happened in an earlier "
            "transaction, where TXDIFF reads the same value before and after.")
    ev.note("The net-effect bound is the more practical of the two working forms: it defends "
            "the divergence the user actually cares about without needing to name any "
            "counterparty's storage layout.")
    return ev


if __name__ == "__main__":
    run_scenario("P5 — oracle time-of-check/time-of-use", main)
