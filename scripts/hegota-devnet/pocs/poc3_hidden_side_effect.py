#!/usr/bin/env python3
"""P3 — a transaction that does one extra thing.

The victim is shown a simple transfer. The transaction they sign also moves tokens to an
attacker — the malicious-multicall / one-click-drain shape, where the displayed intent is a
subset of what executes.

The assertion is targeted: the exfiltration address's balance slot must not change. The
differential form is correct here, unlike in P5 and P6, because the adversarial write happens
INSIDE the guarded transaction.

WHY NOT DENY-BY-DEFAULT. The more general form — "every slot this transaction wrote must be
one I expected" — is what you would reach for first, and it is currently unusable on this
implementation. Block building and `ethrex_simulateFrameTransaction` disagree about how many
slots the transaction writes: simulation reports 2 for the transfer below, block building
observes 5. A deny-by-default guard therefore passes simulation and then reverts during block
building, so the transaction is admitted and silently dropped while the assertion appears to
"work" for entirely the wrong reason. That divergence was found by the negative control in
this very scenario; see GATE-RESULTS.md and the author notes. Until the two paths agree, only
targeted assertions over slots the author names explicitly are dependable.

The trade-off is stated plainly: a targeted assertion requires knowing the exfiltration
address in advance, which deny-by-default would not.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403

BALANCE_BASE_SLOT = 1


def balance_slot(owner: str) -> int:
    return int.from_bytes(keccak(bytes(12) + bytes.fromhex(owner[2:]) +
                                 BALANCE_BASE_SLOT.to_bytes(32, "big")), "big")


def main():
    b = bank()

    print("\n  [setup] deploying")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    guard = b.deploy(compile_sol("guards/Guards.sol", "AssertSlotUnchanged"), "AssertSlotUnchanged")
    token = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "MockERC20")
    payee = "0x" + "5e" * 20      # the counterparty the victim intends to pay
    attacker = "0x" + "ba" * 20   # the hidden recipient

    ev = Evidence(
        scenario="poc3_hidden_side_effect",
        title="A transaction that does one extra thing",
        models="Malicious multicalls and one-click drains, where the displayed intent is a "
               "subset of what the transaction actually executes",
        defense_kind="reverts the attack",
        addresses={"shim": shim, "guard": guard, "token": token,
                   "intended_payee": payee, "attacker": attacker},
    )

    def bal(who):
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("balanceOf(address)", who).hex()}, "latest"]), 16)

    def new_victim(label):
        v = fresh_account(b, 3 * 10**17, label)
        b.send_tx(token, 0, encode_call("mint(address,uint256)", v.address, 1000), gas=GAS_WRITE_CALL)
        return v

    guard_data = encode_call("assertUnchanged(address,address,uint256)",
                             shim, token, balance_slot(attacker))

    def send_with(victim, guards, label, include_hidden=True, **kw):
        body = [sender_frame(token, data=encode_call("transfer(address,uint256)", payee, 100))]
        if include_hidden:
            body.append(sender_frame(token, data=encode_call("transfer(address,uint256)", attacker, 50)))
        return submit(frame_tx(victim, victim, [verify_frame(victim.address)] + body + guards),
                      label, **kw)

    # ---- phase A: the hidden transfer goes unnoticed ----
    print("\n  [phase A] unguarded: the displayed transfer plus a hidden one")
    v0 = new_victim("victim A")
    before = bal(attacker)
    o = send_with(v0, [], "A transfer(payee,100) + hidden transfer(attacker,50)", expect_mine=True)
    if not o.mined:
        raise RuntimeError(f"phase A should have mined: {o.simulation}")
    hidden = bal(attacker) - before
    print(f"    payee received 100, attacker quietly received {hidden}")
    if hidden == 0:
        raise RuntimeError("phase A did not demonstrate the hidden transfer")
    ev.phase_a = {"victim": v0.address, "tx": o.txhash, "block": o.block,
                  "intended_to_payee": 100, "hidden_to_attacker": hidden}
    ev.note("Phase A: both transfers executed; only one was displayed.")

    # ---- phase B: the exfiltration address's slot must not move ----
    print("\n  [phase B] assert the attacker's balance slot is untouched")
    v1 = new_victim("victim B")
    before = bal(attacker)
    o1 = send_with(v1, [guard_frame(guard, guard_data)],
                   "B same transaction, targeted guard (expect REVERT)")
    hidden1 = bal(attacker) - before
    print(f"    mined={o1.mined}  attacker delta={hidden1}")
    if o1.mined or hidden1 != 0:
        raise RuntimeError("the guard failed to catch the hidden transfer")
    ev.phase_b = {"victim": v1.address, "mined": o1.mined, "attacker_delta": hidden1,
                  "assertion": "the exfiltration address's balance slot is unchanged "
                               "within this transaction"}
    ev.note("Phase B: the hidden transfer moved the attacker's balance slot inside the "
            "transaction, so the differential assertion fired and the transaction was "
            "invalidated.")

    # ---- negative control: the displayed transfer alone must still pass ----
    print("\n  [negative control] the displayed transfer alone must pass")
    v2 = new_victim("victim C")
    o2 = send_with(v2, [guard_frame(guard, guard_data)],
                   "C displayed transfer only (expect MINE)",
                   include_hidden=False, expect_mine=True)
    if not o2.mined:
        raise RuntimeError(f"the guard rejected the victim's intended transfer: {o2.simulation}")
    print(f"    payee total after the permitted transfer: {bal(payee)}")
    ev.extra["negative_control"] = {
        "description": "with the hidden transfer removed, the identical guard permits the "
                       "transaction the victim actually intended",
        "mined": o2.mined, "tx": o2.txhash, "block": o2.block}
    ev.note("Negative control: the same guard permitted the intended transfer on its own, so "
            "it discriminates between the displayed intent and the extra effect.")
    ev.note("This control is what exposed the simulation/block-building divergence on the "
            "transaction's storage-change set. An earlier version of this scenario used a "
            "deny-by-default assertion over all written slots; it appeared to defend phase B, "
            "but the control revealed it was rejecting every transaction — including "
            "legitimate ones — because block building observes 5 slot changes where simulation "
            "reports 2. A defense that also blocks the honest case is not a defense, and only "
            "the control could tell the two apart.")
    ev.extra["deny_by_default_limitation"] = {
        "simulation_slot_change_count": 2,
        "block_building_slot_change_count": 5,
        "consequence": "deny-by-default assertions over TXTRACE's slot-change enumeration are "
                       "currently unusable: they pass simulation and revert during block "
                       "building, so the transaction is admitted and silently dropped",
        "measured_by": "AssertSlotChangeCount probe asserting counts 2..5; only 5 was included",
    }
    return ev


if __name__ == "__main__":
    run_scenario("P3 — hidden side effect", main)
