use crate::{
    errors::{InternalError, VMError},
    opcodes::Opcode,
};
use bytes::Bytes;
use ethrex_core::U256;

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
            Operation::Stop => Bytes::copy_from_slice(&[u8::from(Opcode::STOP)]),
            Operation::Add => Bytes::copy_from_slice(&[u8::from(Opcode::ADD)]),
            Operation::Mul => Bytes::copy_from_slice(&[u8::from(Opcode::MUL)]),
            Operation::Sub => Bytes::copy_from_slice(&[u8::from(Opcode::SUB)]),
            Operation::Div => Bytes::copy_from_slice(&[u8::from(Opcode::DIV)]),
            Operation::Sdiv => Bytes::copy_from_slice(&[u8::from(Opcode::SDIV)]),
            Operation::Mod => Bytes::copy_from_slice(&[u8::from(Opcode::MOD)]),
            Operation::SMod => Bytes::copy_from_slice(&[u8::from(Opcode::SMOD)]),
            Operation::Addmod => Bytes::copy_from_slice(&[u8::from(Opcode::ADDMOD)]),
            Operation::Mulmod => Bytes::copy_from_slice(&[u8::from(Opcode::MULMOD)]),
            Operation::Exp => Bytes::copy_from_slice(&[u8::from(Opcode::EXP)]),
            Operation::SignExtend => Bytes::copy_from_slice(&[u8::from(Opcode::SIGNEXTEND)]),
            Operation::Lt => Bytes::copy_from_slice(&[u8::from(Opcode::LT)]),
            Operation::Gt => Bytes::copy_from_slice(&[u8::from(Opcode::GT)]),
            Operation::Slt => Bytes::copy_from_slice(&[u8::from(Opcode::SLT)]),
            Operation::Sgt => Bytes::copy_from_slice(&[u8::from(Opcode::SGT)]),
            Operation::Eq => Bytes::copy_from_slice(&[u8::from(Opcode::EQ)]),
            Operation::IsZero => Bytes::copy_from_slice(&[u8::from(Opcode::ISZERO)]),
            Operation::And => Bytes::copy_from_slice(&[u8::from(Opcode::AND)]),
            Operation::Or => Bytes::copy_from_slice(&[u8::from(Opcode::OR)]),
            Operation::Xor => Bytes::copy_from_slice(&[u8::from(Opcode::XOR)]),
            Operation::Not => Bytes::copy_from_slice(&[u8::from(Opcode::NOT)]),
            Operation::Byte => Bytes::copy_from_slice(&[u8::from(Opcode::BYTE)]),
            Operation::Shl => Bytes::copy_from_slice(&[u8::from(Opcode::SHL)]),
            Operation::Shr => Bytes::copy_from_slice(&[u8::from(Opcode::SHR)]),
            Operation::Sar => Bytes::copy_from_slice(&[u8::from(Opcode::SAR)]),
            Operation::Keccak256 => Bytes::copy_from_slice(&[u8::from(Opcode::KECCAK256)]),
            Operation::Address => Bytes::copy_from_slice(&[u8::from(Opcode::ADDRESS)]),
            Operation::Balance => Bytes::copy_from_slice(&[u8::from(Opcode::BALANCE)]),
            Operation::Origin => Bytes::copy_from_slice(&[u8::from(Opcode::ORIGIN)]),
            Operation::Caller => Bytes::copy_from_slice(&[u8::from(Opcode::CALLER)]),
            Operation::Callvalue => Bytes::copy_from_slice(&[u8::from(Opcode::CALLVALUE)]),
            Operation::CallDataLoad => Bytes::copy_from_slice(&[u8::from(Opcode::CALLDATALOAD)]),
            Operation::CallDataSize => Bytes::copy_from_slice(&[u8::from(Opcode::CALLDATASIZE)]),
            Operation::CallDataCopy => Bytes::copy_from_slice(&[u8::from(Opcode::CALLDATACOPY)]),
            Operation::Codesize => Bytes::copy_from_slice(&[u8::from(Opcode::CODESIZE)]),
            Operation::Codecopy => Bytes::copy_from_slice(&[u8::from(Opcode::CODECOPY)]),
            Operation::Gasprice => Bytes::copy_from_slice(&[u8::from(Opcode::GASPRICE)]),
            Operation::ExtcodeSize => Bytes::copy_from_slice(&[u8::from(Opcode::EXTCODESIZE)]),
            Operation::ExtcodeCopy => Bytes::copy_from_slice(&[u8::from(Opcode::EXTCODECOPY)]),
            Operation::ReturnDataSize => {
                Bytes::copy_from_slice(&[u8::from(Opcode::RETURNDATASIZE)])
            }
            Operation::ReturnDataCopy => {
                Bytes::copy_from_slice(&[u8::from(Opcode::RETURNDATACOPY)])
            }
            Operation::ExtcodeHash => Bytes::copy_from_slice(&[u8::from(Opcode::EXTCODEHASH)]),
            Operation::BlockHash => Bytes::copy_from_slice(&[u8::from(Opcode::BLOCKHASH)]),
            Operation::Coinbase => Bytes::copy_from_slice(&[u8::from(Opcode::COINBASE)]),
            Operation::Timestamp => Bytes::copy_from_slice(&[u8::from(Opcode::TIMESTAMP)]),
            Operation::Number => Bytes::copy_from_slice(&[u8::from(Opcode::NUMBER)]),
            Operation::Prevrandao => Bytes::copy_from_slice(&[u8::from(Opcode::PREVRANDAO)]),
            Operation::Gaslimit => Bytes::copy_from_slice(&[u8::from(Opcode::GASLIMIT)]),
            Operation::Chainid => Bytes::copy_from_slice(&[u8::from(Opcode::CHAINID)]),
            Operation::SelfBalance => Bytes::copy_from_slice(&[u8::from(Opcode::SELFBALANCE)]),
            Operation::Basefee => Bytes::copy_from_slice(&[u8::from(Opcode::BASEFEE)]),
            Operation::BlobHash => Bytes::copy_from_slice(&[u8::from(Opcode::BLOBHASH)]),
            Operation::BlobBaseFee => Bytes::copy_from_slice(&[u8::from(Opcode::BLOBBASEFEE)]),
            Operation::Pop => Bytes::copy_from_slice(&[u8::from(Opcode::POP)]),
            Operation::Mload => Bytes::copy_from_slice(&[u8::from(Opcode::MLOAD)]),
            Operation::Mstore => Bytes::copy_from_slice(&[u8::from(Opcode::MSTORE)]),
            Operation::Mstore8 => Bytes::copy_from_slice(&[u8::from(Opcode::MSTORE8)]),
            Operation::Sload => Bytes::copy_from_slice(&[u8::from(Opcode::SLOAD)]),
            Operation::Sstore => Bytes::copy_from_slice(&[u8::from(Opcode::SSTORE)]),
            Operation::Jump => Bytes::copy_from_slice(&[u8::from(Opcode::JUMP)]),
            Operation::Jumpi => Bytes::copy_from_slice(&[u8::from(Opcode::JUMPI)]),
            Operation::PC => Bytes::copy_from_slice(&[u8::from(Opcode::PC)]),
            Operation::Msize => Bytes::copy_from_slice(&[u8::from(Opcode::MSIZE)]),
            Operation::Gas => Bytes::copy_from_slice(&[u8::from(Opcode::GAS)]),
            Operation::Jumpdest => Bytes::copy_from_slice(&[u8::from(Opcode::JUMPDEST)]),
            Operation::Tload => Bytes::copy_from_slice(&[u8::from(Opcode::TLOAD)]),
            Operation::Tstore => Bytes::copy_from_slice(&[u8::from(Opcode::TSTORE)]),
            Operation::Mcopy => Bytes::copy_from_slice(&[u8::from(Opcode::MCOPY)]),
            Operation::Push0 => Bytes::copy_from_slice(&[u8::from(Opcode::PUSH0)]),
            Operation::Push((n, value)) => {
                let n_usize: usize = (*n).into();
                assert!(*n <= 32, "PUSH32 is the max");
                assert!(
                    value.bits().div_ceil(8) <= n_usize,
                    "value doesn't fit in n bytes"
                );
                let mut word_buffer = [0; 32];
                value.to_big_endian(&mut word_buffer);
                // extract the last n bytes to push
                let value_to_push = &word_buffer
                    .get(
                        (32_usize.checked_sub(n_usize).ok_or(VMError::Internal(
                            InternalError::ArithmeticOperationUnderflow,
                        ))?)..,
                    )
                    .ok_or(VMError::Internal(InternalError::SlicingError))?;
                assert_eq!(value_to_push.len(), n_usize);
                let opcode = Opcode::try_from((u8::from(Opcode::PUSH0)).checked_add(*n).ok_or(
                    VMError::Internal(InternalError::ArithmeticOperationOverflow),
                )?)?;
                let mut bytes = vec![u8::from(opcode)];
                bytes.extend_from_slice(value_to_push);

                Bytes::copy_from_slice(&bytes)
            }
            Operation::Dup(n) => {
                assert!(*n >= 1, "DUP1 is the min");
                assert!(*n <= 16, "DUP16 is the max");
                Bytes::copy_from_slice(&[(u8::from(Opcode::DUP1))
                    .checked_add(*n)
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow,
                    ))?
                    .checked_sub(1)
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationUnderflow,
                    ))?])
            }
            Operation::Swap(n) => {
                assert!(*n >= 1, "SWAP1 is the min");
                assert!(*n <= 16, "SWAP16 is the max");
                Bytes::copy_from_slice(&[(u8::from(Opcode::SWAP1))
                    .checked_add(*n)
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationOverflow,
                    ))?
                    .checked_sub(1)
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationUnderflow,
                    ))?])
            }
            Operation::Log(n) => {
                assert!(*n <= 4, "LOG4 is the max");
                Bytes::copy_from_slice(&[(u8::from(Opcode::LOG0)).checked_add(*n).ok_or(
                    VMError::Internal(InternalError::ArithmeticOperationOverflow),
                )?])
            }
            Operation::Create => Bytes::copy_from_slice(&[u8::from(Opcode::CREATE)]),
            Operation::Call => Bytes::copy_from_slice(&[u8::from(Opcode::CALL)]),
            Operation::CallCode => Bytes::copy_from_slice(&[u8::from(Opcode::CALLCODE)]),
            Operation::Return => Bytes::copy_from_slice(&[u8::from(Opcode::RETURN)]),
            Operation::DelegateCall => Bytes::copy_from_slice(&[u8::from(Opcode::DELEGATECALL)]),
            Operation::Create2 => Bytes::copy_from_slice(&[u8::from(Opcode::CREATE2)]),
            Operation::StaticCall => Bytes::copy_from_slice(&[u8::from(Opcode::STATICCALL)]),
            Operation::Revert => Bytes::copy_from_slice(&[u8::from(Opcode::REVERT)]),
            Operation::Invalid => Bytes::copy_from_slice(&[u8::from(Opcode::INVALID)]),
            Operation::SelfDestruct => Bytes::copy_from_slice(&[u8::from(Opcode::SELFDESTRUCT)]),
        };
        Ok(bytecode)
    }
}
