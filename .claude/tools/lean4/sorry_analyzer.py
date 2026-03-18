#!/usr/bin/env python3
"""
sorry_analyzer.py - Extract and analyze sorry statements in Lean 4 code

Usage:
    ./sorry_analyzer.py <file-or-directory> [--format=text|json|markdown] [--include-deps]

Finds all 'sorry' instances and extracts location, context, documentation, type info.
Excludes .lake/ dependencies by default.
"""

import re
import sys
import json
import os
from pathlib import Path
from dataclasses import dataclass, asdict
from typing import List, Optional


@dataclass
class Sorry:
    file: str
    line: int
    context_before: List[str]
    context_after: List[str]
    documentation: List[str]
    in_declaration: Optional[str] = None


def extract_declaration_name(lines, sorry_idx):
    for i in range(sorry_idx - 1, max(0, sorry_idx - 50), -1):
        match = re.match(r'^\s*(theorem|lemma|def|example)\s+(\w+)', lines[i])
        if match:
            return f"{match.group(1)} {match.group(2)}"
    return None


def extract_documentation(lines, sorry_idx):
    docs = []
    for i in range(sorry_idx + 1, min(len(lines), sorry_idx + 10)):
        line = lines[i].strip()
        if line.startswith('--'):
            comment = line[2:].strip()
            if any(kw in comment.upper() for kw in ['TODO', 'NOTE', 'FIXME', 'STRATEGY']):
                docs.append(comment)
        elif line and not line.startswith('--'):
            break
    return docs


def find_sorries_in_file(filepath):
    try:
        with open(filepath, 'r', encoding='utf-8') as f:
            lines = f.readlines()
    except Exception as e:
        print(f"Warning: Could not read {filepath}: {e}", file=sys.stderr)
        return []

    sorries = []
    for i, line in enumerate(lines):
        if 'sorry' in line:
            code_part = line.split('--')[0]
            if 'sorry' in code_part:
                sorries.append(Sorry(
                    file=str(filepath),
                    line=i + 1,
                    context_before=[l.rstrip() for l in lines[max(0, i-3):i]],
                    context_after=[l.rstrip() for l in lines[i+1:min(len(lines), i+4)]],
                    documentation=extract_documentation(lines, i),
                    in_declaration=extract_declaration_name(lines, i)
                ))
    return sorries


def find_sorries(target, include_deps=False):
    if target.is_file():
        if not include_deps and '.lake' in target.parts:
            return []
        return find_sorries_in_file(target)
    elif target.is_dir():
        sorries = []
        for root, dirs, files in os.walk(target):
            if not include_deps:
                dirs[:] = [d for d in dirs if d != '.lake']
            for fn in files:
                if fn.endswith('.lean'):
                    sorries.extend(find_sorries_in_file(Path(root) / fn))
        return sorries
    raise ValueError(f"{target} is not a file or directory")


def format_text(sorries):
    out = [f"Found {len(sorries)} sorry statement(s)", "=" * 60]
    for i, s in enumerate(sorries, 1):
        out.append(f"\n[{i}] {s.file}:{s.line}")
        if s.in_declaration:
            out.append(f"    In: {s.in_declaration}")
        if s.documentation:
            for doc in s.documentation:
                out.append(f"    > {doc}")
        out.append("    Context:")
        for line in s.context_before[-2:]:
            out.append(f"      {line}")
        out.append(f"      >>> SORRY <<<")
        for line in s.context_after[:2]:
            out.append(f"      {line}")
    return "\n".join(out)


def format_json(sorries):
    return json.dumps({'total': len(sorries), 'sorries': [asdict(s) for s in sorries]}, indent=2)


def format_markdown(sorries):
    out = [f"# Sorry Analysis\n\n**Total:** {len(sorries)}\n"]
    by_file = {}
    for s in sorries:
        by_file.setdefault(s.file, []).append(s)
    for fp, ss in sorted(by_file.items()):
        out.append(f"## {fp} ({len(ss)} sorries)\n")
        for s in ss:
            out.append(f"### Line {s.line}")
            if s.in_declaration:
                out.append(f"**In:** `{s.in_declaration}`\n")
            out.append("```lean")
            for line in s.context_before[-2:]:
                out.append(line)
            out.append("sorry")
            for line in s.context_after[:2]:
                out.append(line)
            out.append("```\n")
    return "\n".join(out)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    target = Path(sys.argv[1])
    fmt = 'text'
    include_deps = False
    for arg in sys.argv[2:]:
        if arg.startswith('--format='):
            fmt = arg.split('=')[1]
        elif arg == '--include-deps':
            include_deps = True

    if not target.exists():
        print(f"Error: {target} does not exist", file=sys.stderr)
        sys.exit(1)

    sorries = find_sorries(target, include_deps)
    if fmt == 'json':
        print(format_json(sorries))
    elif fmt == 'markdown':
        print(format_markdown(sorries))
    else:
        print(format_text(sorries))
    sys.exit(0 if not sorries else 1)


if __name__ == '__main__':
    main()
