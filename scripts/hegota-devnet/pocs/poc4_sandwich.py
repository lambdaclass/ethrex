#!/usr/bin/env python3
"""P4 — sandwiched swap: the fill is worse than the quote because someone traded first.

The victim is shown a quote, and by execution time another trade has moved the pool
against them. The POST_TX assertion carries the minimum output the victim committed to at
signing time, so a fill below it invalidates the transaction.

Two properties worth noting from the result, one favourable and one not:

  * The defended transaction is EXCLUDED and the approved gas payment is rolled back, so
    the victim pays NOTHING. Today the choice is between eating a bad fill and reverting on
    slippage while still burning gas. Here the transaction simply does not happen, for free.
    The attacker's own trades still execute, so a searcher who moved the pool can be left
    holding the position.
  * That same property means a builder bears the execution cost of assertion-reverting
    transactions with no compensation, which is the anti-denial-of-service question already
    open in docs/eip-7906.md, and a plausible incentive to deprioritize guarded traffic.

Ordering note: the pool is moved in an earlier transaction rather than by winning a
same-block ordering race. The economics the victim experiences are identical — their fill
is worse than quoted because someone else traded first — and it does not make the result
depend on beating the builder's ordering, which frame-transaction gossip makes unreliable.
The same-block variant is left as future work and is not claimed here.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403

BALANCE_BASE_SLOT = 1


def main():
    b = bank()

    print("\n  [setup] deploying")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    g_net = b.deploy(compile_sol("guards/Guards.sol", "AssertNetEffect"), "AssertNetEffect")
    ta = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "token A")
    tb = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "token B")
    amm = b.deploy(compile_sol("targets/Market.sol", "MockAMM") +
                   bytes(12) + bytes.fromhex(ta[2:]) + bytes(12) + bytes.fromhex(tb[2:]),
                   "MockAMM(A/B)")
    for t in (ta, tb):
        b.send_tx(t, 0, encode_call("mint(address,uint256)", amm, 1_000_000), gas=GAS_WRITE_CALL)

    def quote(amount_in):
        return int(rpc("eth_call", [{"to": to_checksum_address(amm), "data": "0x" +
            encode_call("quote(uint256,bool)", amount_in, True).hex()}, "latest"]), 16)

    def bal(token, who):
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("balanceOf(address)", who).hex()}, "latest"]), 16)

    AMOUNT_IN = 10_000
    quoted = quote(AMOUNT_IN)
    print(f"    quote for {AMOUNT_IN} A -> {quoted} B")

    ev = Evidence(
        scenario="poc4_sandwich",
        title="Sandwiched swap — the fill is worse than the quote",
        models="Value extracted from a user's own swap by trading ahead of it; the victim's "
               "transaction is honest and their wallet is uncompromised",
        defense_kind="reverts the attack",
        addresses={"shim": shim, "guard": g_net, "tokenA": ta, "tokenB": tb, "amm": amm},
    )
    ev.extra["quote"] = {"amount_in": AMOUNT_IN, "quoted_out": quoted}

    def new_victim(label):
        v = fresh_account(b, 25 * 10**16, label)
        b.send_tx(ta, 0, encode_call("mint(address,uint256)", v.address, 100_000), gas=GAS_WRITE_CALL)
        return v

    def swap_with(victim, guards, label, **kw):
        return submit(frame_tx(victim, victim, [
            verify_frame(victim.address),
            sender_frame(ta, data=encode_call("approve(address,uint256)", amm, AMOUNT_IN)),
            sender_frame(amm, data=encode_call("swap(uint256,bool)", AMOUNT_IN, True)),
        ] + guards), label, **kw)

    # ---- someone trades ahead of the victim ----
    print("\n  [frontrun] a large trade moves the pool before the victim's swap lands")
    b.send_tx(ta, 0, encode_call("mint(address,uint256)", b.address, 400_000), gas=GAS_WRITE_CALL)
    b.send_tx(ta, 0, encode_call("approve(address,uint256)", amm, 400_000), gas=GAS_WRITE_CALL)
    b.send_tx(amm, 0, encode_call("swap(uint256,bool)", 400_000, True), gas=GAS_WRITE_CALL)
    moved = quote(AMOUNT_IN)
    print(f"    quote for the same {AMOUNT_IN} A is now {moved} B "
          f"({100 * moved // max(quoted,1)}% of the original)")
    if moved >= quoted:
        raise RuntimeError("the frontrun did not move the price against the victim")
    ev.extra["after_frontrun"] = {"quoted_out": moved,
                                  "pct_of_original": 100 * moved // max(quoted, 1)}

    # ---- phase A: unguarded victim takes the bad fill ----
    print("\n  [phase A] unguarded swap fills at the moved price")
    v0 = new_victim("victim A")
    before_b = bal(tb, v0.address)
    o = swap_with(v0, [], "A unguarded swap", expect_mine=True)
    if not o.mined:
        raise RuntimeError(f"phase A should have mined: {o.simulation}")
    got = bal(tb, v0.address) - before_b
    print(f"    received {got} B against a quote of {quoted}")
    if got >= quoted:
        raise RuntimeError("phase A did not demonstrate a worse-than-quoted fill")
    ev.phase_a = {"victim": v0.address, "tx": o.txhash, "block": o.block,
                  "received": got, "quoted": quoted,
                  "shortfall_pct": 100 - (100 * got // max(quoted, 1))}
    ev.note(f"Phase A: the victim received {got} against a quote of {quoted} — a "
            f"{100 - (100 * got // max(quoted,1))}% shortfall.")

    # ---- phase B: the committed minimum invalidates the bad fill ----
    print("\n  [phase B] the same swap carrying the committed minimum output")
    min_out = quoted * 99 // 100
    v1 = new_victim("victim B")
    eth_before = balance(v1.address)
    before_b = bal(tb, v1.address)
    o1 = swap_with(v1, [guard_frame(g_net, encode_call(
        "assertTokenDelta(address,address,address,uint256,uint256,uint256)",
        shim, tb, v1.address, BALANCE_BASE_SLOT, min_out, 2**255))],
        f"B minReceived={min_out} (expect REVERT)")
    delta_b = bal(tb, v1.address) - before_b
    eth_spent = eth_before - balance(v1.address)
    print(f"    mined={o1.mined}  token B delta={delta_b}  ETH spent={eth_spent}")
    if o1.mined or delta_b != 0:
        raise RuntimeError("the committed minimum failed to stop the bad fill")

    ev.phase_b = {"victim": v1.address, "min_committed": min_out, "mined": o1.mined,
                  "token_delta": delta_b, "eth_spent_wei": eth_spent}
    ev.note(f"Phase B: the transaction was excluded and the victim spent {eth_spent} wei — "
            f"the approved gas payment is rolled back with the body, so a defended "
            f"transaction costs its sender nothing.")
    ev.note("That zero-cost exclusion cuts both ways: it removes the usual "
            "revert-and-still-pay-gas penalty for the user, and it leaves a builder bearing "
            "uncompensated execution cost — the anti-denial-of-service question open in "
            "docs/eip-7906.md, and a plausible incentive to deprioritize guarded traffic.")

    # Negative control: a minimum the moved pool can satisfy must still fill.
    print("\n  [negative control] a minimum the moved pool CAN satisfy must still fill")
    v2 = new_victim("victim C")
    before_b = bal(tb, v2.address)
    achievable = moved * 90 // 100
    o2 = swap_with(v2, [guard_frame(g_net, encode_call(
        "assertTokenDelta(address,address,address,uint256,uint256,uint256)",
        shim, tb, v2.address, BALANCE_BASE_SLOT, achievable, 2**255))],
        f"C minReceived={achievable} (expect MINE)", expect_mine=True)
    got2 = bal(tb, v2.address) - before_b
    if not o2.mined or got2 == 0:
        raise RuntimeError(f"the guard rejected an achievable fill: {o2.simulation}")
    ev.extra["negative_control"] = {
        "description": "the assertion is a bound, not a blanket refusal: a minimum the moved "
                       "pool can satisfy still fills",
        "min_committed": achievable, "received": got2, "tx": o2.txhash, "mined": o2.mined}
    ev.note(f"Negative control: with an achievable minimum of {achievable} the same guard "
            f"allowed the swap, which returned {got2}.")
    return ev


if __name__ == "__main__":
    run_scenario("P4 — sandwiched swap", main)
