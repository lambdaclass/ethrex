#!/usr/bin/env python3
"""Name the hive cases that failed while hive was building its own fixture.

Some hive cases abandon themselves before the client under test is asked
anything: the simulator could not assemble the payload it intended to corrupt,
so there is no assertion about the client left to pass or fail. Those are
reported like any other failure, and counting them makes CI red for a defect in
the simulator's own setup.

Such a case is identified by the verdict hive writes to the simulation log,

    FAIL (<case name>): <signature>

and never by its name alone, so a genuine failure of the same test still counts.
The case's `summaryResult.log` offsets bracket its RPC traffic rather than this
line, which is why the whole log is scanned for the verdict instead.

Reads the suite JSON named by HIVE_JSON and the newline-separated signatures in
HIVE_SIGNATURES; writes the matching case names, one per line, to stdout.
"""

import json
import os
import pathlib
import re
import sys


def find_sim_log(json_path: pathlib.Path, rel: str) -> pathlib.Path | None:
    """Locate the simulation log, which hive names relative to the results dir."""
    if not rel:
        return None
    candidates = [
        pathlib.Path(rel),
        json_path.parent / rel,
        json_path.parent / pathlib.Path(rel).name,
    ]
    workspace = os.environ.get("HIVE_WORKSPACE_LOGS", "")
    if workspace:
        candidates.append(pathlib.Path(workspace) / pathlib.Path(rel).name)
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def main() -> int:
    json_path = pathlib.Path(os.environ["HIVE_JSON"])
    signatures = [s for s in os.environ.get("HIVE_SIGNATURES", "").splitlines() if s]
    if not signatures:
        return 0

    try:
        results = json.loads(json_path.read_text(errors="replace"))
    except (OSError, ValueError):
        # An unreadable result file is the caller's problem to report, not ours;
        # excluding nothing keeps every failure counted.
        return 0

    sim_log = find_sim_log(json_path, results.get("simLog") or "")
    if sim_log is None:
        return 0

    log = sim_log.read_text(errors="replace")
    verdicts: set[str] = set()
    for signature in signatures:
        pattern = r"^FAIL \((.+?)\): " + re.escape(signature)
        verdicts.update(re.findall(pattern, log, re.MULTILINE))
    if not verdicts:
        return 0

    # hive's verdict line drops the fork and client suffixes the result file
    # keeps ("... Invalid P9" against "... Invalid P9 (Paris) (ethrex)"), so the
    # case name is matched by prefix.
    cases = results.get("testCases") or {}
    entries = cases.values() if isinstance(cases, dict) else cases
    for case in entries:
        if (case.get("summaryResult") or {}).get("pass"):
            continue
        name = case.get("name") or ""
        if any(name.startswith(verdict) for verdict in verdicts):
            print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
