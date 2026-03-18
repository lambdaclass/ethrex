#!/usr/bin/env python3
"""
solverCascade.py - Try automated solvers before resampling with LLM.
Handles 40-60% of simple cases mechanically.

Usage: solverCascade.py CONTEXT.json FILE.lean

Cascade: rfl -> simp -> ring -> linarith -> nlinarith -> omega -> exact? -> apply? -> aesop
"""

import json
import sys
import subprocess
import tempfile
from pathlib import Path
from typing import Optional

SOLVERS = [
    ("rfl", 1), ("simp", 2), ("ring", 2), ("linarith", 3),
    ("nlinarith", 4), ("omega", 3), ("exact?", 5), ("apply?", 5), ("aesop", 8),
]


def try_solver(file_path: Path, line: int, solver: str, timeout: int) -> Optional[str]:
    with open(file_path) as f:
        lines = f.readlines()

    target_line = lines[line - 1]
    if "sorry" in target_line:
        modified = target_line.replace("sorry", solver)
    elif "by" in target_line:
        indent = len(target_line) - len(target_line.lstrip())
        modified = target_line + " " * (indent + 2) + solver + "\n"
    else:
        return None

    lines[line - 1] = modified
    with tempfile.NamedTemporaryFile(mode='w', suffix='.lean', delete=False) as tmp:
        tmp.writelines(lines)
        tmp_path = tmp.name

    try:
        result = subprocess.run(
            ["lake", "env", "lean", tmp_path],
            capture_output=True, timeout=timeout, text=True
        )
        if result.returncode == 0:
            return subprocess.run(
                ["diff", "-u", str(file_path), tmp_path],
                capture_output=True, text=True
            ).stdout
        return None
    except subprocess.TimeoutExpired:
        return None
    finally:
        Path(tmp_path).unlink(missing_ok=True)


def run_cascade(context: dict, file_path: Path) -> Optional[str]:
    line = context.get("line", 0)
    error_type = context.get("errorType", "")

    if error_type in ["unknown_ident", "synth_implicit", "recursion_depth"]:
        return None

    print(f"Trying solver cascade at {file_path}:{line}")
    for solver, timeout in SOLVERS:
        print(f"  {solver}...", end=" ", flush=True)
        diff = try_solver(file_path, line, solver, timeout)
        if diff:
            print(f"OK")
            return diff
        print("no")
    return None


def main():
    if len(sys.argv) < 3:
        print("Usage: solverCascade.py CONTEXT.json FILE.lean", file=sys.stderr)
        sys.exit(1)

    with open(sys.argv[1]) as f:
        context = json.load(f)
    diff = run_cascade(context, Path(sys.argv[2]))
    if diff:
        print(diff)
        sys.exit(0)
    sys.exit(1)


if __name__ == "__main__":
    main()
