#!/usr/bin/env python3
"""P6 — implementation swapped between what the user was shown and what executes.

The victim interacts with an upgradeable proxy whose implementation was replaced with
hostile logic in an EARLIER transaction. This scenario is also where two assertion-design
traps are demonstrated rather than asserted, because both silently defeat the published
proof-of-concept as framed:

  variant 1  differential guard — "the implementation slot did not change"
             PASSES, because TXDIFF's before/after is scoped to THIS transaction and the
             upgrade happened in a prior one. The attack still succeeds.

  variant 2  code-hash guard on the PROXY
             PASSES, because an upgrade does not change the proxy's bytecode — only the
             storage slot holding the implementation address. The attack still succeeds.

  variant 3  absolute guard — "the implementation slot equals what I committed to"
             REVERTS. This is the only correct form.

Variants 1 and 2 are deliberate negative results and are preserved as evidence.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403

# bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
IMPL_SLOT = 0x360894A13BA1A3210667C828492DB98DCA3E2076CC3735A920A3CA505D382BBC
THIEF = "0x" + f"{0xBAD:040x}"


def main():
    b = bank()

    print("\n  [setup] deploying")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    g_unchanged = b.deploy(compile_sol("guards/Guards.sol", "AssertSlotUnchanged"), "AssertSlotUnchanged")
    g_equals = b.deploy(compile_sol("guards/Guards.sol", "AssertSlotEquals"), "AssertSlotEquals")
    token = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "MockERC20")
    benign = b.deploy(compile_sol("targets/Targets.sol", "ImplBenign"), "ImplBenign")
    hostile = b.deploy(compile_sol("targets/Targets.sol", "ImplHostile"), "ImplHostile")
    proxy = b.deploy(compile_sol("targets/Targets.sol", "MinimalProxy") +
                     bytes(12) + bytes.fromhex(benign[2:]), "MinimalProxy(->benign)")

    proxy_codehash = int.from_bytes(keccak(bytes.fromhex(
        rpc("eth_getCode", [to_checksum_address(proxy), "latest"])[2:])), "big")
    print(f"    proxy implementation now: {'0x' + f'{storage_at(proxy, IMPL_SLOT):040x}'}")

    ev = Evidence(
        scenario="poc6_proxy_swap",
        title="Implementation swapped before inclusion — and two assertion traps",
        models="A user transacting into an upgradeable contract whose implementation was "
               "replaced after they were shown a quote; the CPIMP class (~$1M, 2025) is the "
               "clean fit, where a malicious implementation was inserted to intercept user calls",
        defense_kind="reverts the attack",
        addresses={"shim": shim, "proxy": proxy, "benign_impl": benign,
                   "hostile_impl": hostile, "token": token,
                   "guard_differential": g_unchanged, "guard_absolute": g_equals},
    )

    def thief_balance():
        return int(rpc("eth_call", [{"to": to_checksum_address(token), "data": "0x" +
            encode_call("balanceOf(address)", THIEF).hex()}, "latest"]), 16)

    def deposit_with(victim, guard_frames, label):
        """The victim's intent: approve 100 and deposit it into the proxy."""
        return submit(frame_tx(victim, victim, [
            verify_frame(victim.address),
            sender_frame(token, data=encode_call("approve(address,uint256)", proxy, 100)),
            sender_frame(proxy, data=encode_call("deposit(address,uint256)", token, 100)),
        ] + guard_frames), label)

    def new_victim(label):
        v = fresh_account(b, 6 * 10**17, label)
        b.send_tx(token, 0, encode_call("mint(address,uint256)", v.address, 1000), gas=GAS_WRITE_CALL)
        return v

    # ---- the attacker upgrades the proxy in its own, earlier transaction ----
    print("\n  [attack] owner upgrades the implementation to hostile logic (prior block)")
    b.send_tx(proxy, 0, encode_call("upgradeTo(address)", hostile), gas=GAS_WRITE_CALL)
    swapped_to = "0x" + f"{storage_at(proxy, IMPL_SLOT):040x}"
    print(f"    proxy implementation now: {swapped_to}")
    if swapped_to.lower() != hostile.lower():
        raise RuntimeError("the upgrade did not take effect")
    ev.extra["upgrade"] = {"from": benign, "to": hostile, "slot": hex(IMPL_SLOT)}

    # ---- phase A: unguarded victim loses the deposit ----
    print("\n  [phase A] unguarded victim deposits into the swapped implementation")
    v0 = new_victim("victim A")
    before = thief_balance()
    o = deposit_with(v0, [], "A unguarded deposit")
    if not o.mined:
        raise RuntimeError(f"phase A deposit should have mined: {o.simulation}")
    stolen = thief_balance() - before
    print(f"    hostile implementation diverted {stolen} tokens")
    if stolen == 0:
        raise RuntimeError("phase A did not demonstrate the loss")
    ev.phase_a = {"victim": v0.address, "tx": o.txhash, "block": o.block, "stolen": stolen}
    ev.note("Phase A: the victim's deposit was diverted by the implementation that had "
            "already been swapped in.")

    # ---- variant 1: differential guard silently passes ----
    print("\n  [variant 1] differential guard: 'implementation slot did not change'")
    v1 = new_victim("victim V1")
    before = thief_balance()
    o1 = deposit_with(v1, [guard_frame(g_unchanged, encode_call(
        "assertUnchanged(address,address,uint256)", shim, proxy, IMPL_SLOT))],
        "V1 differential guard (expect PASS = trap)")
    stolen1 = thief_balance() - before
    trap1 = o1.mined and stolen1 > 0
    print(f"    mined={o1.mined} stolen={stolen1} -> {'TRAP CONFIRMED' if trap1 else 'unexpected'}")
    if not trap1:
        raise RuntimeError("expected the differential guard to pass and the attack to succeed")

    # ---- variant 2: proxy code-hash guard silently passes ----
    print("\n  [variant 2] code-hash guard on the PROXY address")
    v2 = new_victim("victim V2")
    before = thief_balance()
    o2 = deposit_with(v2, [guard_frame(g_equals, encode_call(
        "assertCodeHashEquals(address,address,uint256)", shim, proxy, proxy_codehash))],
        "V2 proxy code-hash guard (expect PASS = trap)")
    stolen2 = thief_balance() - before
    trap2 = o2.mined and stolen2 > 0
    print(f"    mined={o2.mined} stolen={stolen2} -> {'TRAP CONFIRMED' if trap2 else 'unexpected'}")
    if not trap2:
        raise RuntimeError("expected the proxy code-hash guard to pass and the attack to succeed")

    # ---- variant 3: absolute guard catches it ----
    print("\n  [variant 3] absolute guard: 'implementation slot equals what I committed to'")
    v3 = new_victim("victim V3")
    before = thief_balance()
    o3 = deposit_with(v3, [guard_frame(g_equals, encode_call(
        "assertSlotEquals(address,address,uint256,uint256)",
        shim, proxy, IMPL_SLOT, int(benign, 16)))],
        "V3 absolute guard (expect REVERT)")
    stolen3 = thief_balance() - before
    print(f"    mined={o3.mined} stolen={stolen3}")
    if o3.mined or stolen3 != 0:
        raise RuntimeError("the absolute guard failed to stop the attack")

    ev.phase_b = {
        "variant_1_differential": {
            "guard": "assertUnchanged(implementation slot)", "mined": o1.mined,
            "stolen": stolen1, "verdict": "TRAP — passes because TXDIFF before/after is "
            "scoped to this transaction and the upgrade happened in a prior one"},
        "variant_2_proxy_codehash": {
            "guard": "assertCodeHashEquals(proxy)", "mined": o2.mined, "stolen": stolen2,
            "verdict": "TRAP — passes because an upgrade does not change the proxy's bytecode"},
        "variant_3_absolute": {
            "guard": "assertSlotEquals(implementation slot, committed value)",
            "mined": o3.mined, "stolen": stolen3,
            "verdict": "CORRECT — the only form that detects a change made by an earlier "
                       "transaction"},
    }
    ev.note("Trap 1 confirmed on-chain: a differential assertion over the implementation "
            "slot passed while the implementation HAD been swapped, because TXDIFF's "
            "before/after is scoped to the guarded transaction.")
    ev.note("Trap 2 confirmed on-chain: asserting the PROXY's code hash passed for the same "
            "reason an upgrade is invisible to it — proxy bytecode is unchanged; only the "
            "implementation slot moves.")
    ev.note("Consequence for wallets: detecting a prior-transaction change requires an "
            "ABSOLUTE assertion against a value captured at signing time, so the signer must "
            "commit to expected environment values, not merely request a diff check.")
    return ev


if __name__ == "__main__":
    run_scenario("P6 — proxy/implementation swap (and two assertion traps)", main)
