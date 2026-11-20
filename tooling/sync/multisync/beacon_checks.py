"""Consensus-client checks for multisync, over the standard Beacon API.

Client-agnostic: every check reads endpoints every CL serves, so the same code
judges Lighthouse, Prysm and ethlambda. The monitor calls `update` on every
poll, `start_window` when the EL enters block processing, and `end_checks`
when the EL's window ends. Every failure message starts with `CL:`.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Callable, Optional

import requests

from .util import fmt_time


@dataclass(frozen=True)
class ClTimeouts:
    startup: int = 15 * 60
    unresponsive: int = 5 * 60
    head_stall: int = 10 * 60
    provider_interval: int = 60


class BeaconApi:
    """GET-only Beacon API client; every method answers None on any failure."""

    def __init__(self, base_url: str, timeout: float = 5):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def get(self, path: str) -> Optional[dict]:
        try:
            response = requests.get(
                f"{self.base_url}{path}", headers={"Accept": "application/json"}, timeout=self.timeout
            )
            if response.status_code != 200:
                return None
            body = response.json()
            return body if isinstance(body, dict) else None
        except Exception:
            return None

    def syncing(self) -> Optional[dict]:
        body = self.get("/eth/v1/node/syncing")
        data = body.get("data") if body else None
        return data if isinstance(data, dict) else None

    def finalized_checkpoint(self, state_id: str = "head") -> Optional[tuple[int, str]]:
        body = self.get(f"/eth/v1/beacon/states/{state_id}/finality_checkpoints")
        try:
            finalized = body["data"]["finalized"]
            return int(finalized["epoch"]), normalize_root(finalized["root"])
        except (TypeError, KeyError, ValueError, AttributeError):
            return None

    def block(self, block_id: str) -> Optional[dict]:
        return self.get(f"/eth/v2/beacon/blocks/{block_id}")


def normalize_root(root: str) -> str:
    """Lowercase, `0x`-prefixed hex; checkpointz omits the prefix on some routes."""
    root = root.strip().lower()
    return root if root.startswith("0x") else f"0x{root}"


def fcu_finalized_hash(block: dict) -> Optional[str]:
    """The execution block hash a CL sends as fcU `finalizedBlockHash` for this block.

    Before Gloas it is the block's own payload hash. From Gloas on the payload
    arrives separately, and the spec's finalized hash is the bid's parent hash.
    Which field the body carries decides, so no fork schedule is needed.
    """
    try:
        body = block["data"]["message"]["body"]
        if "execution_payload" in body:
            return normalize_root(body["execution_payload"]["block_hash"])
        if "signed_execution_payload_bid" in body:
            return normalize_root(body["signed_execution_payload_bid"]["message"]["parent_block_hash"])
    except (TypeError, KeyError, AttributeError):
        return None
    return None


@dataclass
class ClState:
    status: str = "waiting"  # waiting | following | failed
    error: str = ""
    started_at: float = 0
    last_ok: float = 0
    head_slot: int = 0
    head_changed_at: float = 0
    # Every finalized checkpoint the CL reported this cycle, by epoch.
    finalized: dict[int, str] = field(default_factory=dict)
    finalized_epoch: int = 0
    syncing: Optional[dict] = None
    in_window: bool = False
    window_start_epoch: Optional[int] = None
    window_match_epoch: Optional[int] = None
    provider_last_poll: Optional[float] = None
    provider_failures: int = 0
    window_provider_polls: int = 0
    window_provider_failures: int = 0
    provider_epoch: Optional[int] = None
    provider_match_epoch: Optional[int] = None
    hash_note: str = ""


def update(
    state: ClState,
    api: BeaconApi,
    provider: BeaconApi,
    now: float,
    timeouts: ClTimeouts,
    exit_code: Callable[[], Optional[int]] = lambda: None,
) -> bool:
    """One poll: liveness, head progress, finalized history, provider match.

    Returns True when the change is worth printing.
    """
    if state.status == "failed":
        return False
    if not state.started_at:
        state.started_at = now

    syncing = api.syncing()
    head = _as_int(syncing.get("head_slot")) if syncing else None
    if head is None:
        if state.status == "waiting":
            code = exit_code()
            if code is not None:
                return _fail(state, f"CL: container exited with code {code} before its Beacon API answered")
            if now - state.started_at > timeouts.startup:
                return _fail(state, f"CL: no Beacon API answer after {fmt_time(timeouts.startup)}")
            return False
        if now - state.last_ok > timeouts.unresponsive:
            code = exit_code()
            exited = f", container exited with code {code}" if code is not None else ""
            return _fail(state, f"CL: unresponsive for {fmt_time(now - state.last_ok)}{exited}")
        return False

    changed = False
    state.last_ok = now
    state.syncing = syncing
    if state.status == "waiting":
        state.status, state.head_slot, state.head_changed_at = "following", head, now
        changed = True
    elif head > state.head_slot:
        state.head_slot, state.head_changed_at = head, now
    elif now - state.head_changed_at > timeouts.head_stall:
        return _fail(state, f"CL: head stuck at slot {state.head_slot} for {fmt_time(now - state.head_changed_at)}")

    checkpoint = api.finalized_checkpoint("head")
    if checkpoint:
        epoch, root = checkpoint
        state.finalized[epoch] = root
        if state.in_window and state.window_start_epoch is None:
            state.window_start_epoch = epoch
        if epoch > state.finalized_epoch:
            state.finalized_epoch = epoch
            changed = True

    if state.provider_last_poll is None or now - state.provider_last_poll >= timeouts.provider_interval:
        state.provider_last_poll = now
        changed = _check_provider(state, provider) or changed
    return changed


def start_window(state: ClState) -> None:
    """The EL entered block processing: finality and provider matches count from here."""
    state.in_window = True
    # Unset when nothing was observed yet: `update` takes the first observation.
    state.window_start_epoch = state.finalized_epoch if state.finalized else None
    state.window_match_epoch = None
    state.window_provider_polls = 0
    state.window_provider_failures = 0


def end_checks(
    state: ClState,
    api: BeaconApi,
    el_finalized_hash: Callable[[], Optional[str]],
    sleep: Callable[[float], None] = time.sleep,
    attempts: int = 10,
    delay: float = 4.0,
) -> Optional[str]:
    """Checks for the end of the EL's block-processing window; None when all pass."""
    if state.status != "following":
        return state.error or "CL: never answered its Beacon API"
    if state.window_start_epoch is None:
        return "CL: no finalized checkpoint observed during the window"
    if state.finalized_epoch <= state.window_start_epoch:
        return (
            f"CL: finalized epoch did not advance during the window "
            f"(stuck at {state.finalized_epoch}; provider at {state.provider_epoch})"
        )
    if state.window_match_epoch is None:
        return (
            f"CL: no finalized checkpoint matched the provider during the window "
            f"({state.window_provider_failures} of {state.window_provider_polls} provider polls failed)"
        )

    # Each attempt re-reads everything the API says: a slot of EL SYNCING flips
    # is_optimistic for a moment, and the EL learns its finalized block from the
    # next fcU. Three 12 s slots fit in the default budget.
    reason = ""
    for attempt in range(attempts):
        if attempt:
            sleep(delay)
        reason = _attempt(state, api, el_finalized_hash)
        if not reason:
            return None
    return reason


def _attempt(state: ClState, api: BeaconApi, el_finalized_hash: Callable[[], Optional[str]]) -> str:
    """One read of the health flags and the EL/CL finalized hash; "" when both pass."""
    syncing = api.syncing()
    if syncing is None:
        return "CL: Beacon API did not answer at the end of the window"
    for key in ("is_syncing", "is_optimistic", "el_offline"):
        # A CL that omits a flag is taken as reporting false.
        if _is_true(syncing.get(key)):
            return f"CL: {key} is true at the end of the window"

    # fcU carries fork choice's finalized block, which the head state's own
    # finalized checkpoint lags from an epoch boundary until the next block.
    candidates: dict[str, Optional[str]] = {}
    checkpoint = api.finalized_checkpoint("head")
    sources = [("finalized", "finalized")]
    if checkpoint:
        sources.append((checkpoint[1], f"head state's finalized {checkpoint[1]}"))
    for block_id, label in sources:
        block = api.block(block_id)
        candidates[label] = fcu_finalized_hash(block) if block else None
    el_hash = el_finalized_hash()
    if el_hash and normalize_root(el_hash) in candidates.values():
        state.hash_note = "EL/CL hash ✓"
        return ""
    state.hash_note = "EL/CL hash ✗"
    named = ", ".join(f"{label} names {hash_}" for label, hash_ in candidates.items())
    return f"CL: EL/CL finalized hash mismatch: CL {named}; EL finalized is {el_hash}"


def describe(state: ClState) -> str:
    """One-line CL summary for the status screen, the run log and Slack."""
    if state.status == "waiting":
        return "waiting for its Beacon API"
    finalized = f"finalized {state.finalized_epoch}"
    if state.window_start_epoch is not None:
        finalized += f" (+{state.finalized_epoch - state.window_start_epoch})"
    parts = [state.status, f"head {state.head_slot}", finalized]
    if state.provider_match_epoch is not None:
        parts.append(f"provider ✓ @{state.provider_match_epoch}")
    if state.hash_note:
        parts.append(state.hash_note)
    return ", ".join(parts)


def _check_provider(state: ClState, provider: BeaconApi) -> bool:
    """Compare the provider's finalized checkpoint with ours at the same epoch.

    The epoch->root history makes this work whether the provider is ahead of
    us or behind, and whichever checkpoint its finalized state reports.
    """
    checkpoint = provider.finalized_checkpoint("finalized")
    if state.in_window:
        state.window_provider_polls += 1
    if checkpoint is None:
        state.provider_failures += 1
        if state.in_window:
            state.window_provider_failures += 1
        return False
    epoch, root = checkpoint
    state.provider_epoch = epoch
    ours = state.finalized.get(epoch)
    if ours is None:
        return False
    if ours != root:
        return _fail(state, f"CL: finalized root at epoch {epoch} is {ours}, the provider has {root}")
    first_match = state.provider_match_epoch is None
    state.provider_match_epoch = epoch
    if state.in_window:
        state.window_match_epoch = epoch
    return first_match


def _fail(state: ClState, error: str) -> bool:
    state.status, state.error = "failed", error
    return True


def _as_int(value: Any) -> Optional[int]:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _is_true(value: Any) -> bool:
    return value is True or (isinstance(value, str) and value.lower() == "true")
