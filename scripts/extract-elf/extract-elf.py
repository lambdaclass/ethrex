#!/usr/bin/env python3
"""
Extract raw ELF from ere Program format.

ere Program format uses bincode serialization with the ELF as the first field:
- 8 bytes: u64 little-endian ELF length
- N bytes: raw ELF data
- Remaining: optional metadata

Usage:
    python3 extract-elf.py <input_ere_program> <output_elf>

Example:
    python3 extract-elf.py ethrex-v9_0_0-sp1-v5_0_8 ethrex-v9_0_0-sp1-v5_0_8.elf
"""

import struct
import sys
from pathlib import Path


def extract_elf(input_path: Path, output_path: Path) -> int:
    """Extract raw ELF from ere Program format."""
    with open(input_path, "rb") as f:
        data = f.read()

    if len(data) < 8:
        print(f"Error: Input file too small ({len(data)} bytes)", file=sys.stderr)
        return 1

    # Read ELF length (first 8 bytes, little-endian u64)
    elf_len = struct.unpack("<Q", data[:8])[0]

    if elf_len > len(data) - 8:
        print(
            f"Error: ELF length ({elf_len}) exceeds available data ({len(data) - 8})",
            file=sys.stderr,
        )
        return 1

    # Extract ELF bytes
    elf = data[8 : 8 + elf_len]

    # Verify ELF magic
    if elf[:4] != b"\x7fELF":
        print("Warning: Extracted data does not start with ELF magic", file=sys.stderr)

    with open(output_path, "wb") as f:
        f.write(elf)

    print(f"Extracted ELF: {elf_len} bytes -> {output_path}")
    return 0


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(1)

    input_path = Path(sys.argv[1])
    output_path = Path(sys.argv[2])

    if not input_path.exists():
        print(f"Error: Input file not found: {input_path}", file=sys.stderr)
        sys.exit(1)

    sys.exit(extract_elf(input_path, output_path))


if __name__ == "__main__":
    main()
