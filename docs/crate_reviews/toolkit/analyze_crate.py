#!/usr/bin/env python3
"""Utility to collect complexity and concurrency signals for an ethrex crate."""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys
from collections import Counter, defaultdict
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import dataclass
from typing import TypedDict, cast


DEFAULT_BRANCH_MARKERS = [
    " if",
    " else if",
    " match",
    " for",
    " while",
    " loop",
    "&&",
    "||",
]

DEFAULT_KEYWORD_PATTERNS: Mapping[str, str] = {
    "async_fn": r"^\s*(pub\s+)?(async\s+)(const\s+)?fn\s+",
    "await": r"\\.await",
    "tokio_spawn": r"tokio::spawn",
    "spawn_blocking": r"spawn_blocking",
    "tokio_mutex": r"tokio::sync::Mutex",
    "std_mutex": r"std::sync::Mutex",
    "tokio_rwlock": r"tokio::sync::RwLock",
    "std_rwlock": r"std::sync::RwLock",
    "arc": r"Arc<",
    "atomic": r"Atomic",
    "mpsc": r"tokio::sync::mpsc",
    "broadcast": r"tokio::sync::broadcast",
    "spawned": r"spawned_concurrency",
    "genserver": r"GenServer",
    "await_mutex_guard": r"MutexGuard[\w:<>, ]*.*\\.await|\\.await.*MutexGuard",
    "arc_mutex_receiver": r"Arc<Mutex<mpsc::Receiver",
}

FN_REGEX = re.compile(r"^\s*(pub\s+)?(async\s+)?(const\s+)?fn\s+(?P<name>\w+)")
COMMENT_REGEX = re.compile(r"^\s*//")
BLOCK_COMMENT_START = re.compile(r"/\*")
BLOCK_COMMENT_END = re.compile(r"\*/")


class Totals(TypedDict):
    files: int
    total_lines: int
    code_lines: int
    function_count: int
    complex_function_count: int


class ComplexFunctionEntry(TypedDict):
    file: str
    name: str
    lines: int
    branches: int


class Summary(TypedDict):
    crate: str
    totals: Totals
    keyword_totals: dict[str, int]
    keyword_hotspots: dict[str, list[tuple[str, int]]]
    complex_functions: list[ComplexFunctionEntry]


@dataclass
class CLIArgs:
    crate_path: str
    exclude: list[str]
    exclude_prefix: list[str]
    keyword: list[str]
    complex_line_threshold: int
    branch_threshold: int
    combined_line_threshold: int
    combined_branch_threshold: int
    top_complex: int
    json: bool


def _ensure_str(value: object, name: str) -> str:
    if isinstance(value, str):
        return value
    raise TypeError(f"{name} must be a string")


def _ensure_str_list(value: object, name: str) -> list[str]:
    if value is None:
        return []
    if isinstance(value, list):
        value_list = cast(list[object], value)
        result: list[str] = []
        for item in value_list:
            if not isinstance(item, str):
                raise TypeError(f"{name} must contain only strings")
            result.append(item)
        return result
    raise TypeError(f"{name} must be a list of strings or None")


def _ensure_int(value: object, name: str) -> int:
    if isinstance(value, int):
        return value
    raise TypeError(f"{name} must be an integer")


def _ensure_bool(value: object, name: str) -> bool:
    if isinstance(value, bool):
        return value
    raise TypeError(f"{name} must be a boolean")


def parse_args(argv: Sequence[str] | None = None) -> CLIArgs:
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument(
        "crate_path", help="Path to the crate root (e.g. crates/networking/p2p)"
    )
    _ = parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        help="Relative directory names to skip everywhere (can be passed multiple times)",
    )
    _ = parser.add_argument(
        "--exclude-prefix",
        action="append",
        default=[],
        metavar="PATH",
        help=(
            "Relative path prefixes to skip (e.g. 'levm' to drop crates/vm/levm without "
            "touching backends/levm)"
        ),
    )
    _ = parser.add_argument(
        "--keyword",
        action="append",
        default=[],
        metavar="LABEL=REGEX",
        help="Additional keyword pattern to tally (may be repeated)",
    )
    _ = parser.add_argument(
        "--complex-line-threshold",
        type=int,
        default=60,
        help="Function line count threshold for complexity flagging",
    )
    _ = parser.add_argument(
        "--branch-threshold",
        type=int,
        default=6,
        help="Branch keyword threshold for complexity flagging",
    )
    _ = parser.add_argument(
        "--combined-line-threshold",
        type=int,
        default=40,
        help="Minimum lines for the combined line+branch heuristic",
    )
    _ = parser.add_argument(
        "--combined-branch-threshold",
        type=int,
        default=3,
        help="Minimum branches for the combined line+branch heuristic",
    )
    _ = parser.add_argument(
        "--top-complex",
        type=int,
        default=20,
        help="Number of complex functions to list explicitly",
    )
    _ = parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON instead of the default human-readable summary",
    )
    parsed = parser.parse_args(argv)
    crate_path = _ensure_str(cast(object, parsed.crate_path), "crate_path")
    exclude = _ensure_str_list(cast(object, parsed.exclude), "exclude")
    exclude_prefix = _ensure_str_list(
        cast(object, parsed.exclude_prefix), "exclude_prefix"
    )
    keyword = _ensure_str_list(cast(object, parsed.keyword), "keyword")
    complex_line_threshold = _ensure_int(
        cast(object, parsed.complex_line_threshold), "complex_line_threshold"
    )
    branch_threshold = _ensure_int(
        cast(object, parsed.branch_threshold), "branch_threshold"
    )
    combined_line_threshold = _ensure_int(
        cast(object, parsed.combined_line_threshold), "combined_line_threshold"
    )
    combined_branch_threshold = _ensure_int(
        cast(object, parsed.combined_branch_threshold), "combined_branch_threshold"
    )
    top_complex = _ensure_int(cast(object, parsed.top_complex), "top_complex")
    json_output = _ensure_bool(cast(object, parsed.json), "json")

    return CLIArgs(
        crate_path=crate_path,
        exclude=exclude,
        exclude_prefix=exclude_prefix,
        keyword=keyword,
        complex_line_threshold=complex_line_threshold,
        branch_threshold=branch_threshold,
        combined_line_threshold=combined_line_threshold,
        combined_branch_threshold=combined_branch_threshold,
        top_complex=top_complex,
        json=json_output,
    )


def load_keyword_patterns(extra_patterns: Iterable[str]) -> dict[str, re.Pattern[str]]:
    patterns = dict(DEFAULT_KEYWORD_PATTERNS)
    for item in extra_patterns:
        if "=" not in item:
            raise ValueError(f"Custom keyword '{item}' must use LABEL=REGEX syntax")
        label, pattern = item.split("=", 1)
        patterns[label.strip()] = pattern.strip()
    return {label: re.compile(pattern) for label, pattern in patterns.items()}


def normalize_prefixes(raw_prefixes: Iterable[str]) -> list[tuple[str, ...]]:
    prefixes: list[tuple[str, ...]] = []
    for raw in raw_prefixes:
        prefix = pathlib.PurePosixPath(raw.strip())
        if prefix.is_absolute():
            raise ValueError(f"Prefix '{raw}' must be relative to the crate root")
        parts = tuple(part for part in prefix.parts if part not in ("", "."))
        if not parts:
            raise ValueError(
                "Empty prefix provided; remove redundant '/' or '.' entries"
            )
        prefixes.append(parts)
    return prefixes


def should_exclude(
    path: pathlib.Path,
    crate_root: pathlib.Path,
    name_exclusions: set[str],
    prefix_exclusions: list[tuple[str, ...]],
) -> bool:
    if not name_exclusions and not prefix_exclusions:
        return False
    try:
        rel_parts = path.relative_to(crate_root).parts
    except ValueError:
        return False

    if name_exclusions and any(part in name_exclusions for part in rel_parts[:-1]):
        return True

    if prefix_exclusions:
        for prefix_parts in prefix_exclusions:
            if rel_parts[: len(prefix_parts)] == prefix_parts:
                return True

    return False


def count_keyword_occurrences(
    text: str, compiled_patterns: Mapping[str, re.Pattern[str]]
) -> dict[str, int]:
    totals: dict[str, int] = {}
    for label, pattern in compiled_patterns.items():
        totals[label] = len(pattern.findall(text))
    return totals


def _new_counter() -> Counter[str]:
    return Counter()


def analyze(
    crate_root: pathlib.Path,
    name_exclusions: set[str],
    prefix_exclusions: list[tuple[str, ...]],
    compiled_patterns: Mapping[str, re.Pattern[str]],
    *,
    complex_line_threshold: int,
    branch_threshold: int,
    combined_line_threshold: int,
    combined_branch_threshold: int,
    top_complex: int,
) -> Summary:
    files = [
        path
        for path in sorted(crate_root.rglob("*.rs"))
        if not should_exclude(path, crate_root, name_exclusions, prefix_exclusions)
    ]

    totals: Totals = {
        "files": len(files),
        "total_lines": 0,
        "code_lines": 0,
        "function_count": 0,
        "complex_function_count": 0,
    }

    keyword_totals: Counter[str] = Counter()
    keyword_by_file: defaultdict[pathlib.Path, Counter[str]] = defaultdict(_new_counter)
    complex_fns: list[ComplexFunctionEntry] = []

    for path in files:
        text = path.read_text()
        lines = text.splitlines()
        totals["total_lines"] += len(lines)

        code_lines = 0
        in_block_comment = False
        for line in lines:
            stripped = line.strip()
            if not stripped:
                continue
            if in_block_comment:
                code_lines += 0
            if BLOCK_COMMENT_START.search(line):
                in_block_comment = True
            if not COMMENT_REGEX.match(line) and not in_block_comment:
                code_lines += 1
            if in_block_comment and BLOCK_COMMENT_END.search(line):
                in_block_comment = False
        totals["code_lines"] += code_lines

        kw_counts = count_keyword_occurrences(text, compiled_patterns)
        keyword_totals.update(kw_counts)
        keyword_by_file[path].update(kw_counts)

        i = 0
        while i < len(lines):
            match = FN_REGEX.match(lines[i])
            if not match:
                i += 1
                continue

            name = match.group("name")
            body_lines = 0
            branches = 0
            depth = 0
            j = i

            # Scan to first opening brace
            while j < len(lines):
                line = lines[j]
                if "{" in line:
                    depth += line.count("{") - line.count("}")
                    body_lines += 1
                    break
                j += 1
            j += 1

            while j < len(lines) and depth > 0:
                line = lines[j]
                depth += line.count("{") - line.count("}")
                body_lines += 1
                branches += sum(
                    1 for marker in DEFAULT_BRANCH_MARKERS if marker in line
                )
                j += 1

            totals["function_count"] += 1
            is_complex = (
                body_lines >= complex_line_threshold
                or branches >= branch_threshold
                or (
                    body_lines >= combined_line_threshold
                    and branches >= combined_branch_threshold
                )
            )
            if is_complex:
                totals["complex_function_count"] += 1
                complex_fns.append(
                    {
                        "file": str(path),
                        "name": name,
                        "lines": body_lines,
                        "branches": branches,
                    }
                )
            i = j

    complex_fns.sort(
        key=lambda entry: (entry["lines"], entry["branches"]), reverse=True
    )

    # Compute keyword hotspots per file
    keyword_hotspots: dict[str, list[tuple[str, int]]] = {}
    for label in compiled_patterns:
        per_file = sorted(
            (
                (str(path), counts[label])
                for path, counts in keyword_by_file.items()
                if counts[label]
            ),
            key=lambda item: item[1],
            reverse=True,
        )
        if per_file:
            keyword_hotspots[label] = per_file[: min(5, len(per_file))]

    return {
        "crate": str(crate_root),
        "totals": totals,
        "keyword_totals": dict(keyword_totals),
        "keyword_hotspots": keyword_hotspots,
        "complex_functions": complex_fns[:top_complex],
    }


def emit_human(summary: Summary) -> None:
    totals: Totals = summary["totals"]
    print(f"Crate: {summary['crate']}")
    totals_line = (
        "Totals: {files} files | {total_lines} lines | {code_lines} code lines | "
        "{function_count} functions ({complex_function_count} complex)"
    ).format(
        files=totals["files"],
        total_lines=totals["total_lines"],
        code_lines=totals["code_lines"],
        function_count=totals["function_count"],
        complex_function_count=totals["complex_function_count"],
    )
    print(totals_line)
    print("\nKeyword counts:")
    keyword_items = sorted(
        summary["keyword_totals"].items(), key=lambda item: (-item[1], item[0])
    )
    for label, count in keyword_items:
        print(f"  - {label}: {count}")

    if summary["keyword_hotspots"]:
        print("\nKeyword hotspots (top ≤5 files per signal):")
        for label, entries in summary["keyword_hotspots"].items():
            formatted = ", ".join(f"{path} ({count})" for path, count in entries)
            print(f"  - {label}: {formatted}")

    if summary["complex_functions"]:
        print("\nTop complex functions:")
        for entry in summary["complex_functions"]:
            print(
                f"  - {entry['file']}::{entry['name']} — {entry['lines']} lines, {entry['branches']} branches"
            )


def main() -> None:
    args = parse_args()
    crate_root = pathlib.Path(args.crate_path).resolve()
    if not crate_root.exists():
        raise SystemExit(f"Crate path '{crate_root}' does not exist")

    exclusions = set(args.exclude)
    prefix_exclusions = normalize_prefixes(args.exclude_prefix)
    compiled_patterns = load_keyword_patterns(args.keyword)

    summary = analyze(
        crate_root,
        exclusions,
        prefix_exclusions,
        compiled_patterns,
        complex_line_threshold=args.complex_line_threshold,
        branch_threshold=args.branch_threshold,
        combined_line_threshold=args.combined_line_threshold,
        combined_branch_threshold=args.combined_branch_threshold,
        top_complex=args.top_complex,
    )

    if args.json:
        json.dump(summary, fp=sys.stdout, indent=2)
        print()
    else:
        emit_human(summary)


if __name__ == "__main__":
    import sys

    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
