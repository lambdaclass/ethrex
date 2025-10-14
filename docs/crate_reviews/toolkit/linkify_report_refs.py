#!/usr/bin/env python3
"""Convert crate review line references into GitHub permalinks.

The script scans Markdown crate review reports for tokens such as
```
crates/blockchain/blockchain.rs:184
payload.rs:320-327
```
and replaces them with `[token](permalink)` entries that point at the
commit hash listed near the top of the report. The commit is expected to
appear as a line starting with `Commit:` (the standard review template).

Usage examples:
    docs/crate_reviews/toolkit/linkify_report_refs.py docs/crate_reviews/ethrex_blockchain_review.md
    docs/crate_reviews/toolkit/linkify_report_refs.py docs/crate_reviews/ethrex_*_review.md
    docs/crate_reviews/toolkit/linkify_report_refs.py docs/crate_reviews

When a directory is supplied, every `*.md` file under it is processed.
Existing Markdown links are left untouched so the script remains
idempotent.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import cast

_BASE_URL = "https://github.com/lambdaclass/ethrex/blob"
_TOKEN_PATTERN = re.compile(
    r"(?P<tick>`?)(?P<token>(?:[A-Za-z0-9_.-]+/)*[A-Za-z0-9_.-]+\.[A-Za-z0-9_.-]+:[0-9]+(?:-[0-9]+)?)(?P=tick)"
)
_COMMIT_PATTERN = re.compile(r"^Commit:\s*([0-9a-fA-F]{7,40})", re.MULTILINE)
_TARGET_PATTERN = re.compile(r"Target crate:\s*`([^`]+)`")


def _collect_targets(targets: Sequence[str]) -> Iterable[Path]:
    for target in targets:
        path = Path(target)
        if path.is_dir():
            yield from sorted(path.rglob("*.md"))
        else:
            yield path


@dataclass
class ToolkitArgs:
    targets: list[str]
    dry_run: bool
    verbose: bool


def _resolve_repo_root(script_path: Path) -> Path:
    try:
        return script_path.resolve().parents[3]
    except IndexError as err:  # pragma: no cover - defensive guard
        raise RuntimeError(
            "Unexpected toolkit location; cannot locate repo root"
        ) from err


def _resolve_path(candidate: str, base_path: str | None, repo_root: Path) -> str:
    path_obj = Path(candidate)
    if (repo_root / path_obj).exists():
        return path_obj.as_posix()
    if base_path:
        combined = Path(base_path) / path_obj
        if (repo_root / combined).exists():
            return combined.as_posix()
        return combined.as_posix()
    return path_obj.as_posix()


def _linkify(text: str, repo_root: Path) -> tuple[str, int]:
    commit_match = _COMMIT_PATTERN.search(text)
    if not commit_match:
        return text, 0
    commit = commit_match.group(1)

    target_match = _TARGET_PATTERN.search(text)
    base_path = target_match.group(1) if target_match else None

    replacements = 0

    def repl(match: re.Match[str]) -> str:
        nonlocal replacements
        token = match.group("token")
        tick = match.group("tick")
        start, end = match.span()
        if (
            tick == ""
            and start > 0
            and text[start - 1] == "["
            and end < len(text)
            and text[end] == "]"
        ):
            return match.group(0)
        path_part, line_part = token.rsplit(":", 1)
        full_path = _resolve_path(path_part, base_path, repo_root)
        if "-" in line_part:
            start_line, end_line = [
                segment.strip() for segment in line_part.split("-", 1)
            ]
            anchor = f"#L{start_line}-L{end_line}"
        else:
            anchor = f"#L{line_part.strip()}"
        url = f"{_BASE_URL}/{commit}/{full_path}{anchor}"
        replacements += 1
        return f"[{token}]({url})"

    updated_text = _TOKEN_PATTERN.sub(repl, text)
    return updated_text, replacements


def _process_file(path: Path, repo_root: Path, dry_run: bool, verbose: bool) -> int:
    try:
        original = path.read_text()
    except FileNotFoundError:
        print(f"warning: {path} does not exist", file=sys.stderr)
        return 0

    updated, replacements = _linkify(original, repo_root)
    if replacements and not dry_run:
        _ = path.write_text(updated)
    if verbose and replacements:
        action = "would update" if dry_run else "updated"
        print(
            f"{action} {path} ({replacements} link{'s' if replacements != 1 else ''})"
        )
    return replacements


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument(
        "targets",
        nargs="*",
        default=["docs/crate_reviews"],
        help="Paths to review Markdown files or directories (defaults to docs/crate_reviews)",
    )
    _ = parser.add_argument(
        "-n",
        "--dry-run",
        action="store_true",
        help="Scan and report replacements without writing changes",
    )
    _ = parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="Print a line per file that receives replacements",
    )
    parsed = parser.parse_args(argv)
    targets = list(cast(list[str], parsed.targets))
    dry_run = cast(bool, parsed.dry_run)
    verbose = cast(bool, parsed.verbose)
    args = ToolkitArgs(targets=targets, dry_run=dry_run, verbose=verbose)

    repo_root = _resolve_repo_root(Path(__file__))
    total = 0
    for target in _collect_targets(args.targets):
        total += _process_file(target, repo_root, args.dry_run, args.verbose)

    if args.verbose:
        summary = "would update" if args.dry_run else "updated"
        print(
            f"{summary} {total} link{'s' if total != 1 else ''} across {len(args.targets)} target(s)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
