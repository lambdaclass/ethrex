use std::collections::HashMap;

use bytes::Bytes;
use ethereum_types::Address;

use crate::{call_frame::Log, vm::Account};

/// Errors that halt the program
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VMError {
    StackUnderflow,
    StackOverflow,
    InvalidJump,
    OpcodeNotAllowedInStaticContext,
    OpcodeNotFound,
    InvalidBytecode,
    OutOfGas,
    VeryLargeNumber,
    OverflowInArithmeticOp,
    FatalError,
    InvalidTransaction,
    MissingBlobHashes,
    BlobHashIndexOutOfBounds,
    RevertOpcode,
    AddressDoesNotMatchAnAccount,
    SenderAccountShouldNotHaveBytecode,
    SenderBalanceShouldContainTransferValue,
    GasPriceIsLowerThanBaseFee,
    AddressAlreadyOccupied,
    ContractOutputTooBig,
    InvalidInitialByte,
    NonceOverflow,
}

pub enum OpcodeSuccess {
    Continue,
    Result(ResultReason),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResultReason {
    Stop,
    Revert,
    Return,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxResult {
    Success,
    Revert(VMError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionReport {
    pub result: TxResult,
    pub new_state: HashMap<Address, Account>,
    pub gas_used: u64,
    pub gas_refunded: u64,
    pub output: Bytes,
    pub logs: Vec<Log>,
    // This only applies to create transactions. It's fundamentally ambiguous since
    // a transaction could create multiple new contracts, but whatever.
    pub created_address: Option<Address>,
}
