#!/usr/bin/env python3
"""
Parse Firefox profiler JSON files from SP1 and output statistics in a text format.

Usage:
    python3 parse_sp1_profile.py <profile.json> [--top N] [--output <file>]

The Firefox profiler format is used by SP1 for profiling zkVM execution.
"""

import json
import argparse
import sys
from collections import defaultdict
from typing import Optional


def demangle_rust_name(name: str) -> str:
    """Simplify Rust function names by removing hashes and common prefixes."""
    # Remove hash suffix (e.g., ::h588dd96c4cbebf74)
    if "::h" in name and len(name.split("::h")[-1]) == 16:
        parts = name.rsplit("::h", 1)
        if len(parts[1]) == 16 and all(c in "0123456789abcdef" for c in parts[1]):
            name = parts[0]
    return name


def parse_profile(profile_path: str, top_n: int = 50, output_path: Optional[str] = None):
    """Parse a Firefox profiler JSON file and output statistics."""

    with open(profile_path, 'r') as f:
        data = json.load(f)

    thread = data['threads'][0]
    string_table = thread['stringTable']
    frame_table = thread['frameTable']['data']
    stack_table = thread['stackTable']['data']
    samples = thread['samples']['data']

    # Build frame_id -> function_name mapping
    # frameTable schema: location(0), relevantForJS(1), ...
    frame_to_name = {}
    for frame_idx, frame in enumerate(frame_table):
        string_idx = frame[0]  # location field
        if string_idx < len(string_table):
            frame_to_name[frame_idx] = demangle_rust_name(string_table[string_idx])
        else:
            frame_to_name[frame_idx] = f"<unknown:{string_idx}>"

    # Build stack_id -> (parent_stack_id, frame_id) mapping
    # stackTable schema: prefix(0), frame(1)
    stack_to_parent_frame = {}
    for stack_idx, stack in enumerate(stack_table):
        parent_idx, frame_idx = stack[0], stack[1]
        stack_to_parent_frame[stack_idx] = (parent_idx, frame_idx)

    # Aggregate cycles by function
    # samples schema: stack(0), time(1), eventDelay(2), threadCPUDelta(3)
    inclusive_cycles = defaultdict(int)  # Time in function + callees
    exclusive_cycles = defaultdict(int)  # Time in function only (leaf)
    call_counts = defaultdict(int)

    total_cycles = 0

    for sample in samples:
        stack_idx = sample[0]
        cycles = sample[3] if len(sample) > 3 else 1
        total_cycles += cycles

        if stack_idx is None:
            continue

        # Walk up the stack chain
        current_stack = stack_idx
        frames_in_stack = []
        seen_frames = set()

        while current_stack is not None and current_stack in stack_to_parent_frame:
            parent_idx, frame_idx = stack_to_parent_frame[current_stack]
            if frame_idx not in seen_frames:
                frames_in_stack.append(frame_idx)
                seen_frames.add(frame_idx)
            current_stack = parent_idx

        # First frame (leaf) gets exclusive cycles
        if frames_in_stack:
            leaf_name = frame_to_name.get(frames_in_stack[0], "<unknown>")
            exclusive_cycles[leaf_name] += cycles

        # All frames get inclusive cycles
        for frame_idx in frames_in_stack:
            name = frame_to_name.get(frame_idx, "<unknown>")
            inclusive_cycles[name] += cycles
            call_counts[name] += 1

    # Sort by inclusive cycles
    sorted_inclusive = sorted(inclusive_cycles.items(), key=lambda x: -x[1])[:top_n]
    sorted_exclusive = sorted(exclusive_cycles.items(), key=lambda x: -x[1])[:top_n]

    # Generate output
    output_lines = []
    output_lines.append(f"SP1 Profile Analysis")
    output_lines.append(f"=" * 60)
    output_lines.append(f"Source: {profile_path}")
    output_lines.append(f"Total samples: {len(samples):,}")
    output_lines.append(f"Total cycles: {total_cycles:,}")
    output_lines.append("")

    output_lines.append(f"TOP {top_n} FUNCTIONS BY INCLUSIVE CYCLES")
    output_lines.append(f"(cycles in function + all callees)")
    output_lines.append("-" * 100)
    output_lines.append(f"{'CYCLES':>15} {'%':>7} {'CALLS':>12}  FUNCTION")
    output_lines.append("-" * 100)

    for name, cycles in sorted_inclusive:
        pct = (cycles / total_cycles * 100) if total_cycles > 0 else 0
        calls = call_counts[name]
        # Truncate long function names
        display_name = name[:70] + "..." if len(name) > 73 else name
        output_lines.append(f"{cycles:>15,} {pct:>6.2f}% {calls:>12,}  {display_name}")

    output_lines.append("")
    output_lines.append(f"TOP {top_n} FUNCTIONS BY EXCLUSIVE CYCLES")
    output_lines.append(f"(cycles in function only, excluding callees)")
    output_lines.append("-" * 100)
    output_lines.append(f"{'CYCLES':>15} {'%':>7}  FUNCTION")
    output_lines.append("-" * 100)

    for name, cycles in sorted_exclusive:
        pct = (cycles / total_cycles * 100) if total_cycles > 0 else 0
        display_name = name[:80] + "..." if len(name) > 83 else name
        output_lines.append(f"{cycles:>15,} {pct:>6.2f}%  {display_name}")

    output_text = "\n".join(output_lines)

    if output_path:
        with open(output_path, 'w') as f:
            f.write(output_text)
        print(f"Output written to {output_path}")
    else:
        print(output_text)

    return {
        'total_cycles': total_cycles,
        'total_samples': len(samples),
        'inclusive': dict(sorted_inclusive),
        'exclusive': dict(sorted_exclusive),
    }


def compare_profiles(profile1_path: str, profile2_path: str, top_n: int = 30):
    """Compare two profiles and show differences."""

    print(f"Parsing {profile1_path}...")
    result1 = parse_profile(profile1_path, top_n=1000, output_path=None)

    print(f"Parsing {profile2_path}...")
    result2 = parse_profile(profile2_path, top_n=1000, output_path=None)

    print("\n" + "=" * 80)
    print("COMPARISON SUMMARY")
    print("=" * 80)

    cycles1 = result1['total_cycles']
    cycles2 = result2['total_cycles']
    diff = cycles2 - cycles1
    pct_change = (diff / cycles1 * 100) if cycles1 > 0 else 0

    print(f"Profile 1 (baseline): {cycles1:,} cycles")
    print(f"Profile 2 (current):  {cycles2:,} cycles")
    print(f"Difference: {diff:+,} cycles ({pct_change:+.2f}%)")
    print()

    # Find biggest changes
    all_funcs = set(result1['inclusive'].keys()) | set(result2['inclusive'].keys())

    changes = []
    for func in all_funcs:
        c1 = result1['inclusive'].get(func, 0)
        c2 = result2['inclusive'].get(func, 0)
        change = c2 - c1
        if abs(change) > 0:
            changes.append((func, c1, c2, change))

    # Sort by absolute change
    changes.sort(key=lambda x: -abs(x[3]))

    print(f"TOP {top_n} BIGGEST CHANGES (by absolute cycle difference)")
    print("-" * 100)
    print(f"{'BASELINE':>15} {'CURRENT':>15} {'DIFF':>15} {'%CHG':>8}  FUNCTION")
    print("-" * 100)

    for func, c1, c2, change in changes[:top_n]:
        pct_chg = (change / c1 * 100) if c1 > 0 else (100 if c2 > 0 else 0)
        display_name = func[:50] + "..." if len(func) > 53 else func
        print(f"{c1:>15,} {c2:>15,} {change:>+15,} {pct_chg:>+7.1f}%  {display_name}")


def main():
    parser = argparse.ArgumentParser(description='Parse SP1 Firefox profiler JSON files')
    parser.add_argument('profile', help='Path to profile JSON file')
    parser.add_argument('--compare', '-c', help='Compare with another profile')
    parser.add_argument('--top', '-n', type=int, default=50, help='Number of top functions to show')
    parser.add_argument('--output', '-o', help='Output file path')

    args = parser.parse_args()

    if args.compare:
        compare_profiles(args.profile, args.compare, args.top)
    else:
        parse_profile(args.profile, args.top, args.output)


if __name__ == '__main__':
    main()
