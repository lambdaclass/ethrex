#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd)]
pub enum Opcode {
    // Stop and Arithmetic Operations
    STOP = 0x00,
    ADD = 0x01,
    MUL = 0x02,
    SUB = 0x03,
    DIV = 0x04,
    SDIV = 0x05,
    MOD = 0x06,
    SMOD = 0x07,
    ADDMOD = 0x08,
    MULMOD = 0x09,
    EXP = 0x0A,
    SIGNEXTEND = 0x0B,

    // Comparison & Bitwise Logic Operations
    LT = 0x10,
    GT = 0x11,
    SLT = 0x12,
    SGT = 0x13,
    EQ = 0x14,
    ISZERO = 0x15,
    AND = 0x16,
    OR = 0x17,
    XOR = 0x18,
    NOT = 0x19,
    BYTE = 0x1A,
    SHL = 0x1B,
    SHR = 0x1C,
    SAR = 0x1D,

    // KECCAK256
    KECCAK256 = 0x20,

    // Environmental Information
    ADDRESS = 0x30,
    BALANCE = 0x31,
    ORIGIN = 0x32,
    CALLER = 0x33,
    CALLVALUE = 0x34,
    CALLDATALOAD = 0x35,
    CALLDATASIZE = 0x36,
    CALLDATACOPY = 0x37,
    CODESIZE = 0x38,
    CODECOPY = 0x39,
    GASPRICE = 0x3A,
    EXTCODESIZE = 0x3B,
    EXTCODECOPY = 0x3C,
    RETURNDATASIZE = 0x3D,
    RETURNDATACOPY = 0x3E,
    EXTCODEHASH = 0x3F,

    // Block Information
    BLOCKHASH = 0x40,
    COINBASE = 0x41,
    TIMESTAMP = 0x42,
    NUMBER = 0x43,
    PREVRANDAO = 0x44,
    GASLIMIT = 0x45,
    CHAINID = 0x46,
    SELFBALANCE = 0x47,
    BASEFEE = 0x48,
    BLOBHASH = 0x49,
    BLOBBASEFEE = 0x4A,

    // Stack, Memory, Storage, and Flow Operations
    POP = 0x50,
    MLOAD = 0x51,
    MSTORE = 0x52,
    MSTORE8 = 0x53,
    SLOAD = 0x54,
    SSTORE = 0x55,
    JUMP = 0x56,
    JUMPI = 0x57,
    PC = 0x58,
    MSIZE = 0x59,
    GAS = 0x5A,
    JUMPDEST = 0x5B,
    TLOAD = 0x5C,
    TSTORE = 0x5D,
    MCOPY = 0x5E,

    // Push Operations
    PUSH0 = 0x5F,
    PUSH1 = 0x60,
    PUSH2 = 0x61,
    PUSH3 = 0x62,
    PUSH4 = 0x63,
    PUSH5 = 0x64,
    PUSH6 = 0x65,
    PUSH7 = 0x66,
    PUSH8 = 0x67,
    PUSH9 = 0x68,
    PUSH10 = 0x69,
    PUSH11 = 0x6A,
    PUSH12 = 0x6B,
    PUSH13 = 0x6C,
    PUSH14 = 0x6D,
    PUSH15 = 0x6E,
    PUSH16 = 0x6F,
    PUSH17 = 0x70,
    PUSH18 = 0x71,
    PUSH19 = 0x72,
    PUSH20 = 0x73,
    PUSH21 = 0x74,
    PUSH22 = 0x75,
    PUSH23 = 0x76,
    PUSH24 = 0x77,
    PUSH25 = 0x78,
    PUSH26 = 0x79,
    PUSH27 = 0x7A,
    PUSH28 = 0x7B,
    PUSH29 = 0x7C,
    PUSH30 = 0x7D,
    PUSH31 = 0x7E,
    PUSH32 = 0x7F,

    // Duplication Operations
    DUP1 = 0x80,
    DUP2 = 0x81,
    DUP3 = 0x82,
    DUP4 = 0x83,
    DUP5 = 0x84,
    DUP6 = 0x85,
    DUP7 = 0x86,
    DUP8 = 0x87,
    DUP9 = 0x88,
    DUP10 = 0x89,
    DUP11 = 0x8A,
    DUP12 = 0x8B,
    DUP13 = 0x8C,
    DUP14 = 0x8D,
    DUP15 = 0x8E,
    DUP16 = 0x8F,

    // Swap Operations
    SWAP1 = 0x90,
    SWAP2 = 0x91,
    SWAP3 = 0x92,
    SWAP4 = 0x93,
    SWAP5 = 0x94,
    SWAP6 = 0x95,
    SWAP7 = 0x96,
    SWAP8 = 0x97,
    SWAP9 = 0x98,
    SWAP10 = 0x99,
    SWAP11 = 0x9A,
    SWAP12 = 0x9B,
    SWAP13 = 0x9C,
    SWAP14 = 0x9D,
    SWAP15 = 0x9E,
    SWAP16 = 0x9F,
    // Logging Operations
    LOG0 = 0xA0,
    LOG1 = 0xA1,
    LOG2 = 0xA2,
    LOG3 = 0xA3,
    LOG4 = 0xA4,
    // // System Operations
    CREATE = 0xF0,
    CALL = 0xF1,
    CALLCODE = 0xF2,
    RETURN = 0xF3,
    DELEGATECALL = 0xF4,
    CREATE2 = 0xF5,
    STATICCALL = 0xFA,
    REVERT = 0xFD,
    INVALID = 0xFE,
    SELFDESTRUCT = 0xFF,
}

impl From<u8> for Opcode {
    #[expect(clippy::as_conversions)]
    fn from(byte: u8) -> Self {
        // We use a manual lookup table instead of a match because it gives improved perfomance
        // See https://godbolt.org/z/eG8M1jz3M
        const OPCODE_TABLE: [Opcode; 256] = const {
            let mut table = [Opcode::INVALID; 256];
            table[0x00] = Opcode::STOP;
            table[0x01] = Opcode::ADD;
            table[0x16] = Opcode::AND;
            table[0x17] = Opcode::OR;
            table[0x18] = Opcode::XOR;
            table[0x19] = Opcode::NOT;
            table[0x1A] = Opcode::BYTE;
            table[0x1B] = Opcode::SHL;
            table[0x1C] = Opcode::SHR;
            table[0x1D] = Opcode::SAR;
            table[0x02] = Opcode::MUL;
            table[0x03] = Opcode::SUB;
            table[0x04] = Opcode::DIV;
            table[0x05] = Opcode::SDIV;
            table[0x06] = Opcode::MOD;
            table[0x07] = Opcode::SMOD;
            table[0x08] = Opcode::ADDMOD;
            table[0x09] = Opcode::MULMOD;
            table[0x0A] = Opcode::EXP;
            table[0x0B] = Opcode::SIGNEXTEND;
            table[0x10] = Opcode::LT;
            table[0x11] = Opcode::GT;
            table[0x12] = Opcode::SLT;
            table[0x13] = Opcode::SGT;
            table[0x14] = Opcode::EQ;
            table[0x15] = Opcode::ISZERO;
            table[0x20] = Opcode::KECCAK256;
            table[0x30] = Opcode::ADDRESS;
            table[0x31] = Opcode::BALANCE;
            table[0x32] = Opcode::ORIGIN;
            table[0x33] = Opcode::CALLER;
            table[0x34] = Opcode::CALLVALUE;
            table[0x35] = Opcode::CALLDATALOAD;
            table[0x36] = Opcode::CALLDATASIZE;
            table[0x37] = Opcode::CALLDATACOPY;
            table[0x38] = Opcode::CODESIZE;
            table[0x39] = Opcode::CODECOPY;
            table[0x3A] = Opcode::GASPRICE;
            table[0x3B] = Opcode::EXTCODESIZE;
            table[0x3C] = Opcode::EXTCODECOPY;
            table[0x3D] = Opcode::RETURNDATASIZE;
            table[0x3E] = Opcode::RETURNDATACOPY;
            table[0x3F] = Opcode::EXTCODEHASH;
            table[0x40] = Opcode::BLOCKHASH;
            table[0x41] = Opcode::COINBASE;
            table[0x42] = Opcode::TIMESTAMP;
            table[0x43] = Opcode::NUMBER;
            table[0x44] = Opcode::PREVRANDAO;
            table[0x45] = Opcode::GASLIMIT;
            table[0x46] = Opcode::CHAINID;
            table[0x47] = Opcode::SELFBALANCE;
            table[0x48] = Opcode::BASEFEE;
            table[0x49] = Opcode::BLOBHASH;
            table[0x4A] = Opcode::BLOBBASEFEE;
            table[0x50] = Opcode::POP;
            table[0x56] = Opcode::JUMP;
            table[0x57] = Opcode::JUMPI;
            table[0x58] = Opcode::PC;
            table[0x5B] = Opcode::JUMPDEST;
            table[0x5F] = Opcode::PUSH0;
            table[0x60] = Opcode::PUSH1;
            table[0x61] = Opcode::PUSH2;
            table[0x62] = Opcode::PUSH3;
            table[0x63] = Opcode::PUSH4;
            table[0x64] = Opcode::PUSH5;
            table[0x65] = Opcode::PUSH6;
            table[0x66] = Opcode::PUSH7;
            table[0x67] = Opcode::PUSH8;
            table[0x68] = Opcode::PUSH9;
            table[0x69] = Opcode::PUSH10;
            table[0x6A] = Opcode::PUSH11;
            table[0x6B] = Opcode::PUSH12;
            table[0x6C] = Opcode::PUSH13;
            table[0x6D] = Opcode::PUSH14;
            table[0x6E] = Opcode::PUSH15;
            table[0x6F] = Opcode::PUSH16;
            table[0x70] = Opcode::PUSH17;
            table[0x71] = Opcode::PUSH18;
            table[0x72] = Opcode::PUSH19;
            table[0x73] = Opcode::PUSH20;
            table[0x74] = Opcode::PUSH21;
            table[0x75] = Opcode::PUSH22;
            table[0x76] = Opcode::PUSH23;
            table[0x77] = Opcode::PUSH24;
            table[0x78] = Opcode::PUSH25;
            table[0x79] = Opcode::PUSH26;
            table[0x7A] = Opcode::PUSH27;
            table[0x7B] = Opcode::PUSH28;
            table[0x7C] = Opcode::PUSH29;
            table[0x7D] = Opcode::PUSH30;
            table[0x7E] = Opcode::PUSH31;
            table[0x7F] = Opcode::PUSH32;
            table[0x80] = Opcode::DUP1;
            table[0x81] = Opcode::DUP2;
            table[0x82] = Opcode::DUP3;
            table[0x83] = Opcode::DUP4;
            table[0x84] = Opcode::DUP5;
            table[0x85] = Opcode::DUP6;
            table[0x86] = Opcode::DUP7;
            table[0x87] = Opcode::DUP8;
            table[0x88] = Opcode::DUP9;
            table[0x89] = Opcode::DUP10;
            table[0x8A] = Opcode::DUP11;
            table[0x8B] = Opcode::DUP12;
            table[0x8C] = Opcode::DUP13;
            table[0x8D] = Opcode::DUP14;
            table[0x8E] = Opcode::DUP15;
            table[0x8F] = Opcode::DUP16;
            table[0x90] = Opcode::SWAP1;
            table[0x91] = Opcode::SWAP2;
            table[0x92] = Opcode::SWAP3;
            table[0x93] = Opcode::SWAP4;
            table[0x94] = Opcode::SWAP5;
            table[0x95] = Opcode::SWAP6;
            table[0x96] = Opcode::SWAP7;
            table[0x97] = Opcode::SWAP8;
            table[0x98] = Opcode::SWAP9;
            table[0x99] = Opcode::SWAP10;
            table[0x9A] = Opcode::SWAP11;
            table[0x9B] = Opcode::SWAP12;
            table[0x9C] = Opcode::SWAP13;
            table[0x9D] = Opcode::SWAP14;
            table[0x9E] = Opcode::SWAP15;
            table[0x9F] = Opcode::SWAP16;
            table[0xA0] = Opcode::LOG0;
            table[0xA1] = Opcode::LOG1;
            table[0xA2] = Opcode::LOG2;
            table[0xA3] = Opcode::LOG3;
            table[0xA4] = Opcode::LOG4;
            table[0x51] = Opcode::MLOAD;
            table[0x52] = Opcode::MSTORE;
            table[0x53] = Opcode::MSTORE8;
            table[0x54] = Opcode::SLOAD;
            table[0x55] = Opcode::SSTORE;
            table[0x59] = Opcode::MSIZE;
            table[0x5A] = Opcode::GAS;
            table[0x5E] = Opcode::MCOPY;
            table[0x5C] = Opcode::TLOAD;
            table[0x5D] = Opcode::TSTORE;
            table[0xF0] = Opcode::CREATE;
            table[0xF1] = Opcode::CALL;
            table[0xF2] = Opcode::CALLCODE;
            table[0xF3] = Opcode::RETURN;
            table[0xF5] = Opcode::CREATE2;
            table[0xF4] = Opcode::DELEGATECALL;
            table[0xFA] = Opcode::STATICCALL;
            table[0xFD] = Opcode::REVERT;
            table[0xFF] = Opcode::SELFDESTRUCT;

            table
        };
        #[expect(clippy::indexing_slicing)] // can never happen
        OPCODE_TABLE[byte as usize]
    }
}

impl From<Opcode> for u8 {
    #[allow(clippy::as_conversions)]
    fn from(opcode: Opcode) -> Self {
        opcode as u8
    }
}

impl From<Opcode> for usize {
    #[allow(clippy::as_conversions)]
    fn from(opcode: Opcode) -> Self {
        opcode as usize
    }
}
