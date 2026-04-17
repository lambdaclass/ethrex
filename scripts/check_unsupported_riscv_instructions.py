#!/usr/bin/env python3
"""Scan a RISC-V ELF binary for instructions unsupported by Airbender's ISA.

Airbender supports RV32IM but rejects:
  - MULH   (signed × signed → high 32 bits)
  - MULHSU (signed × unsigned → high 32 bits)

Usage: python3 check_unsupported_riscv_instructions.py <elf-path> [--nm-path <nm>]
"""
import struct
import subprocess
import sys

UNSUPPORTED = {
    (0x33, 1, 1): "MULH",
    (0x33, 1, 2): "MULHSU",
}

def load_symbols(elf_path, nm_path="nm"):
    try:
        result = subprocess.run(
            [nm_path, "--defined-only", elf_path],
            capture_output=True, text=True
        )
        symbols = []
        for line in result.stdout.strip().split("\n"):
            parts = line.split()
            if len(parts) >= 3 and parts[1] not in ("t",) or (len(parts) >= 3 and parts[2] not in (".L0",)):
                try:
                    addr = int(parts[0], 16)
                    name = parts[2] if len(parts) >= 3 else "?"
                    if not name.startswith(".L"):
                        symbols.append((addr, name))
                except ValueError:
                    pass
        symbols.sort()
        return symbols
    except FileNotFoundError:
        return []

def find_symbol(symbols, addr):
    result = "???"
    for sa, sn in symbols:
        if sa <= addr:
            result = sn
        else:
            break
    return result

def demangle(name):
    """Rough demangling: extract crate::module::function from Rust symbol."""
    try:
        result = subprocess.run(
            ["rustfilt", name], capture_output=True, text=True, timeout=1
        )
        if result.returncode == 0:
            return result.stdout.strip()
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass
    return name

def scan_elf(elf_path):
    with open(elf_path, "rb") as f:
        # Parse ELF32 header
        magic = f.read(4)
        if magic != b"\x7fELF":
            print(f"Error: {elf_path} is not an ELF file", file=sys.stderr)
            return 1

        f.seek(28)
        phoff = struct.unpack("<I", f.read(4))[0]
        f.seek(44)
        phnum = struct.unpack("<H", f.read(2))[0]

        # Collect PT_LOAD segments
        segments = []
        f.seek(phoff)
        for _ in range(phnum):
            ph = f.read(32)
            p_type, p_offset, p_vaddr, _, p_filesz = struct.unpack("<IIIII", ph[:20])
            if p_type == 1:  # PT_LOAD
                segments.append((p_vaddr, p_offset, p_filesz))

        # Scan for unsupported instructions
        found = []
        for vaddr, offset, size in segments:
            f.seek(offset)
            data = f.read(size)
            for i in range(0, len(data) - 3, 4):
                instr = struct.unpack_from("<I", data, i)[0]
                opcode = instr & 0x7F
                funct7 = (instr >> 25) & 0x7F
                funct3 = (instr >> 12) & 0x7
                key = (opcode, funct7, funct3)
                if key in UNSUPPORTED:
                    found.append((vaddr + i, UNSUPPORTED[key]))

        return found

def main():
    if len(sys.argv) < 2:
        print(f"Usage: {sys.argv[0]} <elf-path>", file=sys.stderr)
        return 1

    elf_path = sys.argv[1]
    found = scan_elf(elf_path)

    if not found:
        print("OK: no unsupported instructions found")
        return 0

    symbols = load_symbols(elf_path)

    print(f"FAIL: {len(found)} unsupported instruction(s) found:\n")
    by_func = {}
    for addr, name in found:
        func = find_symbol(symbols, addr)
        by_func.setdefault(func, []).append((addr, name))

    for func, instrs in by_func.items():
        demangled = demangle(func)
        print(f"  {demangled}")
        for addr, name in instrs:
            print(f"    0x{addr:08x}  {name}")
        print()

    return 1

if __name__ == "__main__":
    sys.exit(main())
