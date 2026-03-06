"""Minimal EVM assembler with label support and custom EIP-8141 opcodes."""

OPCODES = {
    'STOP': 0x00, 'ADD': 0x01, 'MUL': 0x02, 'SUB': 0x03, 'DIV': 0x04,
    'SDIV': 0x05, 'MOD': 0x06, 'SMOD': 0x07, 'ADDMOD': 0x08, 'MULMOD': 0x09,
    'EXP': 0x0A, 'SIGNEXTEND': 0x0B,
    'LT': 0x10, 'GT': 0x11, 'SLT': 0x12, 'SGT': 0x13, 'EQ': 0x14,
    'ISZERO': 0x15, 'AND': 0x16, 'OR': 0x17, 'XOR': 0x18, 'NOT': 0x19,
    'BYTE': 0x1A, 'SHL': 0x1B, 'SHR': 0x1C, 'SAR': 0x1D,
    'SHA3': 0x20, 'KECCAK256': 0x20,
    'ADDRESS': 0x30, 'BALANCE': 0x31, 'ORIGIN': 0x32, 'CALLER': 0x33,
    'CALLVALUE': 0x34, 'CALLDATALOAD': 0x35, 'CALLDATASIZE': 0x36,
    'CALLDATACOPY': 0x37, 'CODESIZE': 0x38, 'CODECOPY': 0x39,
    'GASPRICE': 0x3A, 'EXTCODESIZE': 0x3B, 'EXTCODECOPY': 0x3C,
    'RETURNDATASIZE': 0x3D, 'RETURNDATACOPY': 0x3E, 'EXTCODEHASH': 0x3F,
    'BLOCKHASH': 0x40, 'COINBASE': 0x41, 'TIMESTAMP': 0x42, 'NUMBER': 0x43,
    'PREVRANDAO': 0x44, 'GASLIMIT': 0x45, 'CHAINID': 0x46, 'SELFBALANCE': 0x47,
    'BASEFEE': 0x48,
    'POP': 0x50, 'MLOAD': 0x51, 'MSTORE': 0x52, 'MSTORE8': 0x53,
    'SLOAD': 0x54, 'SSTORE': 0x55, 'JUMP': 0x56, 'JUMPI': 0x57,
    'PC': 0x58, 'MSIZE': 0x59, 'GAS': 0x5A, 'JUMPDEST': 0x5B,
    'TLOAD': 0x5C, 'TSTORE': 0x5D,
    # DUP
    **{f'DUP{i}': 0x7F + i for i in range(1, 17)},
    # SWAP
    **{f'SWAP{i}': 0x8F + i for i in range(1, 17)},
    # LOG
    'LOG0': 0xA0, 'LOG1': 0xA1, 'LOG2': 0xA2, 'LOG3': 0xA3, 'LOG4': 0xA4,
    # System
    'CREATE': 0xF0, 'CALL': 0xF1, 'CALLCODE': 0xF2, 'RETURN': 0xF3,
    'DELEGATECALL': 0xF4, 'CREATE2': 0xF5, 'STATICCALL': 0xFA,
    'REVERT': 0xFD, 'INVALID': 0xFE, 'SELFDESTRUCT': 0xFF,
    # Custom EIP-8141 opcodes
    'APPROVE': 0xAA,
    'TXPARAMLOAD': 0xB0,
    'TXPARAMSIZE': 0xB1,
    'TXPARAMCOPY': 0xB2,
}


def assemble(source: str, base_offset: int = 0) -> bytes:
    """Assemble EVM mnemonics to bytecode.

    Supports:
    - Standard opcodes and custom EIP-8141 opcodes
    - Labels: label_name:  (on its own line)
    - Label references in PUSH: PUSH1 :label_name, PUSH2 :label_name
    - Hex values: PUSH1 0xff, PUSH4 0xdeadbeef
    - Comments: ; or //
    - base_offset: added to all label positions (for initcode embedding)
    """
    lines = source.strip().split('\n')

    # Parse into tokens
    tokens = []
    for line in lines:
        line = line.split(';')[0].split('//')[0].strip()
        if not line:
            continue
        if line.endswith(':') and not line.startswith('.'):
            tokens.append(('label_def', line[:-1].strip()))
            continue
        parts = line.split()
        if not parts:
            continue
        mnemonic = parts[0].upper()
        if mnemonic.startswith('PUSH'):
            n = int(mnemonic[4:])
            tokens.append(('push', 0x5F + n, n))
            if len(parts) > 1 and parts[1].startswith(':'):
                tokens.append(('label_ref', parts[1][1:], n))
            else:
                val = int(parts[1], 16) if len(parts) > 1 else 0
                tokens.append(('imm', val.to_bytes(n, 'big')))
        elif mnemonic in OPCODES:
            tokens.append(('opcode', OPCODES[mnemonic]))
        else:
            raise ValueError(f"Unknown mnemonic: {mnemonic}")

    # Pass 1: compute label positions
    labels = {}
    pos = 0
    for tok in tokens:
        if tok[0] == 'label_def':
            labels[tok[1]] = pos + base_offset
        elif tok[0] in ('opcode', 'push'):
            pos += 1
        elif tok[0] == 'imm':
            pos += len(tok[1])
        elif tok[0] == 'label_ref':
            pos += tok[2]

    # Pass 2: emit bytes
    result = bytearray()
    for tok in tokens:
        if tok[0] == 'label_def':
            continue
        elif tok[0] in ('opcode', 'push'):
            result.append(tok[1])
        elif tok[0] == 'imm':
            result.extend(tok[1])
        elif tok[0] == 'label_ref':
            name, n = tok[1], tok[2]
            if name not in labels:
                raise ValueError(f"Undefined label: {name}")
            result.extend(labels[name].to_bytes(n, 'big'))

    return bytes(result)


def wrap_initcode(runtime: bytes) -> bytes:
    """Create simple initcode that deploys runtime bytecode.

    Generates: PUSH runtime_len, DUP1, PUSH offset, PUSH1 0, CODECOPY, PUSH1 0, RETURN
    """
    rlen = len(runtime)
    if rlen <= 255:
        ctor_len = 11
        ctor = bytes([
            0x60, rlen,         # PUSH1 runtime_len
            0x80,               # DUP1
            0x60, ctor_len,     # PUSH1 offset
            0x60, 0x00,         # PUSH1 0
            0x39,               # CODECOPY
            0x60, 0x00,         # PUSH1 0
            0xF3,               # RETURN
        ])
    else:
        ctor_len = 13
        ctor = bytes([
            0x61, (rlen >> 8) & 0xFF, rlen & 0xFF,  # PUSH2 runtime_len
            0x80,               # DUP1
            0x60, ctor_len,     # PUSH1 offset
            0x60, 0x00,         # PUSH1 0
            0x39,               # CODECOPY
            0x60, 0x00,         # PUSH1 0
            0xF3,               # RETURN
        ])
    return ctor + runtime


def make_initcode_with_constructor(constructor_asm: str, runtime_asm: str) -> bytes:
    """Create initcode with custom constructor that runs before deploying runtime.

    The constructor ASM should end with CODECOPY + RETURN for the runtime.
    Use {RUNTIME_LEN} and {CONSTRUCTOR_LEN} placeholders.
    """
    # First assemble runtime to know its length
    runtime = assemble(runtime_asm)

    # Replace placeholders in constructor
    constructor_asm = constructor_asm.replace('{RUNTIME_LEN}', hex(len(runtime)))

    # Need to figure out constructor length (chicken-and-egg)
    # Assemble constructor once to get its length
    ctor_first = assemble(constructor_asm)
    ctor_len = len(ctor_first)
    constructor_asm_final = constructor_asm.replace('{CONSTRUCTOR_LEN}', hex(ctor_len))

    # Re-assemble with correct length (check it didn't change size)
    ctor = assemble(constructor_asm_final)
    assert len(ctor) == ctor_len, "Constructor length changed after placeholder substitution"

    return ctor + runtime


if __name__ == '__main__':
    # Quick test
    code = assemble("""
        PUSH1 0x01
        PUSH1 0x02
        ADD
        STOP
    """)
    assert code == bytes([0x60, 0x01, 0x60, 0x02, 0x01, 0x00])

    # Test labels
    code = assemble("""
        PUSH1 :end
        JUMP
    end:
        JUMPDEST
        STOP
    """)
    assert code == bytes([0x60, 0x03, 0x56, 0x5B, 0x00])

    print("Assembler tests passed!")
    print(f"Test bytecode: 0x{code.hex()}")
