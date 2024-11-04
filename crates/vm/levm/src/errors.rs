use crate::account::Account;
use bytes::Bytes;
use ethereum_rust_core::{types::Log, Address};
use std::collections::HashMap;

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
    RevertOpcode,
    InvalidOpcode,
    MissingBlobHashes,
    BlobHashIndexOutOfBounds,
    SenderAccountDoesNotExist,
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
    SelfDestruct,
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
