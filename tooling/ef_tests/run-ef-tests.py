#!/usr/bin/env python3
"""EF Blockchain Test Runner & Analyzer for ethrex.

Runs EF blockchain tests, parses cargo test output, categorizes failures,
and produces LLM-friendly reports for efficient iteration on EVM fixes.

Usage:
    python run-ef-tests.py                     # Run all tests, show report
    python run-ef-tests.py --from-file out.log # Parse saved output
    python run-ef-tests.py --filter eip7702    # Run only matching tests
    python run-ef-tests.py --get-json <name>   # Find & print JSON for a test
    python run-ef-tests.py --summary-only      # Counts + category table only
    python run-ef-tests.py --json-output       # Machine-readable JSON output
    python run-ef-tests.py --save-output f.log # Save raw cargo output
    python run-ef-tests.py --count-by-eip      # Break down failures by EIP
    python run-ef-tests.py --list-categories   # Show failure category definitions
    python run-ef-tests.py --state --forks Amsterdam  # Run state tests for Amsterdam fork
    python run-ef-tests.py --state --summary-only     # State tests, summary only
"""

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum, auto
from pathlib import Path
from typing import Optional


# ---------------------------------------------------------------------------
# Failure categories - derived from test_runner.rs error messages
# ---------------------------------------------------------------------------

class FailureCategory(Enum):
    RLPDecodingError = auto()
    GenesisHeaderMismatch = auto()
    UnexpectedExecutionFailure = auto()
    ExpectedExceptionNotRaised = auto()
    GenesisStateRootMismatch = auto()
    AccountInfoNotFound = auto()
    AccountInfoMismatch = auto()
    AccountCodeNotFound = auto()
    AccountCodeMismatch = auto()
    StorageNotFound = auto()
    StorageMismatch = auto()
    LastBlockHashMismatch = auto()
    WitnessCreationFailed = auto()
    StatelessExecutionFailed = auto()
    StatelessUnexpectedSuccess = auto()
    Unknown = auto()


# Ordered list: more specific patterns first.
CATEGORY_PATTERNS: list[tuple[FailureCategory, re.Pattern]] = [
    (FailureCategory.RLPDecodingError,
     re.compile(r"Failed to decode genesis RLP")),
    (FailureCategory.GenesisHeaderMismatch,
     re.compile(r"Decoded genesis header does not match")),
    (FailureCategory.UnexpectedExecutionFailure,
     re.compile(r"Transaction execution unexpectedly failed")),
    (FailureCategory.ExpectedExceptionNotRaised,
     re.compile(r"Expected transaction execution to fail")),
    (FailureCategory.GenesisStateRootMismatch,
     re.compile(r"Mismatched genesis state root")),
    (FailureCategory.AccountInfoNotFound,
     re.compile(r"Account info for address .* not found")),
    (FailureCategory.AccountInfoMismatch,
     re.compile(r"Mismatched account info")),
    (FailureCategory.AccountCodeNotFound,
     re.compile(r"Account code for code hash .* not found")),
    (FailureCategory.AccountCodeMismatch,
     re.compile(r"Mismatched account code")),
    (FailureCategory.StorageNotFound,
     re.compile(r"Storage missing for address")),
    (FailureCategory.StorageMismatch,
     re.compile(r"Mismatched storage value")),
    (FailureCategory.LastBlockHashMismatch,
     re.compile(r"Last block number does not match")),
    (FailureCategory.WitnessCreationFailed,
     re.compile(r"Failed to create witness")),
    (FailureCategory.StatelessExecutionFailed,
     re.compile(r"to succeed but failed")),
    (FailureCategory.StatelessUnexpectedSuccess,
     re.compile(r"to fail but succeeded")),
]

CATEGORY_DESCRIPTIONS: dict[FailureCategory, str] = {
    FailureCategory.RLPDecodingError: "Failed to decode genesis block RLP (test_runner.rs L73)",
    FailureCategory.GenesisHeaderMismatch: "Decoded genesis header != expected header (test_runner.rs L77)",
    FailureCategory.UnexpectedExecutionFailure: "Block execution failed when it should have succeeded (test_runner.rs L128)",
    FailureCategory.ExpectedExceptionNotRaised: "Block execution succeeded when it should have failed (test_runner.rs L142)",
    FailureCategory.GenesisStateRootMismatch: "Genesis state root in DB != expected (test_runner.rs L322)",
    FailureCategory.AccountInfoNotFound: "Post-state account not found in DB (test_runner.rs L341)",
    FailureCategory.AccountInfoMismatch: "Post-state account info differs from DB (test_runner.rs L345)",
    FailureCategory.AccountCodeNotFound: "Account code not found for expected code hash (test_runner.rs L356)",
    FailureCategory.AccountCodeMismatch: "Account code in DB differs from expected (test_runner.rs L360)",
    FailureCategory.StorageNotFound: "Storage slot missing from DB (test_runner.rs L369)",
    FailureCategory.StorageMismatch: "Storage value in DB differs from expected (test_runner.rs L373)",
    FailureCategory.LastBlockHashMismatch: "Last block hash after execution != expected (test_runner.rs L383)",
    FailureCategory.WitnessCreationFailed: "Failed to create execution witness (test_runner.rs L412)",
    FailureCategory.StatelessExecutionFailed: "Stateless execution failed when expected to succeed (test_runner.rs L429)",
    FailureCategory.StatelessUnexpectedSuccess: "Stateless execution succeeded when expected to fail (test_runner.rs L433)",
    FailureCategory.Unknown: "Unrecognized error pattern",
}


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------

@dataclass
class TestFailure:
    file_path: str          # vectors/eest/... path from test name
    test_key: str           # full test key (from JSON)
    error_text: str         # raw error message
    category: str = "Unknown"  # the actual error kind (e.g. InvalidBlock(BlockAccessListHashMismatch))


@dataclass
class TestResults:
    passed: int = 0
    failed: int = 0
    ignored: int = 0
    duration_secs: float = 0.0
    failures: list[TestFailure] = field(default_factory=list)
    raw_output: str = ""
    timestamp: str = ""


# ---------------------------------------------------------------------------
# Categorization
# ---------------------------------------------------------------------------

def clean_error_text(text: str) -> str:
    """Strip debug-format artifacts like escaped quotes."""
    return text.replace('\\"', '"').strip('"').strip("\\").strip()


# Regex to extract the inner error from "with error <ACTUAL_ERROR>"
RE_INNER_ERROR = re.compile(r"with error\s+(.+?)(?:\s*$|\")")


def categorize_error(error_text: str) -> str:
    """Extract the actual error kind from the error message.

    For "unexpectedly failed ... with error X" -> returns X (the useful part).
    For panics like "Mismatched account info" -> returns that message directly.
    """
    # First try to extract inner error from execution failures
    m = RE_INNER_ERROR.search(error_text)
    if m:
        return m.group(1).strip()

    # Fall back to pattern matching for panics / other messages
    for category, pattern in CATEGORY_PATTERNS:
        if pattern.search(error_text):
            return category.name
    return "Unknown"


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

# Matches: test blockchain_runner::eest/path/file.json ... ok
# datatest_stable uses "::" separator (not brackets)
# The path may be followed by variable whitespace before "..."
RE_TEST_LINE = re.compile(
    r"^test\s+blockchain_runner::(.+?\.json)\s+\.\.\.\s+(ok|FAILED|ignored)"
)

# Matches the summary line: test result: FAILED. X passed; Y failed; Z ignored; ...
RE_SUMMARY = re.compile(
    r"test result:.*?(\d+)\s+passed;\s+(\d+)\s+failed;\s+(\d+)\s+ignored"
)

# Matches duration: finished in Xs
RE_DURATION = re.compile(r"finished in\s+([\d.]+)s")

# Matches failure block header:
#   ---- blockchain_runner::eest/path/file.json ----
RE_FAILURE_HEADER = re.compile(
    r"^----\s+blockchain_runner::(.+?\.json)\s+----$"
)

# Extract test key from error text patterns like "test:key_name" or "test_key: error"
RE_TEST_KEY_IN_ERROR = re.compile(r"test:(\S+)")

# Separator used when test_runner.rs joins multiple errors
ERROR_SEPARATOR = "     -------     "

# Regex to strip ANSI escape sequences
RE_ANSI = re.compile(r"\x1b\[[0-9;]*m")

# State test patterns
RE_STATE_SUMMARY = re.compile(r"Summary:\s*(\d+)/(\d+)")
RE_STATE_DIR_RESULT = re.compile(r"(state_tests/\S+|GeneralStateTests/\S+|LegacyTests/\S+):\s*(\d+)/(\d+)\s*\(([\d.]+)%\)")
RE_STATE_DURATION = re.compile(r"real\s+(\d+)m([\d.]+)s")


def parse_output(raw: str) -> TestResults:
    """Parse cargo test output into structured results."""
    results = TestResults(
        raw_output=raw,
        timestamp=datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
    )

    lines = raw.split("\n")

    # --- Pass 1: Count results from test lines ---
    failed_paths: set[str] = set()
    for line in lines:
        m = RE_TEST_LINE.match(line.strip())
        if m:
            path, status = m.group(1), m.group(2)
            if status == "FAILED":
                failed_paths.add(path)

    # --- Pass 2: Extract summary line ---
    # Keep the result line with the highest total (avoids doc-test "0 passed; 0 failed")
    best_total = -1
    for line in lines:
        m = RE_SUMMARY.search(line)
        if m:
            p, f, ig = int(m.group(1)), int(m.group(2)), int(m.group(3))
            if p + f + ig > best_total:
                best_total = p + f + ig
                results.passed = p
                results.failed = f
                results.ignored = ig
        m = RE_DURATION.search(line)
        if m:
            results.duration_secs = float(m.group(1))

    # --- Pass 3: Extract failure blocks ---
    # Failure details appear in stderr between the header markers.
    i = 0
    while i < len(lines):
        header_match = RE_FAILURE_HEADER.match(lines[i].strip())
        if header_match:
            file_path = header_match.group(1)
            i += 1
            # Collect all lines until the next header or "failures:" section
            block_lines: list[str] = []
            while i < len(lines):
                stripped = lines[i].strip()
                if RE_FAILURE_HEADER.match(stripped):
                    break
                if stripped == "failures:":
                    break
                if stripped.startswith("note:"):
                    i += 1
                    continue
                block_lines.append(lines[i])
                i += 1

            error_block = "\n".join(block_lines).strip()
            if not error_block:
                continue

            # Handle two failure modes:
            # 1. Returned errors joined by separator
            # 2. Panics with "panicked at" messages

            if ERROR_SEPARATOR in error_block:
                # Mode 1: Multiple errors joined by separator
                parts = error_block.split(ERROR_SEPARATOR)
                for part in parts:
                    part = part.strip().strip('"')
                    if not part:
                        continue
                    # Format: "test_key: error_message" or just error text
                    test_key = ""
                    error_text = part
                    # Try to extract test key from "key: error" format
                    colon_idx = part.find(": ")
                    if colon_idx > 0 and not part[:colon_idx].startswith("Failed"):
                        test_key = part[:colon_idx].strip().strip('"')
                        error_text = part[colon_idx + 2:].strip().strip('"')

                    error_text = clean_error_text(error_text)
                    category = categorize_error(error_text)
                    results.failures.append(TestFailure(
                        file_path=file_path,
                        test_key=test_key or file_path,
                        error_text=error_text,
                        category=category,
                    ))
            else:
                # Mode 2: Panic or single error
                clean = error_block.strip().strip('"')
                test_key = ""
                error_text = clean

                # Try "key: error" format first
                colon_idx = clean.find(": ")
                if colon_idx > 0 and not clean[:colon_idx].startswith("Failed"):
                    test_key = clean[:colon_idx].strip().strip('"')
                    error_text = clean[colon_idx + 2:].strip().strip('"')

                # Fallback: extract test key from "test:key_name" in error
                if not test_key:
                    key_match = RE_TEST_KEY_IN_ERROR.search(error_text)
                    if key_match:
                        test_key = key_match.group(1)

                error_text = clean_error_text(error_text)
                category = categorize_error(error_text)
                results.failures.append(TestFailure(
                    file_path=file_path,
                    test_key=test_key or file_path,
                    error_text=error_text,
                    category=category,
                ))
        else:
            i += 1

    return results


def parse_state_output(raw: str) -> TestResults:
    """Parse state test runner output into structured results."""
    results = TestResults(
        raw_output=raw,
        timestamp=datetime.now().strftime("%Y-%m-%d %H:%M:%S"),
    )

    # Strip ANSI codes for reliable matching
    clean = RE_ANSI.sub("", raw)
    lines = clean.split("\n")

    # Extract summary line (first match of "Summary: N/M")
    for line in lines:
        m = RE_STATE_SUMMARY.search(line)
        if m:
            results.passed = int(m.group(1))
            total = int(m.group(2))
            results.failed = total - results.passed
            break

    # Extract duration from `time` output: real XmY.Zs
    for line in lines:
        m = RE_STATE_DURATION.search(line)
        if m:
            minutes = int(m.group(1))
            seconds = float(m.group(2))
            results.duration_secs = minutes * 60 + seconds

    # Parse "Failed tests:" section for individual failure blocks
    in_failed_section = False
    i = 0
    while i < len(lines):
        stripped = lines[i].strip()

        if stripped == "Failed tests:":
            in_failed_section = True
            i += 1
            continue

        if not in_failed_section:
            i += 1
            continue

        # Each failure block starts with "Test:"
        if stripped == "Test:":
            # Collect block: Test name, Test path, description, then fork/vector/error lines
            test_name = ""
            test_path = ""
            test_description = ""
            i += 1

            # Read test metadata lines
            while i < len(lines):
                stripped = lines[i].strip()
                if stripped.startswith("Test name:"):
                    test_name = stripped[len("Test name:"):].strip()
                elif stripped.startswith("Test path:"):
                    test_path = stripped[len("Test path:"):].strip()
                elif stripped.startswith("Test description:"):
                    test_description = stripped[len("Test description:"):].strip()
                elif stripped.startswith("Fork:") or stripped == "Test:" or stripped == "":
                    if stripped.startswith("Fork:") or stripped == "Test:":
                        break
                    # Skip blank lines within the metadata
                elif stripped.startswith("Note:") or stripped.startswith("- http") or stripped.startswith("- Test"):
                    pass  # Skip note/link lines
                i += 1

            # Now parse Fork/Failed Vector/Error blocks
            while i < len(lines):
                stripped = lines[i].strip()
                if stripped == "Test:" or stripped == "":
                    # Check if this blank line is followed by another Test: block
                    if stripped == "":
                        # Look ahead for Test: or end
                        j = i + 1
                        while j < len(lines) and lines[j].strip() == "":
                            j += 1
                        if j >= len(lines) or lines[j].strip() == "Test:":
                            break
                        # Otherwise it might be within the block, keep going
                        i += 1
                        continue
                    break

                if stripped.startswith("Fork:"):
                    current_fork = stripped[len("Fork:"):].strip()
                    i += 1
                    # Read Failed Vector / Error pairs
                    while i < len(lines):
                        stripped = lines[i].strip()
                        if stripped.startswith("Fork:") or stripped == "Test:" or (stripped == "" and _is_block_end(lines, i)):
                            break
                        if stripped.startswith("Failed Vector:"):
                            vector_text = stripped
                            error_text = ""
                            category = ""
                            i += 1
                            # Read Error and other detail lines
                            while i < len(lines):
                                stripped = lines[i].strip()
                                if stripped.startswith("Error:"):
                                    error_text = stripped[len("Error:"):].strip()
                                    category = _categorize_state_error(error_text)
                                elif (stripped.startswith("Failed Vector:") or
                                      stripped.startswith("Fork:") or
                                      stripped == "Test:" or
                                      (stripped == "" and _is_block_end(lines, i))):
                                    break
                                # Other detail lines (execution result mismatch, gas mismatch, etc.) - skip
                                i += 1

                            results.failures.append(TestFailure(
                                file_path=test_path,
                                test_key=test_name,
                                error_text=error_text or vector_text,
                                category=category or "Unknown",
                            ))
                            continue
                        i += 1
                    continue
                i += 1
            continue
        i += 1

    return results


def _is_block_end(lines: list[str], i: int) -> bool:
    """Check if a blank line signals the end of a failure block."""
    j = i + 1
    while j < len(lines) and lines[j].strip() == "":
        j += 1
    if j >= len(lines):
        return True
    next_line = lines[j].strip()
    return next_line == "Test:" or next_line == ""


def _categorize_state_error(error_text: str) -> str:
    """Extract a short category from state test error text."""
    # Common patterns in state test errors
    if "Post-state root mismatch" in error_text or "post-state" in error_text.lower():
        return "Post-state root mismatch"
    if "Logs mismatch" in error_text or "logs mismatch" in error_text.lower():
        return "Logs mismatch"
    if "Gas used mismatch" in error_text or "gas used" in error_text.lower():
        return "Gas used mismatch"
    if "Gas refunded mismatch" in error_text or "gas refunded" in error_text.lower():
        return "Gas refunded mismatch"
    if "Execution result mismatch" in error_text:
        return "Execution result mismatch"
    if "execution failed when it was not expected" in error_text.lower():
        return "Unexpected execution failure"
    if "Exception does not match" in error_text:
        return "Exception mismatch"
    if "Failed to ensure pre-state" in error_text:
        return "Pre-state validation failure"
    if "Failed to ensure post-state" in error_text:
        return "Post-state validation failure"
    if "VM initialization failed" in error_text:
        return "VM initialization failure"
    # For more specific errors, use the first significant phrase
    if error_text:
        # Truncate long errors to a reasonable category name
        short = error_text[:80]
        return short
    return "Unknown"


# ---------------------------------------------------------------------------
# Test execution
# ---------------------------------------------------------------------------

SCRIPT_DIR = Path(__file__).resolve().parent


def run_tests(filter_pattern: Optional[str] = None) -> str:
    """Run EF blockchain tests and return combined stdout+stderr."""
    if filter_pattern:
        cmd = [
            "cargo", "test",
            "--profile", "release-with-debug",
            "--", filter_pattern,
        ]
    else:
        cmd = ["make", "test-levm"]

    blockchain_dir = SCRIPT_DIR / "blockchain"
    print(f"Running: {' '.join(cmd)}", file=sys.stderr)
    print(f"Working directory: {blockchain_dir}", file=sys.stderr)

    proc = subprocess.run(
        cmd,
        cwd=str(blockchain_dir),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    return proc.stdout


def run_state_tests(forks: Optional[str] = None) -> str:
    """Run EF state tests and return combined stdout+stderr."""
    flags = "--summary"
    if forks:
        flags = f"--forks {forks} --summary"
    cmd = ["make", "run-evm-ef-tests", f"flags={flags}"]
    state_dir = SCRIPT_DIR / "state"
    print(f"Running: {' '.join(cmd)}", file=sys.stderr)
    print(f"Working directory: {state_dir}", file=sys.stderr)
    proc = subprocess.run(
        cmd,
        cwd=str(state_dir),
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    return proc.stdout


# ---------------------------------------------------------------------------
# JSON test lookup
# ---------------------------------------------------------------------------

VECTORS_DIR = SCRIPT_DIR / "blockchain" / "vectors"


def get_json(search_term: str) -> None:
    """Find and print the JSON test fixture matching the search term."""
    if not VECTORS_DIR.exists():
        print(f"Error: vectors directory not found at {VECTORS_DIR}", file=sys.stderr)
        sys.exit(1)

    # Strategy 1: Search by file path
    matches: list[tuple[Path, str]] = []  # (file_path, key)

    for json_file in VECTORS_DIR.rglob("*.json"):
        rel = str(json_file.relative_to(SCRIPT_DIR))
        if search_term.lower() in rel.lower():
            try:
                with open(json_file) as f:
                    data = json.load(f)
                for key in data:
                    matches.append((json_file, key))
            except (json.JSONDecodeError, OSError):
                continue

    # Strategy 2: Search by test key inside JSON files (if no path matches)
    if not matches:
        for json_file in VECTORS_DIR.rglob("*.json"):
            try:
                with open(json_file) as f:
                    data = json.load(f)
                for key in data:
                    if search_term.lower() in key.lower():
                        matches.append((json_file, key))
            except (json.JSONDecodeError, OSError):
                continue

    if not matches:
        print(f"No test found matching: {search_term}")
        return

    if len(matches) > 20:
        print(f"Found {len(matches)} matches. Showing first 20:\n")
        for fpath, key in matches[:20]:
            print(f"  {fpath.relative_to(SCRIPT_DIR)}")
            print(f"    Key: {key}\n")
        print(f"... and {len(matches) - 20} more. Refine your search term.")
        return

    if len(matches) > 1:
        print(f"Found {len(matches)} matches:\n")
        for idx, (fpath, key) in enumerate(matches, 1):
            print(f"  [{idx}] {fpath.relative_to(SCRIPT_DIR)}")
            print(f"      Key: {key}\n")

        # If few enough, print all; otherwise just list
        if len(matches) > 5:
            print("Refine your search term or use a more specific path/key.")
            return

    # Print matching entries
    for fpath, key in matches:
        with open(fpath) as f:
            data = json.load(f)
        entry = data[key]

        print(f"--- File: {fpath.relative_to(SCRIPT_DIR)}")
        print(f"--- Key:  {key}")
        if "network" in entry:
            print(f"--- Network: {entry['network']}")
        if "blocks" in entry:
            print(f"--- Blocks: {len(entry['blocks'])}")
        if "_info" in entry and "description" in entry["_info"]:
            print(f"--- Description: {entry['_info']['description']}")
        print()

        # Print the entry without _info (it's metadata noise)
        filtered = {k: v for k, v in entry.items() if k != "_info"}
        print(json.dumps(filtered, indent=2))
        print()


# ---------------------------------------------------------------------------
# Output formatters
# ---------------------------------------------------------------------------

def group_by_category(failures: list[TestFailure]) -> dict[str, list[TestFailure]]:
    groups: dict[str, list[TestFailure]] = {}
    for f in failures:
        groups.setdefault(f.category, []).append(f)
    return groups


def extract_eip(text: str) -> Optional[str]:
    """Extract EIP number from a test path or key."""
    m = re.search(r"(eip\d+)", text, re.IGNORECASE)
    return m.group(1).lower() if m else None


def group_by_eip(failures: list[TestFailure]) -> dict[str, list[TestFailure]]:
    groups: dict[str, list[TestFailure]] = {}
    for f in failures:
        eip = extract_eip(f.file_path) or extract_eip(f.test_key) or "unknown"
        groups.setdefault(eip, []).append(f)
    return groups


def print_report(results: TestResults, summary_only: bool = False,
                 count_by_eip: bool = False) -> None:
    """Print a markdown-formatted report."""
    total = results.passed + results.failed + results.ignored

    print(f"# EF Blockchain Test Results")
    print(f"Run: {results.timestamp} | Duration: {results.duration_secs:.1f}s")
    print()
    print(f"## Summary")
    print(f"Passed: {results.passed} | Failed: {results.failed} | "
          f"Ignored: {results.ignored} | Total: {total}")
    print()

    if results.failed == 0:
        print("All tests passed!")
        return

    # Category breakdown
    by_cat = group_by_category(results.failures)
    print(f"## Failures by Category")
    sorted_cats = sorted(by_cat.items(), key=lambda x: len(x[1]), reverse=True)
    for cat, items in sorted_cats:
        pct = len(items) / len(results.failures) * 100 if results.failures else 0
        print(f"  {cat:<50s} {len(items):>4d}  ({pct:5.1f}%)")
    print()

    # EIP breakdown
    if count_by_eip:
        by_eip = group_by_eip(results.failures)
        print(f"## Failures by EIP")
        sorted_eips = sorted(by_eip.items(), key=lambda x: len(x[1]), reverse=True)
        for eip, items in sorted_eips:
            print(f"  {eip:<20s} {len(items):>4d}")
        print()

    if summary_only:
        return

    # Detailed failures grouped by category
    print(f"## Detailed Failures")
    for cat, items in sorted_cats:
        print(f"\n### {cat} ({len(items)})")
        for idx, f in enumerate(items, 1):
            print(f"{idx:>3d}. File:  {f.file_path}")
            if f.test_key != f.file_path:
                print(f"     Key:   {f.test_key}")
            # Truncate very long error messages
            err = f.error_text
            if len(err) > 500:
                err = err[:500] + "... [truncated]"
            print(f"     Error: {err}")
    print()


def print_json_output(results: TestResults) -> None:
    """Print machine-readable JSON output."""
    by_cat = group_by_category(results.failures)
    output = {
        "timestamp": results.timestamp,
        "duration_secs": results.duration_secs,
        "summary": {
            "passed": results.passed,
            "failed": results.failed,
            "ignored": results.ignored,
            "total": results.passed + results.failed + results.ignored,
        },
        "categories": {
            cat: {
                "count": len(items),
                "failures": [
                    {
                        "file_path": f.file_path,
                        "test_key": f.test_key,
                        "error_text": f.error_text[:1000],
                    }
                    for f in items
                ],
            }
            for cat, items in by_cat.items()
        },
    }
    print(json.dumps(output, indent=2))


def print_categories() -> None:
    """Print all failure category definitions."""
    print("# EF Test Failure Categories\n")
    print(f"{'Category':<35s} {'Pattern':<45s} Description")
    print("-" * 120)
    for cat, pattern in CATEGORY_PATTERNS:
        desc = CATEGORY_DESCRIPTIONS.get(cat, "")
        print(f"{cat.name:<35s} {pattern.pattern:<45s} {desc}")
    print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="EF Blockchain Test Runner & Analyzer for ethrex",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "--from-file", metavar="FILE",
        help="Parse saved cargo test output instead of running tests",
    )
    parser.add_argument(
        "--filter", metavar="PATTERN",
        help="Run only tests matching this pattern (passed to cargo test)",
    )
    parser.add_argument(
        "--get-json", metavar="NAME",
        help="Find & print JSON fixture for a test name",
    )
    parser.add_argument(
        "--summary-only", action="store_true",
        help="Show only counts and category table",
    )
    parser.add_argument(
        "--json-output", action="store_true",
        help="Output results as machine-readable JSON",
    )
    parser.add_argument(
        "--save-output", metavar="FILE",
        help="Save raw cargo test output to file",
    )
    parser.add_argument(
        "--count-by-eip", action="store_true",
        help="Include failure breakdown by EIP number",
    )
    parser.add_argument(
        "--list-categories", action="store_true",
        help="Show all failure category definitions and exit",
    )
    parser.add_argument(
        "--state", action="store_true",
        help="Run state tests instead of blockchain tests",
    )
    parser.add_argument(
        "--forks", metavar="FORKS",
        help="Comma-separated fork list for state tests (e.g., Amsterdam)",
    )

    args = parser.parse_args()

    # --list-categories: just print and exit
    if args.list_categories:
        print_categories()
        return

    # --get-json: lookup mode
    if args.get_json:
        get_json(args.get_json)
        return

    # Get test output
    if args.from_file:
        with open(args.from_file) as f:
            raw = f.read()
    else:
        if args.state:
            raw = run_state_tests(args.forks)
        else:
            raw = run_tests(args.filter)

    # Save output if requested
    if args.save_output:
        with open(args.save_output, "w") as f:
            f.write(raw)
        print(f"Saved raw output to {args.save_output}", file=sys.stderr)

    # Parse and report
    if args.state:
        results = parse_state_output(raw)
    else:
        results = parse_output(raw)

    if args.json_output:
        print_json_output(results)
    else:
        print_report(results, summary_only=args.summary_only,
                     count_by_eip=args.count_by_eip)


if __name__ == "__main__":
    main()
