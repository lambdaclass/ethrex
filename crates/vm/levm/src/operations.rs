use crate::{errors::VMError, opcodes::Opcode};
use bytes::Bytes;
use ethereum_rust_core::U256;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Operation {
    Stop,
    Add,
    Mul,
    Sub,
    Div,
    Sdiv,
    Mod,
    SMod,
    Addmod,
    Mulmod,
    Exp,
    SignExtend,
    Lt,
    Gt,
    Slt,
    Sgt,
    Eq,
    IsZero,
    And,
    Or,
    Xor,
    Not,
    Byte,
    Shl,
    Shr,
    Sar,
    Keccak256,
    Address,
    Balance,
    Origin,
    Caller,
    Callvalue,
    CallDataLoad,
    CallDataSize,
    CallDataCopy,
    Codesize,
    Codecopy,
    Gasprice,
    ExtcodeSize,
    ExtcodeCopy,
    ReturnDataSize,
    ReturnDataCopy,
    ExtcodeHash,
    BlockHash,
    Coinbase,
    Timestamp,
    Number,
    Prevrandao,
    Gaslimit,
    Chainid,
    SelfBalance,
    Basefee,
    BlobHash,
    BlobBaseFee,
    Pop,
    Mload,
    Mstore,
    Mstore8,
    Sload,
    Sstore,
    Jump,
    Jumpi,
    PC,
    Msize,
    Gas,
    Jumpdest,
    Tload,
    Tstore,
    Mcopy,
    Push0,
    Push((u8, U256)),
    Dup(u8),
    Swap(u8),
    Log(u8),
    Create,
    Call,
    CallCode,
    Return,
    DelegateCall,
    Create2,
    StaticCall,
    Revert,
    Invalid,
    SelfDestruct,
}

impl Operation {
    pub fn to_bytecode(&self) -> Result<Bytes, VMError> {
        let bytecode = match self {
            Operation::Stop => Bytes::copy_from_slice(&[Opcode::STOP as u8]),
            Operation::Add => Bytes::copy_from_slice(&[Opcode::ADD as u8]),
            Operation::Mul => Bytes::copy_from_slice(&[Opcode::MUL as u8]),
            Operation::Sub => Bytes::copy_from_slice(&[Opcode::SUB as u8]),
            Operation::Div => Bytes::copy_from_slice(&[Opcode::DIV as u8]),
            Operation::Sdiv => Bytes::copy_from_slice(&[Opcode::SDIV as u8]),
            Operation::Mod => Bytes::copy_from_slice(&[Opcode::MOD as u8]),
            Operation::SMod => Bytes::copy_from_slice(&[Opcode::SMOD as u8]),
            Operation::Addmod => Bytes::copy_from_slice(&[Opcode::ADDMOD as u8]),
            Operation::Mulmod => Bytes::copy_from_slice(&[Opcode::MULMOD as u8]),
            Operation::Exp => Bytes::copy_from_slice(&[Opcode::EXP as u8]),
            Operation::SignExtend => Bytes::copy_from_slice(&[Opcode::SIGNEXTEND as u8]),
            Operation::Lt => Bytes::copy_from_slice(&[Opcode::LT as u8]),
            Operation::Gt => Bytes::copy_from_slice(&[Opcode::GT as u8]),
            Operation::Slt => Bytes::copy_from_slice(&[Opcode::SLT as u8]),
            Operation::Sgt => Bytes::copy_from_slice(&[Opcode::SGT as u8]),
            Operation::Eq => Bytes::copy_from_slice(&[Opcode::EQ as u8]),
            Operation::IsZero => Bytes::copy_from_slice(&[Opcode::ISZERO as u8]),
            Operation::And => Bytes::copy_from_slice(&[Opcode::AND as u8]),
            Operation::Or => Bytes::copy_from_slice(&[Opcode::OR as u8]),
            Operation::Xor => Bytes::copy_from_slice(&[Opcode::XOR as u8]),
            Operation::Not => Bytes::copy_from_slice(&[Opcode::NOT as u8]),
            Operation::Byte => Bytes::copy_from_slice(&[Opcode::BYTE as u8]),
            Operation::Shl => Bytes::copy_from_slice(&[Opcode::SHL as u8]),
            Operation::Shr => Bytes::copy_from_slice(&[Opcode::SHR as u8]),
            Operation::Sar => Bytes::copy_from_slice(&[Opcode::SAR as u8]),
            Operation::Keccak256 => Bytes::copy_from_slice(&[Opcode::KECCAK256 as u8]),
            Operation::Address => Bytes::copy_from_slice(&[Opcode::ADDRESS as u8]),
            Operation::Balance => Bytes::copy_from_slice(&[Opcode::BALANCE as u8]),
            Operation::Origin => Bytes::copy_from_slice(&[Opcode::ORIGIN as u8]),
            Operation::Caller => Bytes::copy_from_slice(&[Opcode::CALLER as u8]),
            Operation::Callvalue => Bytes::copy_from_slice(&[Opcode::CALLVALUE as u8]),
            Operation::CallDataLoad => Bytes::copy_from_slice(&[Opcode::CALLDATALOAD as u8]),
            Operation::CallDataSize => Bytes::copy_from_slice(&[Opcode::CALLDATASIZE as u8]),
            Operation::CallDataCopy => Bytes::copy_from_slice(&[Opcode::CALLDATACOPY as u8]),
            Operation::Codesize => Bytes::copy_from_slice(&[Opcode::CODESIZE as u8]),
            Operation::Codecopy => Bytes::copy_from_slice(&[Opcode::CODECOPY as u8]),
            Operation::Gasprice => Bytes::copy_from_slice(&[Opcode::GASPRICE as u8]),
            Operation::ExtcodeSize => Bytes::copy_from_slice(&[Opcode::EXTCODESIZE as u8]),
            Operation::ExtcodeCopy => Bytes::copy_from_slice(&[Opcode::EXTCODECOPY as u8]),
            Operation::ReturnDataSize => Bytes::copy_from_slice(&[Opcode::RETURNDATASIZE as u8]),
            Operation::ReturnDataCopy => Bytes::copy_from_slice(&[Opcode::RETURNDATACOPY as u8]),
            Operation::ExtcodeHash => Bytes::copy_from_slice(&[Opcode::EXTCODEHASH as u8]),
            Operation::BlockHash => Bytes::copy_from_slice(&[Opcode::BLOCKHASH as u8]),
            Operation::Coinbase => Bytes::copy_from_slice(&[Opcode::COINBASE as u8]),
            Operation::Timestamp => Bytes::copy_from_slice(&[Opcode::TIMESTAMP as u8]),
            Operation::Number => Bytes::copy_from_slice(&[Opcode::NUMBER as u8]),
            Operation::Prevrandao => Bytes::copy_from_slice(&[Opcode::PREVRANDAO as u8]),
            Operation::Gaslimit => Bytes::copy_from_slice(&[Opcode::GASLIMIT as u8]),
            Operation::Chainid => Bytes::copy_from_slice(&[Opcode::CHAINID as u8]),
            Operation::SelfBalance => Bytes::copy_from_slice(&[Opcode::SELFBALANCE as u8]),
            Operation::Basefee => Bytes::copy_from_slice(&[Opcode::BASEFEE as u8]),
            Operation::BlobHash => Bytes::copy_from_slice(&[Opcode::BLOBHASH as u8]),
            Operation::BlobBaseFee => Bytes::copy_from_slice(&[Opcode::BLOBBASEFEE as u8]),
            Operation::Pop => Bytes::copy_from_slice(&[Opcode::POP as u8]),
            Operation::Mload => Bytes::copy_from_slice(&[Opcode::MLOAD as u8]),
            Operation::Mstore => Bytes::copy_from_slice(&[Opcode::MSTORE as u8]),
            Operation::Mstore8 => Bytes::copy_from_slice(&[Opcode::MSTORE8 as u8]),
            Operation::Sload => Bytes::copy_from_slice(&[Opcode::SLOAD as u8]),
            Operation::Sstore => Bytes::copy_from_slice(&[Opcode::SSTORE as u8]),
            Operation::Jump => Bytes::copy_from_slice(&[Opcode::JUMP as u8]),
            Operation::Jumpi => Bytes::copy_from_slice(&[Opcode::JUMPI as u8]),
            Operation::PC => Bytes::copy_from_slice(&[Opcode::PC as u8]),
            Operation::Msize => Bytes::copy_from_slice(&[Opcode::MSIZE as u8]),
            Operation::Gas => Bytes::copy_from_slice(&[Opcode::GAS as u8]),
            Operation::Jumpdest => Bytes::copy_from_slice(&[Opcode::JUMPDEST as u8]),
            Operation::Tload => Bytes::copy_from_slice(&[Opcode::TLOAD as u8]),
            Operation::Tstore => Bytes::copy_from_slice(&[Opcode::TSTORE as u8]),
            Operation::Mcopy => Bytes::copy_from_slice(&[Opcode::MCOPY as u8]),
            Operation::Push0 => Bytes::copy_from_slice(&[Opcode::PUSH0 as u8]),
            Operation::Push((n, value)) => {
                assert!(*n <= 32, "PUSH32 is the max");
                // the amount of bytes needed to represent the value must
                // be less than the n in PUSHn
                assert!(
                    value.bits().div_ceil(8) <= *n as usize,
                    "value doesn't fit in n bytes"
                );
                let mut word_buffer = [0; 32];
                value.to_big_endian(&mut word_buffer);
                // extract the last n bytes to push
                let value_to_push = &word_buffer[((32 - *n) as usize)..];
                assert_eq!(value_to_push.len(), *n as usize);
                let opcode = Opcode::try_from(Opcode::PUSH0 as u8 + *n)?;
                let mut bytes = vec![opcode as u8];
                bytes.extend_from_slice(value_to_push);

                Bytes::copy_from_slice(&bytes)
            }
            Operation::Dup(n) => {
                assert!(*n <= 16, "DUP16 is the max");
                Bytes::copy_from_slice(&[Opcode::DUP1 as u8 + n - 1])
            }
            Operation::Swap(n) => {
                assert!(*n <= 16, "SWAP16 is the max");
                Bytes::copy_from_slice(&[Opcode::SWAP1 as u8 + n - 1])
            }
            Operation::Log(n) => {
                assert!(*n <= 4, "LOG4 is the max");
                Bytes::copy_from_slice(&[Opcode::LOG0 as u8 + n])
            }
            Operation::Create => Bytes::copy_from_slice(&[Opcode::CREATE as u8]),
            Operation::Call => Bytes::copy_from_slice(&[Opcode::CALL as u8]),
            Operation::CallCode => Bytes::copy_from_slice(&[Opcode::CALLCODE as u8]),
            Operation::Return => Bytes::copy_from_slice(&[Opcode::RETURN as u8]),
            Operation::DelegateCall => Bytes::copy_from_slice(&[Opcode::DELEGATECALL as u8]),
            Operation::Create2 => Bytes::copy_from_slice(&[Opcode::CREATE2 as u8]),
            Operation::StaticCall => Bytes::copy_from_slice(&[Opcode::STATICCALL as u8]),
            Operation::Revert => Bytes::copy_from_slice(&[Opcode::REVERT as u8]),
            Operation::Invalid => Bytes::copy_from_slice(&[Opcode::INVALID as u8]),
            Operation::SelfDestruct => Bytes::copy_from_slice(&[Opcode::SELFDESTRUCT as u8]),
        };
        Ok(bytecode)
    }
}
