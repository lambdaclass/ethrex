use crate::types::gas_costs;
use std::collections::BTreeMap;

pub fn process_bytecode(code: &[u8]) -> (Vec<u32>, Vec<(u32, u64)>) {
    debug_assert!(code.len() <= u32::MAX as usize);

    let mut costs = BTreeMap::<usize, u64>::new();
    let mut targets = Vec::new();

    let mut cost_entry: Option<&mut u64> = Some(costs.entry(0usize).or_default());

    let mut i = 0;
    while i < code.len() {
        match code[i] {
            // OP_STOP
            0x00 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::STOP;
                }
                cost_entry = None;
            }
            // OP_ADD
            0x01 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::ADD;
                }
            }
            // OP_MUL
            0x02 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MUL;
                }
            }
            // OP_SUB
            0x03 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SUB;
                }
            }
            // OP_DIV
            0x04 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::DIV;
                }
            }
            // OP_SDIV
            0x05 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SDIV;
                }
            }
            // OP_MOD
            0x06 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MOD;
                }
            }
            // OP_SMOD
            0x07 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SMOD;
                }
            }
            // OP_ADDMOD
            0x08 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::ADDMOD;
                }
            }
            // OP_MULMOD
            0x09 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MULMOD;
                }
            }
            // OP_EXP
            0x0A => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::EXP_STATIC;
                }
            }
            // OP_SIGNEXTEND
            0x0B => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SIGNEXTEND;
                }
            }
            // OP_LT
            0x10 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::LT;
                }
            }
            // OP_GT
            0x11 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::GT;
                }
            }
            // OP_SLT
            0x12 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SLT;
                }
            }
            // OP_SGT
            0x13 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SGT;
                }
            }
            // OP_EQ
            0x14 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::EQ;
                }
            }
            // OP_ISZERO
            0x15 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::ISZERO;
                }
            }
            // OP_AND
            0x16 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::AND;
                }
            }
            // OP_OR
            0x17 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::OR;
                }
            }
            // OP_XOR
            0x18 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::XOR;
                }
            }
            // OP_NOT
            0x19 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::NOT;
                }
            }
            // OP_BYTE
            0x1A => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::BYTE;
                }
            }
            // OP_SHL
            0x1B => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SHL;
                }
            }
            // OP_SHR
            0x1C => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SHR;
                }
            }
            // OP_SAR
            0x1D => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SAR;
                }
            }
            // OP_CLZ
            0x1E => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CLZ;
                }
            }
            // OP_KECCAK256
            0x20 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::KECCAK25_STATIC;
                }
            }
            // OP_ADDRESS
            0x30 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::ADDRESS;
                }
            }
            // OP_BALANCE
            0x31 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::BALANCE_STATIC;
                }
            }
            // OP_ORIGIN
            0x32 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::ORIGIN;
                }
            }
            // OP_CALLER
            0x33 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALLER;
                }
            }
            // OP_CALLVALUE
            0x34 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALLVALUE;
                }
            }
            // OP_CALLDATALOAD
            0x35 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALLDATALOAD;
                }
            }
            // OP_CALLDATASIZE
            0x36 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALLDATASIZE;
                }
            }
            // OP_CALLDATACOPY
            0x37 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALLDATACOPY_STATIC;
                }
            }
            // OP_CODESIZE
            0x38 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CODESIZE;
                }
            }
            // OP_CODECOPY
            0x39 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CODECOPY_STATIC;
                }
            }
            // OP_GASPRICE
            0x3A => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::GASPRICE;
                }
            }
            // OP_EXTCODESIZE
            0x3B => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::EXTCODESIZE_STATIC;
                }
            }
            // OP_EXTCODECOPY
            0x3C => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::EXTCODECOPY_STATIC;
                }
            }
            // OP_RETURNDATASIZE
            0x3D => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::RETURNDATASIZE;
                }
            }
            // OP_RETURNDATACOPY
            0x3E => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::RETURNDATACOPY_STATIC;
                }
            }
            // OP_EXTCODEHASH
            0x3F => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::EXTCODEHASH_STATIC;
                }
            }
            // OP_BLOCKHASH
            0x40 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::BLOCKHASH;
                }
            }
            // OP_COINBASE
            0x41 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::COINBASE;
                }
            }
            // OP_TIMESTAMP
            0x42 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::TIMESTAMP;
                }
            }
            // OP_NUMBER
            0x43 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::NUMBER;
                }
            }
            // OP_PREVRANDAO
            0x44 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::PREVRANDAO;
                }
            }
            // OP_GASLIMIT
            0x45 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::GASLIMIT;
                }
            }
            // OP_CHAINID
            0x46 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CHAINID;
                }
            }
            // OP_SELFBALANCE
            0x47 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SELFBALANCE;
                }
            }
            // OP_BASEFEE
            0x48 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::BASEFEE;
                }
            }
            // OP_BLOBHASH
            0x49 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::BLOBHASH;
                }
            }
            // OP_BLOBBASEFEE
            0x4A => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::BLOBBASEFEE;
                }
            }
            // OP_SLOTNUM
            0x4B => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SLOTNUM;
                }
            }
            // OP_POP
            0x50 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::POP;
                }
            }
            // OP_MLOAD
            0x51 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MLOAD_STATIC;
                }
            }
            // OP_MSTORE
            0x52 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MSTORE_STATIC;
                }
            }
            // OP_MSTORE8
            0x53 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MSTORE8_STATIC;
                }
            }
            // OP_SLOAD
            0x54 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SLOAD_STATIC;
                }
            }
            // OP_SSTORE
            0x55 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SSTORE_STATIC;
                }
            }
            // OP_JUMP (terminator)
            0x56 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::JUMP;
                }
                cost_entry = None;
            }
            // OP_JUMPI (terminator, but false branch falls through into a new block)
            0x57 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::JUMPI;
                }
                cost_entry = Some(costs.entry(i + 1).or_default());
            }
            // OP_PC
            0x58 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::PC;
                }
            }
            // OP_MSIZE
            0x59 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MSIZE;
                }
            }
            // OP_GAS
            0x5A => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::GAS;
                }
            }
            // OP_JUMPDEST
            0x5B => {
                cost_entry = Some(costs.entry(i).or_default());
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::JUMPDEST;
                }
                targets.push(i as u32);
            }
            // OP_TLOAD
            0x5C => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::TLOAD;
                }
            }
            // OP_TSTORE
            0x5D => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::TSTORE;
                }
            }
            // OP_MCOPY
            0x5E => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::MCOPY_STATIC;
                }
            }
            // OP_PUSH0
            0x5F => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::PUSH0;
                }
            }
            // OP_PUSH1..32
            c @ 0x60..0x80 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::PUSHN;
                }
                i += (c - 0x5F) as usize;
            }
            // OP_DUP1..16
            0x80..0x90 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::DUPN;
                }
            }
            // OP_SWAP1..16
            0x90..0xA0 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SWAPN;
                }
            }
            // OP_LOG0..4
            0xA0..0xA5 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::LOGN_STATIC;
                }
            }
            // OP_CREATE
            0xF0 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CREATE_BASE_COST;
                }
            }
            // OP_CALL
            0xF1 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALL_STATIC;
                }
            }
            // OP_CALLCODE
            0xF2 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CALLCODE_STATIC;
                }
            }
            // OP_RETURN (terminator)
            0xF3 => {
                cost_entry = None;
            }
            // OP_DELEGATECALL
            0xF4 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::DELEGATECALL_STATIC;
                }
            }
            // OP_CREATE2
            0xF5 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::CREATE_BASE_COST;
                }
            }
            // OP_STATICCALL
            0xFA => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::STATICCALL_STATIC;
                }
            }
            // OP_REVERT (terminator)
            0xFD => {
                cost_entry = None;
            }
            // OP_INVALID (terminator)
            0xFE => {
                cost_entry = None;
            }
            // OP_SELFDESTRUCT (terminator)
            0xFF => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SELFDESTRUCT_STATIC;
                }
                cost_entry = None;
            }
            // OP_DUPN (EIP-8024)
            0xE6 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::DUPN;
                }
            }
            // OP_SWAPN (EIP-8024)
            0xE7 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::SWAPN;
                }
            }
            // OP_EXCHANGE (EIP-8024)
            0xE8 => {
                if let Some(c) = cost_entry.as_deref_mut() {
                    *c += gas_costs::EXCHANGE;
                }
            }
            // Unknown/undefined opcodes - no static cost
            _ => (),
        }
        i += 1;
    }

    (
        targets,
        costs
            .into_iter()
            .map(|(pc, cost)| (pc as u32, cost))
            .collect(),
    )
}
