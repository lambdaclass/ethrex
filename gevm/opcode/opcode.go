package opcode

// Opcode constants for the EVM.
// Every valid opcode.
const (
	STOP       byte = 0x00
	ADD        byte = 0x01
	MUL        byte = 0x02
	SUB        byte = 0x03
	DIV        byte = 0x04
	SDIV       byte = 0x05
	MOD        byte = 0x06
	SMOD       byte = 0x07
	ADDMOD     byte = 0x08
	MULMOD     byte = 0x09
	EXP        byte = 0x0A
	SIGNEXTEND byte = 0x0B

	LT     byte = 0x10
	GT     byte = 0x11
	SLT    byte = 0x12
	SGT    byte = 0x13
	EQ     byte = 0x14
	ISZERO byte = 0x15
	AND    byte = 0x16
	OR     byte = 0x17
	XOR    byte = 0x18
	NOT    byte = 0x19
	BYTE   byte = 0x1A
	SHL    byte = 0x1B
	SHR    byte = 0x1C
	SAR    byte = 0x1D
	CLZ    byte = 0x1E

	KECCAK256 byte = 0x20

	ADDRESS        byte = 0x30
	BALANCE        byte = 0x31
	ORIGIN         byte = 0x32
	CALLER         byte = 0x33
	CALLVALUE      byte = 0x34
	CALLDATALOAD   byte = 0x35
	CALLDATASIZE   byte = 0x36
	CALLDATACOPY   byte = 0x37
	CODESIZE       byte = 0x38
	CODECOPY       byte = 0x39
	GASPRICE       byte = 0x3A
	EXTCODESIZE    byte = 0x3B
	EXTCODECOPY    byte = 0x3C
	RETURNDATASIZE byte = 0x3D
	RETURNDATACOPY byte = 0x3E
	EXTCODEHASH    byte = 0x3F

	BLOCKHASH   byte = 0x40
	COINBASE    byte = 0x41
	TIMESTAMP   byte = 0x42
	NUMBER      byte = 0x43
	DIFFICULTY  byte = 0x44
	GASLIMIT    byte = 0x45
	CHAINID     byte = 0x46
	SELFBALANCE byte = 0x47
	BASEFEE     byte = 0x48
	BLOBHASH    byte = 0x49
	BLOBBASEFEE byte = 0x4A
	SLOTNUM     byte = 0x4B

	POP      byte = 0x50
	MLOAD    byte = 0x51
	MSTORE   byte = 0x52
	MSTORE8  byte = 0x53
	SLOAD    byte = 0x54
	SSTORE   byte = 0x55
	JUMP     byte = 0x56
	JUMPI    byte = 0x57
	PC       byte = 0x58
	MSIZE    byte = 0x59
	GAS      byte = 0x5A
	JUMPDEST byte = 0x5B
	TLOAD    byte = 0x5C
	TSTORE   byte = 0x5D
	MCOPY    byte = 0x5E

	PUSH0  byte = 0x5F
	PUSH1  byte = 0x60
	PUSH2  byte = 0x61
	PUSH3  byte = 0x62
	PUSH4  byte = 0x63
	PUSH5  byte = 0x64
	PUSH6  byte = 0x65
	PUSH7  byte = 0x66
	PUSH8  byte = 0x67
	PUSH9  byte = 0x68
	PUSH10 byte = 0x69
	PUSH11 byte = 0x6A
	PUSH12 byte = 0x6B
	PUSH13 byte = 0x6C
	PUSH14 byte = 0x6D
	PUSH15 byte = 0x6E
	PUSH16 byte = 0x6F
	PUSH17 byte = 0x70
	PUSH18 byte = 0x71
	PUSH19 byte = 0x72
	PUSH20 byte = 0x73
	PUSH21 byte = 0x74
	PUSH22 byte = 0x75
	PUSH23 byte = 0x76
	PUSH24 byte = 0x77
	PUSH25 byte = 0x78
	PUSH26 byte = 0x79
	PUSH27 byte = 0x7A
	PUSH28 byte = 0x7B
	PUSH29 byte = 0x7C
	PUSH30 byte = 0x7D
	PUSH31 byte = 0x7E
	PUSH32 byte = 0x7F

	DUP1  byte = 0x80
	DUP2  byte = 0x81
	DUP3  byte = 0x82
	DUP4  byte = 0x83
	DUP5  byte = 0x84
	DUP6  byte = 0x85
	DUP7  byte = 0x86
	DUP8  byte = 0x87
	DUP9  byte = 0x88
	DUP10 byte = 0x89
	DUP11 byte = 0x8A
	DUP12 byte = 0x8B
	DUP13 byte = 0x8C
	DUP14 byte = 0x8D
	DUP15 byte = 0x8E
	DUP16 byte = 0x8F

	SWAP1  byte = 0x90
	SWAP2  byte = 0x91
	SWAP3  byte = 0x92
	SWAP4  byte = 0x93
	SWAP5  byte = 0x94
	SWAP6  byte = 0x95
	SWAP7  byte = 0x96
	SWAP8  byte = 0x97
	SWAP9  byte = 0x98
	SWAP10 byte = 0x99
	SWAP11 byte = 0x9A
	SWAP12 byte = 0x9B
	SWAP13 byte = 0x9C
	SWAP14 byte = 0x9D
	SWAP15 byte = 0x9E
	SWAP16 byte = 0x9F

	LOG0 byte = 0xA0
	LOG1 byte = 0xA1
	LOG2 byte = 0xA2
	LOG3 byte = 0xA3
	LOG4 byte = 0xA4

	// EOF stack opcodes
	DUPN     byte = 0xE6
	SWAPN    byte = 0xE7
	EXCHANGE byte = 0xE8

	CREATE       byte = 0xF0
	CALL         byte = 0xF1
	CALLCODE     byte = 0xF2
	RETURN       byte = 0xF3
	DELEGATECALL byte = 0xF4
	CREATE2      byte = 0xF5

	STATICCALL   byte = 0xFA
	REVERT       byte = 0xFD
	INVALID      byte = 0xFE
	SELFDESTRUCT byte = 0xFF
)

// NOP is an alias for JUMPDEST.
const NOP = JUMPDEST
