#!/usr/bin/env python3
"""P7 — standing-allowance elimination.

Models the largest EVM cluster in the 2025-2026 incident record: a victim grants a
standing ERC-20 allowance to a router, and an attacker later abuses a flaw in that
router to `transferFrom` the victim's tokens. Two aggregator drains in January 2026
(~$13.4M and ~$3.67M) and a helper-contract drain in 2025 (~$5M) share this shape.

Phase A  victim approves an unlimited amount, attacker drains it later.
Phase B  the victim's interaction is one frame transaction bundling approve-exactly-N,
         use, and a POST_TX assertion that no allowance survives. The attacker's
         identical later drain then finds nothing to take.

HONEST SCOPE. This does NOT revert the attacker's transaction — nothing a victim signs
can constrain a transaction the attacker signs. It removes the surface the attacker
depends on. That is a different and stronger property than blocking one call, but it
must not be described as blocking the attack.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *  # noqa: F403


def main():
    b = bank()

    print("\n  [setup] deploying")
    shim = b.deploy(compile_yul("TxIntrospection.yul"), "TxIntrospection")
    g_zero = b.deploy(compile_sol("guards/Guards.sol", "AssertAllowanceZero"), "AssertAllowanceZero")
    token = b.deploy(compile_sol("targets/MockERC20.sol", "MockERC20"), "MockERC20")
    router = b.deploy(compile_sol("targets/Targets.sol", "MalRouter"), "MalRouter")
    drainer = b.deploy(compile_sol("targets/Targets.sol", "Drainer"), "Drainer")

    ev = Evidence(
        scenario="poc7_allowance_elimination",
        title="Standing-allowance elimination",
        models="Unvalidated arbitrary-call routers abusing standing ERC-20 allowances "
               "(two aggregator drains Jan 2026, ~$13.4M and ~$3.67M; a helper-contract "
               "drain 2025, ~$5M)",
        defense_kind="removes the attack surface",
        addresses={"shim": shim, "guard": g_zero, "token": token,
                   "router": router, "drainer": drainer},
    )

    # ---------------- phase A: the standing allowance is drained later ----------------
    print("\n  [phase A] unguarded victim: unlimited approval, drained later")
    victim_a = fresh_account(b, 10**17, "victim A")
    b.send_tx(token, 0, encode_call("mint(address,uint256)", victim_a.address, 1000), gas=GAS_WRITE_CALL)

    victim_a.send_tx(token, 0, encode_call("approve(address,uint256)", router, 2**256 - 1),
                     gas=GAS_WRITE_CALL)
    allow_a = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("allowance(address,address)", victim_a.address, router).hex()},
        "latest"]), 16)
    print(f"    standing allowance after the victim's approve: {allow_a if allow_a < 2**200 else 'unlimited'}")

    # The attacker's own transaction, abusing the router's unvalidated call.
    inner = encode_call("transferFrom(address,address,uint256)",
                        victim_a.address, drainer, 400)
    b.send_tx(router, 0, encode_call("execute(address,bytes)", token, inner),
              gas=GAS_WRITE_CALL)

    stolen = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("balanceOf(address)", drainer).hex()}, "latest"]), 16)
    victim_a_bal = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("balanceOf(address)", victim_a.address).hex()}, "latest"]), 16)
    print(f"    attacker took {stolen} tokens; victim left with {victim_a_bal}")
    if stolen == 0:
        raise RuntimeError("phase A did not demonstrate the drain")
    ev.phase_a = {"victim": victim_a.address, "unlimited_allowance": True,
                  "attacker_stole": stolen, "victim_remaining": victim_a_bal}
    ev.note("Phase A: an unlimited standing allowance let the attacker's own later "
            "transaction move the victim's tokens.")

    # ------------- phase B: allowance-free bundle leaves nothing to drain -------------
    print("\n  [phase B] guarded victim: atomic approve-use-assert, zero residual")
    victim_b = fresh_account(b, 6 * 10**17, "victim B")
    b.send_tx(token, 0, encode_call("mint(address,uint256)", victim_b.address, 1000), gas=GAS_WRITE_CALL)
    # The router needs output-token liquidity for the benign swap path.
    b.send_tx(token, 0, encode_call("mint(address,uint256)", router, 1000), gas=GAS_WRITE_CALL)

    guard_data = encode_call(
        "assertZero(address,address,address,address,uint256)",
        shim, token, victim_b.address, router, 2)  # 2 = allowance base slot

    # One transaction: approve exactly 100, swap it, assert nothing is left.
    tx = frame_tx(victim_b, victim_b, [
        verify_frame(victim_b.address),
        sender_frame(token, data=encode_call("approve(address,uint256)", router, 100)),
        sender_frame(router, data=encode_call("swap(address,address,uint256)", token, token, 100)),
        guard_frame(g_zero, guard_data),
    ])
    ok = submit(tx, "approve(exact 100) + swap + assert allowance == 0")
    if not ok.mined:
        raise RuntimeError(f"phase B bundle should have mined: {ok.simulation}")

    residual = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("allowance(address,address)", victim_b.address, router).hex()},
        "latest"]), 16)
    print(f"    residual allowance after the guarded bundle: {residual}")
    if residual != 0:
        raise RuntimeError(f"expected zero residual allowance, got {residual}")

    # The identical attack now finds nothing.
    inner_b = encode_call("transferFrom(address,address,uint256)", victim_b.address, drainer, 400)
    drained_before = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("balanceOf(address)", drainer).hex()}, "latest"]), 16)
    attack_failed = False
    try:
        b.send_tx(router, 0, encode_call("execute(address,bytes)", token, inner_b),
                  gas=GAS_WRITE_CALL)
    except RuntimeError:
        attack_failed = True
    drained_after = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("balanceOf(address)", drainer).hex()}, "latest"]), 16)
    print(f"    identical attack against the guarded victim: "
          f"{'reverted' if attack_failed else 'succeeded'}, attacker delta {drained_after - drained_before}")
    if drained_after != drained_before:
        raise RuntimeError("attacker still drained the guarded victim")

    ev.phase_b = {"victim": victim_b.address, "bundle_tx": ok.txhash, "bundle_block": ok.block,
                  "residual_allowance": residual, "attack_reverted": attack_failed,
                  "attacker_delta": drained_after - drained_before}
    ev.note("Phase B: the victim approved exactly what was consumed and the POST_TX frame "
            "proved no allowance survived, so the attacker's identical transaction had "
            "nothing to take.")
    ev.note("The attacker's transaction is never reverted by EIP-7906. What changed is "
            "that the standing allowance it depended on no longer exists.")

    # Negative control: the same bundle over-approving must be rejected by the guard.
    print("\n  [negative control] over-approving inside the bundle must be rejected")
    victim_c = fresh_account(b, 6 * 10**17, "victim C")
    b.send_tx(token, 0, encode_call("mint(address,uint256)", victim_c.address, 1000), gas=GAS_WRITE_CALL)
    guard_data_c = encode_call("assertZero(address,address,address,address,uint256)",
                               shim, token, victim_c.address, router, 2)
    tx_c = frame_tx(victim_c, victim_c, [
        verify_frame(victim_c.address),
        sender_frame(token, data=encode_call("approve(address,uint256)", router, 500)),
        sender_frame(router, data=encode_call("swap(address,address,uint256)", token, token, 100)),
        guard_frame(g_zero, guard_data_c),
    ])
    bad = submit(tx_c, "approve(500) + swap(100) -> residual 400, must be rejected")
    if bad.mined:
        raise RuntimeError("guard failed to reject a residual allowance")
    residual_c = int(rpc("eth_call", [{"to": to_checksum_address(token),
        "data": "0x" + encode_call("allowance(address,address)", victim_c.address, router).hex()},
        "latest"]), 16)
    ev.extra = {"negative_control": {
        "description": "over-approving leaves a residual allowance, so the guard invalidates "
                       "the victim's own transaction rather than letting a drainable "
                       "allowance persist",
        "mined": bad.mined, "residual_after_rejection": residual_c}}
    ev.note("Negative control: an over-approving bundle was invalidated, and no allowance "
            "was left behind (residual %d)." % residual_c)
    return ev



if __name__ == "__main__":
    run_scenario("P7 — standing-allowance elimination", main)
