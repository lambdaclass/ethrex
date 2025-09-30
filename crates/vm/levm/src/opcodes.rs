use crate::{
    errors::{ExceptionalHalt, OpcodeResult, VMError},
    vm::VM,
};
use ethrex_common::types::Fork;
use strum::EnumString;

#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, EnumString)]
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
    CLZ = 0x1E,

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
            table[0x1E] = Opcode::CLZ;
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

/// Represents an opcode function handler.
///
/// The boolean return value is true if the VM's execution should be interrupted for any reason.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpCodeFn<'a>(fn(&'_ mut VM<'a>) -> Result<OpcodeResult, VMError>);

impl<'a> OpCodeFn<'a> {
    /// Call the opcode handler.
    #[inline(always)]
    pub fn call(self, vm: &mut VM<'a>) -> Result<OpcodeResult, VMError> {
        (self.0)(vm)
    }
}

impl<'a> VM<'a> {
    /// Setups the opcode lookup function pointer table, configured according the given fork.
    ///
    /// This is faster than a conventional match.
    #[allow(clippy::as_conversions, clippy::indexing_slicing)]
    pub(crate) fn build_opcode_table(fork: Fork) -> [OpCodeFn<'a>; 256] {
        if fork >= Fork::Osaka {
            Self::build_opcode_table_osaka()
        } else if fork >= Fork::Cancun {
            Self::build_opcode_table_pre_osaka()
        } else if fork >= Fork::Shanghai {
            Self::build_opcode_table_pre_cancun()
        } else {
            Self::build_opcode_table_pre_shanghai()
        }
    }

    #[allow(clippy::as_conversions, clippy::indexing_slicing)]
    const fn build_opcode_table_pre_shanghai() -> [OpCodeFn<'a>; 256] {
        let mut opcode_table: [OpCodeFn<'a>; 256] = [OpCodeFn(VM::on_invalid_opcode); 256];

        opcode_table[Opcode::STOP as usize] = OpCodeFn(VM::op_stop);
        opcode_table[Opcode::MLOAD as usize] = OpCodeFn(VM::op_mload);
        opcode_table[Opcode::MSTORE as usize] = OpCodeFn(VM::op_mstore);
        opcode_table[Opcode::MSTORE8 as usize] = OpCodeFn(VM::op_mstore8);
        opcode_table[Opcode::JUMP as usize] = OpCodeFn(VM::op_jump);
        opcode_table[Opcode::SLOAD as usize] = OpCodeFn(VM::op_sload);
        opcode_table[Opcode::SSTORE as usize] = OpCodeFn(VM::op_sstore);
        opcode_table[Opcode::MSIZE as usize] = OpCodeFn(VM::op_msize);
        opcode_table[Opcode::GAS as usize] = OpCodeFn(VM::op_gas);
        opcode_table[Opcode::PUSH1 as usize] = OpCodeFn(VM::op_push::<1>);
        opcode_table[Opcode::PUSH2 as usize] = OpCodeFn(VM::op_push::<2>);
        opcode_table[Opcode::PUSH3 as usize] = OpCodeFn(VM::op_push::<3>);
        opcode_table[Opcode::PUSH4 as usize] = OpCodeFn(VM::op_push::<4>);
        opcode_table[Opcode::PUSH5 as usize] = OpCodeFn(VM::op_push::<5>);
        opcode_table[Opcode::PUSH6 as usize] = OpCodeFn(VM::op_push::<6>);
        opcode_table[Opcode::PUSH7 as usize] = OpCodeFn(VM::op_push::<7>);
        opcode_table[Opcode::PUSH8 as usize] = OpCodeFn(VM::op_push::<8>);
        opcode_table[Opcode::PUSH8 as usize] = OpCodeFn(VM::op_push::<8>);
        opcode_table[Opcode::PUSH9 as usize] = OpCodeFn(VM::op_push::<9>);
        opcode_table[Opcode::PUSH10 as usize] = OpCodeFn(VM::op_push::<10>);
        opcode_table[Opcode::PUSH11 as usize] = OpCodeFn(VM::op_push::<11>);
        opcode_table[Opcode::PUSH12 as usize] = OpCodeFn(VM::op_push::<12>);
        opcode_table[Opcode::PUSH13 as usize] = OpCodeFn(VM::op_push::<13>);
        opcode_table[Opcode::PUSH14 as usize] = OpCodeFn(VM::op_push::<14>);
        opcode_table[Opcode::PUSH15 as usize] = OpCodeFn(VM::op_push::<15>);
        opcode_table[Opcode::PUSH16 as usize] = OpCodeFn(VM::op_push::<16>);
        opcode_table[Opcode::PUSH17 as usize] = OpCodeFn(VM::op_push::<17>);
        opcode_table[Opcode::PUSH18 as usize] = OpCodeFn(VM::op_push::<18>);
        opcode_table[Opcode::PUSH19 as usize] = OpCodeFn(VM::op_push::<19>);
        opcode_table[Opcode::PUSH20 as usize] = OpCodeFn(VM::op_push::<20>);
        opcode_table[Opcode::PUSH21 as usize] = OpCodeFn(VM::op_push::<21>);
        opcode_table[Opcode::PUSH22 as usize] = OpCodeFn(VM::op_push::<22>);
        opcode_table[Opcode::PUSH23 as usize] = OpCodeFn(VM::op_push::<23>);
        opcode_table[Opcode::PUSH24 as usize] = OpCodeFn(VM::op_push::<24>);
        opcode_table[Opcode::PUSH25 as usize] = OpCodeFn(VM::op_push::<25>);
        opcode_table[Opcode::PUSH26 as usize] = OpCodeFn(VM::op_push::<26>);
        opcode_table[Opcode::PUSH27 as usize] = OpCodeFn(VM::op_push::<27>);
        opcode_table[Opcode::PUSH28 as usize] = OpCodeFn(VM::op_push::<28>);
        opcode_table[Opcode::PUSH29 as usize] = OpCodeFn(VM::op_push::<29>);
        opcode_table[Opcode::PUSH30 as usize] = OpCodeFn(VM::op_push::<30>);
        opcode_table[Opcode::PUSH31 as usize] = OpCodeFn(VM::op_push::<31>);
        opcode_table[Opcode::PUSH32 as usize] = OpCodeFn(VM::op_push::<32>);

        opcode_table[Opcode::DUP1 as usize] = OpCodeFn(VM::op_dup::<0>);
        opcode_table[Opcode::DUP2 as usize] = OpCodeFn(VM::op_dup::<1>);
        opcode_table[Opcode::DUP3 as usize] = OpCodeFn(VM::op_dup::<2>);
        opcode_table[Opcode::DUP4 as usize] = OpCodeFn(VM::op_dup::<3>);
        opcode_table[Opcode::DUP5 as usize] = OpCodeFn(VM::op_dup::<4>);
        opcode_table[Opcode::DUP6 as usize] = OpCodeFn(VM::op_dup::<5>);
        opcode_table[Opcode::DUP7 as usize] = OpCodeFn(VM::op_dup::<6>);
        opcode_table[Opcode::DUP8 as usize] = OpCodeFn(VM::op_dup::<7>);
        opcode_table[Opcode::DUP9 as usize] = OpCodeFn(VM::op_dup::<8>);
        opcode_table[Opcode::DUP10 as usize] = OpCodeFn(VM::op_dup::<9>);
        opcode_table[Opcode::DUP11 as usize] = OpCodeFn(VM::op_dup::<10>);
        opcode_table[Opcode::DUP12 as usize] = OpCodeFn(VM::op_dup::<11>);
        opcode_table[Opcode::DUP13 as usize] = OpCodeFn(VM::op_dup::<12>);
        opcode_table[Opcode::DUP14 as usize] = OpCodeFn(VM::op_dup::<13>);
        opcode_table[Opcode::DUP15 as usize] = OpCodeFn(VM::op_dup::<14>);
        opcode_table[Opcode::DUP16 as usize] = OpCodeFn(VM::op_dup::<15>);

        opcode_table[Opcode::SWAP1 as usize] = OpCodeFn(VM::op_swap::<1>);
        opcode_table[Opcode::SWAP2 as usize] = OpCodeFn(VM::op_swap::<2>);
        opcode_table[Opcode::SWAP3 as usize] = OpCodeFn(VM::op_swap::<3>);
        opcode_table[Opcode::SWAP4 as usize] = OpCodeFn(VM::op_swap::<4>);
        opcode_table[Opcode::SWAP5 as usize] = OpCodeFn(VM::op_swap::<5>);
        opcode_table[Opcode::SWAP6 as usize] = OpCodeFn(VM::op_swap::<6>);
        opcode_table[Opcode::SWAP7 as usize] = OpCodeFn(VM::op_swap::<7>);
        opcode_table[Opcode::SWAP8 as usize] = OpCodeFn(VM::op_swap::<8>);
        opcode_table[Opcode::SWAP9 as usize] = OpCodeFn(VM::op_swap::<9>);
        opcode_table[Opcode::SWAP10 as usize] = OpCodeFn(VM::op_swap::<10>);
        opcode_table[Opcode::SWAP11 as usize] = OpCodeFn(VM::op_swap::<11>);
        opcode_table[Opcode::SWAP12 as usize] = OpCodeFn(VM::op_swap::<12>);
        opcode_table[Opcode::SWAP13 as usize] = OpCodeFn(VM::op_swap::<13>);
        opcode_table[Opcode::SWAP14 as usize] = OpCodeFn(VM::op_swap::<14>);
        opcode_table[Opcode::SWAP15 as usize] = OpCodeFn(VM::op_swap::<15>);
        opcode_table[Opcode::SWAP16 as usize] = OpCodeFn(VM::op_swap::<16>);
        opcode_table[Opcode::POP as usize] = OpCodeFn(VM::op_pop);
        opcode_table[Opcode::ADD as usize] = OpCodeFn(VM::op_add);
        opcode_table[Opcode::MUL as usize] = OpCodeFn(VM::op_mul);
        opcode_table[Opcode::SUB as usize] = OpCodeFn(VM::op_sub);
        opcode_table[Opcode::DIV as usize] = OpCodeFn(VM::op_div);
        opcode_table[Opcode::SDIV as usize] = OpCodeFn(VM::op_sdiv);
        opcode_table[Opcode::MOD as usize] = OpCodeFn(VM::op_mod);
        opcode_table[Opcode::SMOD as usize] = OpCodeFn(VM::op_smod);
        opcode_table[Opcode::ADDMOD as usize] = OpCodeFn(VM::op_addmod);
        opcode_table[Opcode::MULMOD as usize] = OpCodeFn(VM::op_mulmod);
        opcode_table[Opcode::EXP as usize] = OpCodeFn(VM::op_exp);
        opcode_table[Opcode::CALL as usize] = OpCodeFn(VM::op_call);
        opcode_table[Opcode::CALLCODE as usize] = OpCodeFn(VM::op_callcode);
        opcode_table[Opcode::RETURN as usize] = OpCodeFn(VM::op_return);
        opcode_table[Opcode::DELEGATECALL as usize] = OpCodeFn(VM::op_delegatecall);
        opcode_table[Opcode::STATICCALL as usize] = OpCodeFn(VM::op_staticcall);
        opcode_table[Opcode::CREATE as usize] = OpCodeFn(VM::op_create);
        opcode_table[Opcode::CREATE2 as usize] = OpCodeFn(VM::op_create2);
        opcode_table[Opcode::JUMPI as usize] = OpCodeFn(VM::op_jumpi);
        opcode_table[Opcode::JUMPDEST as usize] = OpCodeFn(VM::op_jumpdest);
        opcode_table[Opcode::ADDRESS as usize] = OpCodeFn(VM::op_address);
        opcode_table[Opcode::ORIGIN as usize] = OpCodeFn(VM::op_origin);
        opcode_table[Opcode::BALANCE as usize] = OpCodeFn(VM::op_balance);
        opcode_table[Opcode::CALLER as usize] = OpCodeFn(VM::op_caller);
        opcode_table[Opcode::CALLVALUE as usize] = OpCodeFn(VM::op_callvalue);
        opcode_table[Opcode::CODECOPY as usize] = OpCodeFn(VM::op_codecopy);
        opcode_table[Opcode::SIGNEXTEND as usize] = OpCodeFn(VM::op_signextend);
        opcode_table[Opcode::LT as usize] = OpCodeFn(VM::op_lt);
        opcode_table[Opcode::GT as usize] = OpCodeFn(VM::op_gt);
        opcode_table[Opcode::SLT as usize] = OpCodeFn(VM::op_slt);
        opcode_table[Opcode::SGT as usize] = OpCodeFn(VM::op_sgt);
        opcode_table[Opcode::EQ as usize] = OpCodeFn(VM::op_eq);
        opcode_table[Opcode::ISZERO as usize] = OpCodeFn(VM::op_iszero);
        opcode_table[Opcode::KECCAK256 as usize] = OpCodeFn(VM::op_keccak256);
        opcode_table[Opcode::CALLDATALOAD as usize] = OpCodeFn(VM::op_calldataload);
        opcode_table[Opcode::CALLDATASIZE as usize] = OpCodeFn(VM::op_calldatasize);
        opcode_table[Opcode::CALLDATACOPY as usize] = OpCodeFn(VM::op_calldatacopy);
        opcode_table[Opcode::RETURNDATASIZE as usize] = OpCodeFn(VM::op_returndatasize);
        opcode_table[Opcode::RETURNDATACOPY as usize] = OpCodeFn(VM::op_returndatacopy);
        opcode_table[Opcode::PC as usize] = OpCodeFn(VM::op_pc);
        opcode_table[Opcode::BLOCKHASH as usize] = OpCodeFn(VM::op_blockhash);
        opcode_table[Opcode::COINBASE as usize] = OpCodeFn(VM::op_coinbase);
        opcode_table[Opcode::TIMESTAMP as usize] = OpCodeFn(VM::op_timestamp);
        opcode_table[Opcode::NUMBER as usize] = OpCodeFn(VM::op_number);
        opcode_table[Opcode::PREVRANDAO as usize] = OpCodeFn(VM::op_prevrandao);
        opcode_table[Opcode::GASLIMIT as usize] = OpCodeFn(VM::op_gaslimit);
        opcode_table[Opcode::CHAINID as usize] = OpCodeFn(VM::op_chainid);
        opcode_table[Opcode::BASEFEE as usize] = OpCodeFn(VM::op_basefee);
        opcode_table[Opcode::AND as usize] = OpCodeFn(VM::op_and);
        opcode_table[Opcode::OR as usize] = OpCodeFn(VM::op_or);
        opcode_table[Opcode::XOR as usize] = OpCodeFn(VM::op_xor);
        opcode_table[Opcode::NOT as usize] = OpCodeFn(VM::op_not);
        opcode_table[Opcode::BYTE as usize] = OpCodeFn(VM::op_byte);
        opcode_table[Opcode::SHL as usize] = OpCodeFn(VM::op_shl);
        opcode_table[Opcode::SHR as usize] = OpCodeFn(VM::op_shr);
        opcode_table[Opcode::SAR as usize] = OpCodeFn(VM::op_sar);
        opcode_table[Opcode::SELFBALANCE as usize] = OpCodeFn(VM::op_selfbalance);
        opcode_table[Opcode::CODESIZE as usize] = OpCodeFn(VM::op_codesize);
        opcode_table[Opcode::GASPRICE as usize] = OpCodeFn(VM::op_gasprice);
        opcode_table[Opcode::EXTCODESIZE as usize] = OpCodeFn(VM::op_extcodesize);
        opcode_table[Opcode::EXTCODECOPY as usize] = OpCodeFn(VM::op_extcodecopy);
        opcode_table[Opcode::EXTCODEHASH as usize] = OpCodeFn(VM::op_extcodehash);
        opcode_table[Opcode::REVERT as usize] = OpCodeFn(VM::op_revert);
        opcode_table[Opcode::INVALID as usize] = OpCodeFn(VM::op_invalid);
        opcode_table[Opcode::SELFDESTRUCT as usize] = OpCodeFn(VM::op_selfdestruct);

        opcode_table[Opcode::LOG0 as usize] = OpCodeFn(VM::op_log::<0>);
        opcode_table[Opcode::LOG1 as usize] = OpCodeFn(VM::op_log::<1>);
        opcode_table[Opcode::LOG2 as usize] = OpCodeFn(VM::op_log::<2>);
        opcode_table[Opcode::LOG3 as usize] = OpCodeFn(VM::op_log::<3>);
        opcode_table[Opcode::LOG4 as usize] = OpCodeFn(VM::op_log::<4>);

        opcode_table
    }

    #[allow(clippy::as_conversions, clippy::indexing_slicing)]
    const fn build_opcode_table_pre_cancun() -> [OpCodeFn<'a>; 256] {
        let mut opcode_table: [OpCodeFn<'a>; 256] = Self::build_opcode_table_pre_shanghai();

        // [EIP-3855] - PUSH0 is only available from SHANGHAI
        opcode_table[Opcode::PUSH0 as usize] = OpCodeFn(VM::op_push0);
        opcode_table
    }

    #[allow(clippy::as_conversions, clippy::indexing_slicing)]
    const fn build_opcode_table_pre_osaka() -> [OpCodeFn<'a>; 256] {
        const {
            let mut opcode_table: [OpCodeFn<'a>; 256] = Self::build_opcode_table_pre_cancun();

            // [EIP-5656] - MCOPY is only available from CANCUN
            opcode_table[Opcode::MCOPY as usize] = OpCodeFn(VM::op_mcopy);

            // [EIP-1153] - TLOAD is only available from CANCUN
            opcode_table[Opcode::TLOAD as usize] = OpCodeFn(VM::op_tload);
            opcode_table[Opcode::TSTORE as usize] = OpCodeFn(VM::op_tstore);

            // [EIP-7516] - BLOBBASEFEE is only available from CANCUN
            opcode_table[Opcode::BLOBBASEFEE as usize] = OpCodeFn(VM::op_blobbasefee);
            // [EIP-4844] - BLOBHASH is only available from CANCUN
            opcode_table[Opcode::BLOBHASH as usize] = OpCodeFn(VM::op_blobhash);
            opcode_table
        }
    }

    #[allow(clippy::as_conversions, clippy::indexing_slicing)]
    const fn build_opcode_table_osaka() -> [OpCodeFn<'a>; 256] {
        let mut opcode_table: [OpCodeFn<'a>; 256] = Self::build_opcode_table_pre_osaka();

        opcode_table[Opcode::CLZ as usize] = OpCodeFn(VM::op_clz);
        opcode_table
    }

    /// Used within the opcode table for invalid opcodes.
    pub fn on_invalid_opcode(&mut self) -> Result<OpcodeResult, VMError> {
        Err(ExceptionalHalt::InvalidOpcode.into())
    }

    pub fn op_stop(&mut self) -> Result<OpcodeResult, VMError> {
        Ok(OpcodeResult::Halt)
    }
}
