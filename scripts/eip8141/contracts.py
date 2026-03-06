"""Contract bytecodes for EIP-8141 Frame Transaction demo.

All contracts are hand-assembled using custom EVM opcodes (APPROVE, TXPARAMLOAD)
that are not supported by standard compilers.
"""

from evm_asm import assemble, wrap_initcode

# ============================================================
# SimpleP256Account
# ============================================================
# Smart contract account using P256 (secp256r1) signatures.
# Storage: slot 0 = pubkey_x, slot 1 = pubkey_y
#
# Selectors:
#   0x00000000 (or empty calldata): verify()       — APPROVE scope=0 (sender only)
#   0x00000001: execute(dest, value, data)
#   0x00000002: transfer(dest, amount)
#   0x00000003: verifyAndPay()                      — APPROVE scope=2 (combined sender+payer)

SIMPLE_P256_ACCOUNT_RUNTIME = assemble("""
    ; === Entry / Dispatcher ===
    ; If calldata is empty, accept ETH (receive fallback) and STOP
    CALLDATASIZE
    ISZERO
    PUSH1 :receive
    JUMPI

    PUSH1 0x00
    CALLDATALOAD
    PUSH1 0xE0
    SHR                         ; 4-byte selector

    ; selector 0x00000000 → verify
    DUP1
    ISZERO
    PUSH1 :verify
    JUMPI

    DUP1
    PUSH4 0x00000001
    EQ
    PUSH1 :execute
    JUMPI

    DUP1
    PUSH4 0x00000002
    EQ
    PUSH1 :transfer
    JUMPI

    DUP1
    PUSH4 0x00000003
    EQ
    PUSH1 :verify_and_pay
    JUMPI

    POP
    ; Unknown selector → revert
    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === receive() — accept ETH transfers ===
receive:
    JUMPDEST
    STOP

    ; === p256_check — shared P256 signature verification ===
    ; Expects: calldata contains selector(4) + r(32) + s(32)
    ; After this block, execution falls through to the APPROVE call
    ; On failure, jumps to revert_label
p256_check:
    JUMPDEST
    ; 1. Get sig_hash via TXPARAMLOAD(0x08, 0)
    PUSH1 0x00
    PUSH1 0x08
    TXPARAMLOAD
    PUSH1 0x00
    MSTORE                      ; mem[0..32] = sig_hash

    ; 2. Load r from calldata[4..36]
    PUSH1 0x04
    CALLDATALOAD
    PUSH1 0x20
    MSTORE                      ; mem[32..64] = r

    ; 3. Load s from calldata[36..68]
    PUSH1 0x24
    CALLDATALOAD
    PUSH1 0x40
    MSTORE                      ; mem[64..96] = s

    ; 4. Load pubkey_x from slot 0
    PUSH1 0x00
    SLOAD
    PUSH1 0x60
    MSTORE                      ; mem[96..128] = x

    ; 5. Load pubkey_y from slot 1
    PUSH1 0x01
    SLOAD
    PUSH1 0x80
    MSTORE                      ; mem[128..160] = y

    ; 6. STATICCALL P256VERIFY precompile at 0x0100
    PUSH1 0x20                  ; retSize = 32
    PUSH2 0x00A0                ; retOffset = 160
    PUSH1 0xA0                  ; argsSize = 160
    PUSH1 0x00                  ; argsOffset = 0
    PUSH2 0x0100                ; address = P256VERIFY precompile
    GAS                         ; gas (all remaining)
    STATICCALL

    ; 7. Check call succeeded
    ISZERO
    PUSH1 :revert_label
    JUMPI

    ; 8. Check result == 1
    PUSH2 0x00A0
    MLOAD
    PUSH1 0x01
    EQ
    ISZERO
    PUSH1 :revert_label
    JUMPI

    ; Return to caller (JUMP back)
    JUMP

    ; === verify() — sender-only approval (scope=0) ===
verify:
    JUMPDEST
    PUSH1 :verify_approve
    PUSH1 :p256_check
    JUMP
verify_approve:
    JUMPDEST
    PUSH1 0x00                  ; scope = 0 (sender only)
    PUSH1 0x00
    PUSH1 0x00
    APPROVE                     ; approves as sender and halts frame

    ; === verifyAndPay() — combined sender+payer (scope=2) ===
verify_and_pay:
    JUMPDEST
    POP                         ; pop selector from dispatcher
    PUSH1 :verify_and_pay_approve
    PUSH1 :p256_check
    JUMP
verify_and_pay_approve:
    JUMPDEST
    PUSH1 0x02                  ; scope = 2 (combined sender+payer)
    PUSH1 0x00
    PUSH1 0x00
    APPROVE                     ; approves as sender+payer and halts frame

    ; === revert ===
revert_label:
    JUMPDEST
    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === execute(dest, value, data) ===
    ; calldata: selector(4) || dest(32) || value(32) || data(variable)
execute:
    JUMPDEST
    POP                         ; pop selector from dispatcher

    ; Copy calldata[68..] to memory[0..] for call data
    CALLDATASIZE
    PUSH1 0x44                  ; 68
    SWAP1                       ; [68, calldatasize]
    SUB                         ; calldatasize - 68 = data_len
    PUSH1 0x44                  ; srcOffset = 68
    PUSH1 0x00                  ; destOffset = 0
    CALLDATACOPY

    ; CALL(gas, dest, value, argsOffset, argsSize, retOffset, retSize)
    PUSH1 0x00                  ; retSize
    PUSH1 0x00                  ; retOffset
    CALLDATASIZE
    PUSH1 0x44
    SWAP1                       ; [68, calldatasize]
    SUB                         ; calldatasize - 68 = argsSize
    PUSH1 0x00                  ; argsOffset
    PUSH1 0x24
    CALLDATALOAD                ; value
    PUSH1 0x04
    CALLDATALOAD                ; dest (right-aligned address in 32 bytes)
    GAS
    CALL

    ISZERO
    PUSH1 :revert_label
    JUMPI
    STOP

    ; === transfer(dest, amount) ===
    ; calldata: selector(4) || dest(32) || amount(32)
transfer:
    JUMPDEST
    POP                         ; pop selector
    PUSH1 0x00                  ; retSize
    PUSH1 0x00                  ; retOffset
    PUSH1 0x00                  ; argsSize (no data)
    PUSH1 0x00                  ; argsOffset
    PUSH1 0x24
    CALLDATALOAD                ; amount (value)
    PUSH1 0x04
    CALLDATALOAD                ; dest
    GAS
    CALL
    ISZERO
    PUSH1 :revert_label
    JUMPI
    STOP
""")

# Constructor: store pubkey_x and pubkey_y from code tail, then deploy runtime
# The constructor args (pubkey_x, pubkey_y) are appended AFTER the initcode
# in the deployment transaction data. During CREATE, CALLDATALOAD returns 0,
# so we must use CODECOPY to read args from the end of the code.
_p256_ctor_asm = """
    ; Copy 64 bytes (pubkey_x + pubkey_y) from end of code to memory
    PUSH1 0x40                  ; size = 64
    CODESIZE                    ; [64, codesize]
    PUSH1 0x40                  ; [64, codesize, 64]
    SWAP1                       ; [64, 64, codesize]
    SUB                         ; [64, codesize-64]  (SUB = top - second)
    PUSH1 0x00                  ; [64, codesize-64, 0] = destOffset
    CODECOPY                    ; mem[0..64] = pubkey_x(32) + pubkey_y(32)

    ; Store pubkey_x (mem[0..32]) to slot 0
    PUSH1 0x00
    MLOAD
    PUSH1 0x00
    SSTORE                      ; slot 0 = pubkey_x

    ; Store pubkey_y (mem[32..64]) to slot 1
    PUSH1 0x20
    MLOAD
    PUSH1 0x01
    SSTORE                      ; slot 1 = pubkey_y
"""

_p256_runtime_len = len(SIMPLE_P256_ACCOUNT_RUNTIME)
# Constructor continues: copy runtime to memory and return
_p256_ctor_prefix = assemble(_p256_ctor_asm)
# ctor_prefix + PUSH runtime_len + DUP1 + PUSH ctor_len + PUSH1 0 + CODECOPY + PUSH1 0 + RETURN
if _p256_runtime_len <= 255:
    _ctor_suffix_len = 2 + 1 + 2 + 2 + 1 + 2 + 1  # = 11
    _full_ctor_len = len(_p256_ctor_prefix) + _ctor_suffix_len
    _p256_ctor_suffix = bytes([
        0x60, _p256_runtime_len,    # PUSH1 runtime_len
        0x80,                       # DUP1
        0x60, _full_ctor_len,       # PUSH1 ctor_len (offset to runtime)
        0x60, 0x00,                 # PUSH1 0
        0x39,                       # CODECOPY
        0x60, 0x00,                 # PUSH1 0
        0xF3,                       # RETURN
    ])
else:
    _ctor_suffix_len = 3 + 1 + 2 + 2 + 1 + 2 + 1  # = 12
    _full_ctor_len = len(_p256_ctor_prefix) + _ctor_suffix_len
    _p256_ctor_suffix = bytes([
        0x61, (_p256_runtime_len >> 8) & 0xFF, _p256_runtime_len & 0xFF,
        0x80,
        0x60, _full_ctor_len,
        0x60, 0x00,
        0x39,
        0x60, 0x00,
        0xF3,
    ])

SIMPLE_P256_ACCOUNT_INITCODE = _p256_ctor_prefix + _p256_ctor_suffix + SIMPLE_P256_ACCOUNT_RUNTIME


# ============================================================
# AccountDeployer
# ============================================================
# Simple CREATE2 deployer.
# Selectors:
#   0x00000001: deploy(salt, initcode) — CREATE2
#   0x00000002: getAddress(salt, codehash) — compute CREATE2 address

ACCOUNT_DEPLOYER_RUNTIME = assemble("""
    ; === Dispatcher ===
    PUSH1 0x00
    CALLDATALOAD
    PUSH1 0xE0
    SHR

    DUP1
    PUSH4 0x00000001
    EQ
    PUSH1 :deploy
    JUMPI

    DUP1
    PUSH4 0x00000002
    EQ
    PUSH1 :getaddr
    JUMPI

    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === deploy(salt, initcode) ===
    ; calldata: selector(4) || salt(32) || initcode(variable)
deploy:
    JUMPDEST
    POP

    ; Copy initcode to memory
    CALLDATASIZE
    PUSH1 0x24                  ; 36
    SWAP1                       ; [36, calldatasize]
    SUB                         ; calldatasize - 36 = initcode_len
    DUP1                        ; initcode_len initcode_len
    PUSH1 0x24                  ; srcOffset
    PUSH1 0x00                  ; destOffset
    CALLDATACOPY                ; mem[0..] = initcode

    ; CREATE2(value, offset, size, salt)
    PUSH1 0x04
    CALLDATALOAD                ; salt
    SWAP1                       ; salt initcode_len
    PUSH1 0x00                  ; offset
    PUSH1 0x00                  ; value
    CREATE2                     ; -> address

    ; Check success
    DUP1
    ISZERO
    PUSH1 :revert_deploy
    JUMPI

    ; Return address
    PUSH1 0x00
    MSTORE
    PUSH1 0x20
    PUSH1 0x00
    RETURN

revert_deploy:
    JUMPDEST
    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === getAddress(salt, codehash) ===
    ; calldata: selector(4) || salt(32) || codehash(32)
    ; Returns: keccak256(0xff ++ deployer ++ salt ++ codehash)[12:]
getaddr:
    JUMPDEST
    POP

    ; Build 85-byte preimage in memory:
    ; mem[0] = 0xff
    PUSH1 0xFF
    PUSH1 0x00
    MSTORE8

    ; mem[1..21] = deployer address
    ADDRESS
    PUSH1 0x60                  ; 96 bits
    SHL                         ; left-align address
    PUSH1 0x01
    MSTORE                      ; writes 32 bytes at offset 1 (addr in first 20, zeros after)

    ; mem[21..53] = salt
    PUSH1 0x04
    CALLDATALOAD
    PUSH1 0x15                  ; 21
    MSTORE

    ; mem[53..85] = codehash
    PUSH1 0x24
    CALLDATALOAD
    PUSH1 0x35                  ; 53
    MSTORE

    ; keccak256(mem[0..85])
    PUSH1 0x55                  ; 85
    PUSH1 0x00
    SHA3

    ; Mask to address (low 20 bytes)
    PUSH20 0x000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
    AND

    PUSH1 0x00
    MSTORE
    PUSH1 0x20
    PUSH1 0x00
    RETURN
""")

ACCOUNT_DEPLOYER_INITCODE = wrap_initcode(ACCOUNT_DEPLOYER_RUNTIME)


# ============================================================
# MockERC20
# ============================================================
# Minimal ERC20 with mint. No events for simplicity.
# Storage: balances mapping at slot 0 (keccak256(addr . 0))
#
# Selectors (standard):
#   0x70a08231: balanceOf(address)
#   0xa9059cbb: transfer(address, uint256)
#   0x40c10f19: mint(address, uint256)

MOCK_ERC20_RUNTIME = assemble("""
    ; === Dispatcher ===
    PUSH1 0x00
    CALLDATALOAD
    PUSH1 0xE0
    SHR

    DUP1
    PUSH4 0x70a08231
    EQ
    PUSH1 :balanceof
    JUMPI

    DUP1
    PUSH4 0xa9059cbb
    EQ
    PUSH1 :transfer
    JUMPI

    DUP1
    PUSH4 0x40c10f19
    EQ
    PUSH1 :mint
    JUMPI

    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === balanceOf(address) -> uint256 ===
balanceof:
    JUMPDEST
    POP
    ; Compute storage slot: keccak256(abi.encode(addr, 0))
    PUSH1 0x04
    CALLDATALOAD                ; addr
    PUSH1 0x00
    MSTORE                      ; mem[0..32] = addr
    PUSH1 0x00
    PUSH1 0x20
    MSTORE                      ; mem[32..64] = 0 (slot index)
    PUSH1 0x40
    PUSH1 0x00
    SHA3                        ; storage key
    SLOAD                       ; balance
    PUSH1 0x00
    MSTORE
    PUSH1 0x20
    PUSH1 0x00
    RETURN

    ; === transfer(address to, uint256 amount) -> bool ===
transfer:
    JUMPDEST
    POP

    ; Compute sender's balance slot
    CALLER
    PUSH1 0x00
    MSTORE
    PUSH1 0x00
    PUSH1 0x20
    MSTORE
    PUSH1 0x40
    PUSH1 0x00
    SHA3                        ; sender_slot

    DUP1
    SLOAD                       ; sender_balance sender_slot

    ; Load amount
    PUSH1 0x24
    CALLDATALOAD                ; amount sender_balance sender_slot

    ; Check sender_balance >= amount
    DUP1                        ; amount amount sender_balance sender_slot
    DUP3                        ; sender_balance amount amount sender_balance sender_slot
    LT                          ; sender_balance < amount
    PUSH1 :revert_transfer
    JUMPI

    ; Subtract from sender
    ; stack: amount sender_balance sender_slot
    SWAP1                       ; sender_balance amount sender_slot
    SUB                         ; new_sender_balance sender_slot
    DUP2                        ; sender_slot new_sender_balance sender_slot
    SSTORE                      ; store | sender_slot
    POP                         ; clean

    ; Compute recipient's balance slot
    PUSH1 0x04
    CALLDATALOAD                ; to
    PUSH1 0x00
    MSTORE
    PUSH1 0x00
    PUSH1 0x20
    MSTORE
    PUSH1 0x40
    PUSH1 0x00
    SHA3                        ; to_slot

    DUP1
    SLOAD                       ; to_balance to_slot
    PUSH1 0x24
    CALLDATALOAD                ; amount to_balance to_slot
    ADD                         ; new_to_balance to_slot
    SWAP1
    SSTORE                      ; store

    ; Return true
    PUSH1 0x01
    PUSH1 0x00
    MSTORE
    PUSH1 0x20
    PUSH1 0x00
    RETURN

revert_transfer:
    JUMPDEST
    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === mint(address to, uint256 amount) ===
mint:
    JUMPDEST
    POP
    ; Compute to's balance slot
    PUSH1 0x04
    CALLDATALOAD
    PUSH1 0x00
    MSTORE
    PUSH1 0x00
    PUSH1 0x20
    MSTORE
    PUSH1 0x40
    PUSH1 0x00
    SHA3                        ; to_slot

    DUP1
    SLOAD                       ; balance to_slot
    PUSH1 0x24
    CALLDATALOAD                ; amount balance to_slot
    ADD                         ; new_balance to_slot
    SWAP1
    SSTORE

    STOP
""")

MOCK_ERC20_INITCODE = wrap_initcode(MOCK_ERC20_RUNTIME)


# ============================================================
# ERC20Sponsor
# ============================================================
# Gas sponsor that accepts payment in ERC20 tokens.
# Storage: slot 0 = token_address, slot 1 = rate (tokens per gas unit)
#
# Called in a VERIFY frame to approve as payer (scope=1).
# Checks sender has sufficient token balance.
#
# Selectors:
#   0x00000000 (or empty calldata): verify() — APPROVE as payer
#   0x00000001: setConfig(token_address, rate) — owner setup

ERC20_SPONSOR_RUNTIME = assemble("""
    ; === Dispatcher ===
    CALLDATASIZE
    ISZERO
    PUSH1 :receive
    JUMPI

    PUSH1 0x00
    CALLDATALOAD
    PUSH1 0xE0
    SHR

    ; selector 0x00000000 → verify (payer approval)
    DUP1
    ISZERO
    PUSH1 :verify
    JUMPI

    DUP1
    PUSH4 0x00000001
    EQ
    PUSH1 :setconfig
    JUMPI

    POP
    ; Unknown selector → revert
    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === receive() — accept ETH transfers (for funding) ===
receive:
    JUMPDEST
    STOP

    ; === verify() — approve as payer ===
verify:
    JUMPDEST
    POP                         ; pop selector

    ; Read sender from TXPARAMLOAD(0x02, 0)
    PUSH1 0x00
    PUSH1 0x02
    TXPARAMLOAD                 ; sender address on stack (as U256)

    ; Read token address from storage
    PUSH1 0x00
    SLOAD                       ; token_addr sender

    ; Check sender's token balance via STATICCALL to token.balanceOf(sender)
    ; Build calldata: 0x70a08231 + addr (32 bytes)
    PUSH4 0x70a08231
    PUSH1 0xe0
    SHL                         ; selector left-aligned
    PUSH1 0x00
    MSTORE                      ; mem[0..4] = selector (in high bytes of word)

    ; Store sender address at mem[4..36]
    ; sender is 2 down on stack: token_addr sender
    DUP2                        ; sender token_addr sender
    PUSH1 0x04
    MSTORE                      ; mem[4..36] = sender (right-aligned in 32 bytes... but MSTORE writes full 32 bytes at offset 4, which goes mem[4..36])

    ; STATICCALL(gas, addr, argsOffset, argsSize, retOffset, retSize)
    PUSH1 0x20                  ; retSize = 32
    PUSH1 0x24                  ; retOffset = 36
    PUSH1 0x24                  ; argsSize = 36 (4 selector + 32 addr)
    PUSH1 0x00                  ; argsOffset = 0
    DUP5                        ; token_addr (3 deep now: token_addr sender -> token_addr sender ...)
    GAS
    STATICCALL

    ; Check call succeeded
    ISZERO
    PUSH1 :revert_sponsor
    JUMPI

    ; Load balance result
    PUSH1 0x24
    MLOAD                       ; balance on stack
    ; stack: balance token_addr sender

    ; For simplicity, just check balance > 0
    ISZERO
    PUSH1 :revert_sponsor
    JUMPI

    ; Clean up stack
    POP                         ; token_addr
    POP                         ; sender

    ; APPROVE(offset=0, length=0, scope=1) — payer approval
    PUSH1 0x01                  ; scope = 1 (payer)
    PUSH1 0x00
    PUSH1 0x00
    APPROVE

revert_sponsor:
    JUMPDEST
    PUSH1 0x00
    PUSH1 0x00
    REVERT

    ; === setConfig(token_address, rate) ===
setconfig:
    JUMPDEST
    POP
    PUSH1 0x04
    CALLDATALOAD                ; token_address
    PUSH1 0x00
    SSTORE                      ; slot 0 = token_address

    PUSH1 0x24
    CALLDATALOAD                ; rate
    PUSH1 0x01
    SSTORE                      ; slot 1 = rate

    STOP
""")

# Constructor: store token_address and rate from code tail (same fix as P256 ctor)
_sponsor_ctor_prefix = assemble("""
    PUSH1 0x40
    CODESIZE
    PUSH1 0x40
    SWAP1
    SUB
    PUSH1 0x00
    CODECOPY

    PUSH1 0x00
    MLOAD
    PUSH1 0x00
    SSTORE

    PUSH1 0x20
    MLOAD
    PUSH1 0x01
    SSTORE
""")

_sponsor_runtime_len = len(ERC20_SPONSOR_RUNTIME)
if _sponsor_runtime_len <= 255:
    _s_suffix_len = 2 + 1 + 2 + 2 + 1 + 2 + 1  # = 11
    _s_full_ctor_len = len(_sponsor_ctor_prefix) + _s_suffix_len
    _sponsor_ctor_suffix = bytes([
        0x60, _sponsor_runtime_len,
        0x80,
        0x60, _s_full_ctor_len,
        0x60, 0x00,
        0x39,
        0x60, 0x00,
        0xF3,
    ])
else:
    _s_suffix_len = 3 + 1 + 2 + 2 + 1 + 2 + 1
    _s_full_ctor_len = len(_sponsor_ctor_prefix) + _s_suffix_len
    _sponsor_ctor_suffix = bytes([
        0x61, (_sponsor_runtime_len >> 8) & 0xFF, _sponsor_runtime_len & 0xFF,
        0x80,
        0x60, _s_full_ctor_len,
        0x60, 0x00,
        0x39,
        0x60, 0x00,
        0xF3,
    ])

ERC20_SPONSOR_INITCODE = _sponsor_ctor_prefix + _sponsor_ctor_suffix + ERC20_SPONSOR_RUNTIME


# ============================================================
# Print summary
# ============================================================
if __name__ == '__main__':
    contracts = {
        'SimpleP256Account': (SIMPLE_P256_ACCOUNT_RUNTIME, SIMPLE_P256_ACCOUNT_INITCODE),
        'AccountDeployer': (ACCOUNT_DEPLOYER_RUNTIME, ACCOUNT_DEPLOYER_INITCODE),
        'MockERC20': (MOCK_ERC20_RUNTIME, MOCK_ERC20_INITCODE),
        'ERC20Sponsor': (ERC20_SPONSOR_RUNTIME, ERC20_SPONSOR_INITCODE),
    }

    for name, (runtime, initcode) in contracts.items():
        print(f"\n{'='*60}")
        print(f"{name}")
        print(f"{'='*60}")
        print(f"  Runtime size: {len(runtime)} bytes")
        print(f"  Initcode size: {len(initcode)} bytes")
        print(f"  Runtime hex: 0x{runtime.hex()}")
        print(f"  Initcode hex: 0x{initcode.hex()}")
