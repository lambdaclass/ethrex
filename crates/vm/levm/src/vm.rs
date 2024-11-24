use crate::{
    account::{Account, StorageSlot},
    call_frame::CallFrame,
    constants::*,
    db::{Cache, Database},
    environment::Environment,
    errors::{
        InternalError, OpcodeSuccess, OutOfGasError, ResultReason, TransactionReport, TxResult,
        VMError,
    },
    gas_cost::CREATE_BASE_COST,
    opcodes::Opcode,
};
use bytes::Bytes;
use ethrex_core::{types::TxKind, Address, H256, U256};
use ethrex_rlp;
use ethrex_rlp::encode::RLPEncode;
use keccak_hash::keccak;
use sha3::{Digest, Keccak256};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub type Storage = HashMap<U256, H256>;

#[derive(Debug, Clone, Default)]
// TODO: https://github.com/lambdaclass/ethrex/issues/604
pub struct Substate {
    // accessed addresses and storage keys are considered WARM
    // pub accessed_addresses: HashSet<Address>,
    // pub accessed_storage_keys: HashSet<(Address, U256)>,
    pub selfdestrutct_set: HashSet<Address>,
}

pub struct VM {
    pub call_frames: Vec<CallFrame>,
    pub env: Environment,
    /// Information that is acted upon immediately following the
    /// transaction.
    pub accrued_substate: Substate,
    /// Mapping between addresses (160-bit identifiers) and account
    /// states.
    pub db: Arc<dyn Database>,
    pub cache: Cache,
    pub tx_kind: TxKind,
}

pub fn address_to_word(address: Address) -> U256 {
    // This unwrap can't panic, as Address are 20 bytes long and U256 use 32 bytes
    let mut word = [0u8; 32];

    for (word_byte, address_byte) in word.iter_mut().skip(12).zip(address.as_bytes().iter()) {
        *word_byte = *address_byte;
    }

    U256::from_big_endian(&word)
}

pub fn word_to_address(word: U256) -> Address {
    let mut bytes = [0u8; WORD_SIZE];
    word.to_big_endian(&mut bytes);
    Address::from_slice(&bytes[12..])
}

impl VM {
    // TODO: Refactor this.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        to: TxKind,
        env: Environment,
        value: U256,
        calldata: Bytes,
        db: Arc<dyn Database>,
        mut cache: Cache,
    ) -> Result<Self, VMError> {
        // Maybe this decision should be made in an upper layer

        // Add sender, coinbase and recipient (in the case of a Call) to cache [https://www.evm.codes/about#access_list]
        let sender_account_info = db.get_account_info(env.origin);
        cache.add_account(&env.origin, &Account::from(sender_account_info.clone()));

        let coinbase_account_info = db.get_account_info(env.coinbase);
        cache.add_account(&env.coinbase, &Account::from(coinbase_account_info));

        match to {
            TxKind::Call(address_to) => {
                // add address_to to cache
                let recipient_account_info = db.get_account_info(address_to);
                cache.add_account(&address_to, &Account::from(recipient_account_info.clone()));

                // CALL tx
                let initial_call_frame = CallFrame::new(
                    env.origin,
                    address_to,
                    address_to,
                    recipient_account_info.bytecode,
                    value,
                    calldata.clone(),
                    false,
                    env.gas_limit.min(MAX_BLOCK_GAS_LIMIT),
                    U256::zero(),
                    0,
                );

                Ok(Self {
                    call_frames: vec![initial_call_frame],
                    db,
                    env,
                    accrued_substate: Substate::default(),
                    cache,
                    tx_kind: to,
                })
            }
            TxKind::Create => {
                // CREATE tx

                // (2)
                let new_contract_address =
                    VM::calculate_create_address(env.origin, sender_account_info.nonce).map_err(
                        |_| VMError::Internal(InternalError::CouldNotComputeCreateAddress),
                    )?; // TODO: Remove after merging the PR that removes unwraps.

                // (3)
                let created_contract = Account::new(value, calldata.clone(), 1, HashMap::new());
                cache.add_account(&new_contract_address, &created_contract);

                // (5)
                let code: Bytes = calldata.clone();

                let initial_call_frame = CallFrame::new(
                    env.origin,
                    new_contract_address,
                    new_contract_address,
                    code,
                    value,
                    calldata.clone(),
                    false,
                    env.gas_limit.min(MAX_BLOCK_GAS_LIMIT),
                    U256::zero(),
                    0,
                );

                Ok(Self {
                    call_frames: vec![initial_call_frame],
                    db,
                    env,
                    accrued_substate: Substate::default(),
                    cache,
                    tx_kind: TxKind::Create,
                })
            }
        }
        // TODO: https://github.com/lambdaclass/lambda_ethrex/issues/1088
    }

    /// Executes opcodes in callframe
    /// Returns a TransactionReport with the result of the transaction
    /// and the new state of the accounts.
    /// It only returns an error if the error was internal, otherwise it will be a TransactionReport with revert status.
    pub fn execute(
        &mut self,
        current_call_frame: &mut CallFrame,
    ) -> Result<TransactionReport, VMError> {
        // Backup of Database, Substate and Gas Refunds if sub-context is reverted
        let (backup_db, backup_substate, backup_refunded_gas) = (
            self.cache.clone(),
            self.accrued_substate.clone(),
            self.env.refunded_gas,
        );

        loop {
            let opcode = current_call_frame.next_opcode()?.unwrap_or(Opcode::STOP); // This will execute opcode stop if there are no more opcodes, there are other ways of solving this but this is the simplest and doesn't change VM behavior.

            // Note: This is commented because it's used for debugging purposes in development.
            // dbg!(&current_call_frame.gas_used);
            // dbg!(&opcode);
            let op_result: Result<OpcodeSuccess, VMError> = match opcode {
                Opcode::STOP => Ok(OpcodeSuccess::Result(ResultReason::Stop)),
                Opcode::ADD => self.op_add(current_call_frame),
                Opcode::MUL => self.op_mul(current_call_frame),
                Opcode::SUB => self.op_sub(current_call_frame),
                Opcode::DIV => self.op_div(current_call_frame),
                Opcode::SDIV => self.op_sdiv(current_call_frame),
                Opcode::MOD => self.op_mod(current_call_frame),
                Opcode::SMOD => self.op_smod(current_call_frame),
                Opcode::ADDMOD => self.op_addmod(current_call_frame),
                Opcode::MULMOD => self.op_mulmod(current_call_frame),
                Opcode::EXP => self.op_exp(current_call_frame),
                Opcode::SIGNEXTEND => self.op_signextend(current_call_frame),
                Opcode::LT => self.op_lt(current_call_frame),
                Opcode::GT => self.op_gt(current_call_frame),
                Opcode::SLT => self.op_slt(current_call_frame),
                Opcode::SGT => self.op_sgt(current_call_frame),
                Opcode::EQ => self.op_eq(current_call_frame),
                Opcode::ISZERO => self.op_iszero(current_call_frame),
                Opcode::KECCAK256 => self.op_keccak256(current_call_frame),
                Opcode::CALLDATALOAD => self.op_calldataload(current_call_frame),
                Opcode::CALLDATASIZE => self.op_calldatasize(current_call_frame),
                Opcode::CALLDATACOPY => self.op_calldatacopy(current_call_frame),
                Opcode::RETURNDATASIZE => self.op_returndatasize(current_call_frame),
                Opcode::RETURNDATACOPY => self.op_returndatacopy(current_call_frame),
                Opcode::JUMP => self.op_jump(current_call_frame),
                Opcode::JUMPI => self.op_jumpi(current_call_frame),
                Opcode::JUMPDEST => self.op_jumpdest(current_call_frame),
                Opcode::PC => self.op_pc(current_call_frame),
                Opcode::BLOCKHASH => self.op_blockhash(current_call_frame),
                Opcode::COINBASE => self.op_coinbase(current_call_frame),
                Opcode::TIMESTAMP => self.op_timestamp(current_call_frame),
                Opcode::NUMBER => self.op_number(current_call_frame),
                Opcode::PREVRANDAO => self.op_prevrandao(current_call_frame),
                Opcode::GASLIMIT => self.op_gaslimit(current_call_frame),
                Opcode::CHAINID => self.op_chainid(current_call_frame),
                Opcode::BASEFEE => self.op_basefee(current_call_frame),
                Opcode::BLOBHASH => self.op_blobhash(current_call_frame),
                Opcode::BLOBBASEFEE => self.op_blobbasefee(current_call_frame),
                Opcode::PUSH0 => self.op_push0(current_call_frame),
                // PUSHn
                op if (Opcode::PUSH1..=Opcode::PUSH32).contains(&op) => {
                    self.op_push(current_call_frame, op)
                }
                Opcode::AND => self.op_and(current_call_frame),
                Opcode::OR => self.op_or(current_call_frame),
                Opcode::XOR => self.op_xor(current_call_frame),
                Opcode::NOT => self.op_not(current_call_frame),
                Opcode::BYTE => self.op_byte(current_call_frame),
                Opcode::SHL => self.op_shl(current_call_frame),
                Opcode::SHR => self.op_shr(current_call_frame),
                Opcode::SAR => self.op_sar(current_call_frame),
                // DUPn
                op if (Opcode::DUP1..=Opcode::DUP16).contains(&op) => {
                    self.op_dup(current_call_frame, op)
                }
                // SWAPn
                op if (Opcode::SWAP1..=Opcode::SWAP16).contains(&op) => {
                    self.op_swap(current_call_frame, op)
                }
                Opcode::POP => self.op_pop(current_call_frame),
                op if (Opcode::LOG0..=Opcode::LOG4).contains(&op) => {
                    self.op_log(current_call_frame, op)
                }
                Opcode::MLOAD => self.op_mload(current_call_frame),
                Opcode::MSTORE => self.op_mstore(current_call_frame),
                Opcode::MSTORE8 => self.op_mstore8(current_call_frame),
                Opcode::SLOAD => self.op_sload(current_call_frame),
                Opcode::SSTORE => self.op_sstore(current_call_frame),
                Opcode::MSIZE => self.op_msize(current_call_frame),
                Opcode::GAS => self.op_gas(current_call_frame),
                Opcode::MCOPY => self.op_mcopy(current_call_frame),
                Opcode::CALL => self.op_call(current_call_frame),
                Opcode::CALLCODE => self.op_callcode(current_call_frame),
                Opcode::RETURN => self.op_return(current_call_frame),
                Opcode::DELEGATECALL => self.op_delegatecall(current_call_frame),
                Opcode::STATICCALL => self.op_staticcall(current_call_frame),
                Opcode::CREATE => self.op_create(current_call_frame),
                Opcode::CREATE2 => self.op_create2(current_call_frame),
                Opcode::TLOAD => self.op_tload(current_call_frame),
                Opcode::TSTORE => self.op_tstore(current_call_frame),
                Opcode::SELFBALANCE => self.op_selfbalance(current_call_frame),
                Opcode::ADDRESS => self.op_address(current_call_frame),
                Opcode::ORIGIN => self.op_origin(current_call_frame),
                Opcode::BALANCE => self.op_balance(current_call_frame),
                Opcode::CALLER => self.op_caller(current_call_frame),
                Opcode::CALLVALUE => self.op_callvalue(current_call_frame),
                Opcode::CODECOPY => self.op_codecopy(current_call_frame),
                Opcode::CODESIZE => self.op_codesize(current_call_frame),
                Opcode::GASPRICE => self.op_gasprice(current_call_frame),
                Opcode::EXTCODESIZE => self.op_extcodesize(current_call_frame),
                Opcode::EXTCODECOPY => self.op_extcodecopy(current_call_frame),
                Opcode::EXTCODEHASH => self.op_extcodehash(current_call_frame),
                Opcode::REVERT => self.op_revert(current_call_frame),
                Opcode::INVALID => self.op_invalid(),
                Opcode::SELFDESTRUCT => self.op_selfdestruct(current_call_frame),

                _ => Err(VMError::OpcodeNotFound),
            };

            // Gas refunds are applied at the end of a transaction. Should it be implemented here?

            match op_result {
                Ok(OpcodeSuccess::Continue) => {}
                Ok(OpcodeSuccess::Result(_)) => {
                    self.call_frames.push(current_call_frame.clone());
                    return Ok(TransactionReport {
                        result: TxResult::Success,
                        new_state: self.cache.accounts.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: self.env.refunded_gas.low_u64(),
                        output: current_call_frame.returndata.clone(),
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    });
                }
                Err(error) => {
                    self.call_frames.push(current_call_frame.clone());

                    // If error is internal then propagate it to the upper layer
                    if error.is_internal() {
                        return Err(error);
                    }

                    // Unless error is from Revert opcode, all gas is consumed
                    if error != VMError::RevertOpcode {
                        let left_gas = current_call_frame
                            .gas_limit
                            .saturating_sub(current_call_frame.gas_used);
                        current_call_frame.gas_used =
                            current_call_frame.gas_used.saturating_add(left_gas);
                        self.env.consumed_gas = self.env.consumed_gas.saturating_add(left_gas);
                    }

                    self.restore_state(backup_db, backup_substate, backup_refunded_gas);

                    return Ok(TransactionReport {
                        result: TxResult::Revert(error),
                        new_state: self.cache.accounts.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: self.env.refunded_gas.low_u64(),
                        output: current_call_frame.returndata.clone(), // Bytes::new() if error is not RevertOpcode
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    });
                }
            }
        }
    }

    fn restore_state(
        &mut self,
        backup_cache: Cache,
        backup_substate: Substate,
        backup_refunded_gas: U256,
    ) {
        self.cache = backup_cache;
        self.accrued_substate = backup_substate;
        self.env.refunded_gas = backup_refunded_gas;
    }

    /// ## Description
    /// This method performs validations and returns an error if any of the validations fail.
    /// It also makes initial changes alongside the validations:
    /// - It increases sender nonce
    /// - It substracts up-front-cost from sender balance.
    /// - It calculates and adds intrinsic gas to the 'gas used' of callframe and environment.
    ///   See 'docs' for more information about validations.
    fn validate_transaction(&mut self, initial_call_frame: &mut CallFrame) -> Result<(), VMError> {
        //TODO: This should revert the transaction, not throw an error. And I don't know if it should be done here...
        // if self.is_create() {
        //     // If address is already in db, there's an error
        //     let new_address_acc = self.db.get_account_info(call_frame.to);
        //     if !new_address_acc.is_empty() {
        //         return Err(VMError::AddressAlreadyOccupied);
        //     }
        // }
        let sender_address = self.env.origin;
        let mut sender_account = self.get_account(&sender_address);

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        let gaslimit_price_product = self
            .gas_price_or_max_fee_per_gas()
            .checked_mul(self.env.gas_limit)
            .ok_or(VMError::GasLimitPriceProductOverflow)?;

        // Up front cost is the maximum amount of wei that a user is willing to pay for.
        let up_front_cost = gaslimit_price_product
            .checked_add(initial_call_frame.msg_value)
            .ok_or(VMError::InsufficientAccountFunds)?;

        // (2) INSUFFICIENT_ACCOUNT_FUNDS
        sender_account.info.balance = sender_account
            .info
            .balance
            .checked_sub(up_front_cost)
            .ok_or(VMError::InsufficientAccountFunds)?;

        // (3) INSUFFICIENT_MAX_FEE_PER_GAS
        if self.gas_price_or_max_fee_per_gas() < self.env.base_fee_per_gas {
            return Err(VMError::InsufficientMaxFeePerGas);
        }

        // (4) INITCODE_SIZE_EXCEEDED
        if self.is_create() {
            // INITCODE_SIZE_EXCEEDED
            if initial_call_frame.calldata.len() > INIT_CODE_MAX_SIZE {
                return Err(VMError::InitcodeSizeExceeded);
            }
        }

        // (5) INTRINSIC_GAS_TOO_LOW
        self.add_intrinsic_gas(initial_call_frame)?;

        // (6) NONCE_IS_MAX
        sender_account.info.nonce = sender_account
            .info
            .nonce
            .checked_add(1)
            .ok_or(VMError::NonceIsMax)?;

        // Update cache with account with incremented nonce
        self.cache.add_account(&sender_address, &sender_account);

        // (7) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
            self.env.tx_max_priority_fee_per_gas,
            self.env.tx_max_fee_per_gas,
        ) {
            if tx_max_priority_fee > tx_max_fee_per_gas {
                return Err(VMError::PriorityGreaterThanMaxFeePerGas);
            }
        }

        // (8) SENDER_NOT_EOA
        if sender_account.has_code() {
            return Err(VMError::SenderNotEOA);
        }

        // (9) GAS_ALLOWANCE_EXCEEDED
        if self.env.gas_limit > self.env.block_gas_limit {
            return Err(VMError::GasAllowanceExceeded);
        }

        // (10) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = self.env.tx_max_fee_per_blob_gas {
            if tx_max_fee_per_blob_gas < self.env.base_fee_per_gas {
                return Err(VMError::InsufficientMaxFeePerGas);
            }
        }

        //TODO: Implement the rest of the validations (TYPE_3)

        // (11) TYPE_3_TX_ZERO_BLOBS
        if let Some(tx_blob_hashes) = &self.env.tx_blob_hashes {
            if tx_blob_hashes.is_empty() {
                return Err(VMError::Type3TxZeroBlobs);
            }
        }

        // (12) TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH
        if let Some(tx_blob_hashes) = &self.env.tx_blob_hashes {
            for blob_hash in tx_blob_hashes {
                let blob_hash = blob_hash.as_bytes();
                if let Some(first_byte) = blob_hash.first() {
                    if !VALID_BLOB_PREFIXES.contains(first_byte) {
                        return Err(VMError::Type3TxInvalidBlobVersionedHash);
                    }
                }
            }
        }

        // (13) TYPE_3_TX_PRE_FORK -> This is not necessary for now because we are not supporting pre-cancun transactions yet. But we should somehow be able to tell the current context.

        // (14) TYPE_3_TX_BLOB_COUNT_EXCEEDED
        if let Some(tx_blob_hashes) = &self.env.tx_blob_hashes {
            if tx_blob_hashes.len() > MAX_BLOB_COUNT {
                return Err(VMError::Type3TxBlobCountExceeded);
            }
        }

        // (15) TYPE_3_TX_CONTRACT_CREATION
        // The current way of checking if the transaction is a Type 3 Tx is by checking if the tx_blob_hashes is Some.
        if self.env.tx_blob_hashes.is_some() && self.is_create() {
            return Err(VMError::Type3TxContractCreation);
        }

        Ok(())
    }

    fn is_create(&self) -> bool {
        matches!(self.tx_kind, TxKind::Create)
    }

    // Maybe this function is not necessary.
    // fn revert_create(&mut self) -> Result<(), VMError> {
    //     // Note: currently working with copies
    //     let call_frame = self
    //         .call_frames
    //         .last()
    //         .ok_or(VMError::Internal(
    //             InternalError::CouldNotAccessLastCallframe,
    //         ))?
    //         .clone();

    //     let sender = call_frame.msg_sender;
    //     let mut sender_account = self.get_account(&sender);

    //     sender_account.info.nonce = sender_account
    //         .info
    //         .nonce
    //         .checked_sub(1)
    //         .ok_or(VMError::Internal(InternalError::NonceUnderflowed))?;

    //     let new_contract_address = call_frame.to;

    //     if self.cache.accounts.remove(&new_contract_address).is_none() {
    //         return Err(VMError::AddressDoesNotMatchAnAccount); // Should not be this error
    //     }

    //     // Should revert this?
    //     // sender_account.info.balance -= self.call_frames.first().ok_or(VMError::FatalUnwrap)?.msg_value;

    //     Ok(())
    // }

    /// Calculates effective gas price and updates variable gas_price in env.
    pub fn update_effective_gas_price(&mut self) -> Result<(), VMError> {
        if let (Some(max_priority_fee_per_gas), Some(max_fee_per_gas)) = (
            self.env.tx_max_priority_fee_per_gas,
            self.env.tx_max_fee_per_gas,
        ) {
            // Effective Gas Price = Base Fee + min(maxPriorityFeePerGas, maxFeePerGas - Base Fee)
            self.env.gas_price = self
                .env
                .base_fee_per_gas
                .checked_add(
                    max_priority_fee_per_gas.min(
                        max_fee_per_gas
                            .checked_sub(self.env.base_fee_per_gas)
                            .ok_or(VMError::InsufficientMaxFeePerGas)?,
                    ),
                )
                .ok_or(VMError::Internal(
                    InternalError::ArithmeticOperationOverflow,
                ))?;
            // TODO: See what to return if base fee + priority fee causes overflow.
        };
        Ok(())
    }

    pub fn create_post_execution(
        &mut self,
        initial_call_frame: &mut CallFrame,
        report: &mut TransactionReport,
    ) -> Result<(), VMError> {
        if self.is_create() {
            if let TxResult::Revert(error) = &report.result {
                return Err(error.clone());
            }

            let contract_code = report.clone().output;

            // (6)
            if contract_code.len() > MAX_CODE_SIZE {
                return Err(VMError::ContractOutputTooBig);
            }

            // If contract code is not empty then the first byte should not be 0xef
            if *contract_code.first().unwrap_or(&0) == INVALID_CONTRACT_PREFIX {
                return Err(VMError::InvalidInitialByte);
            }

            let max_gas = self.env.gas_limit.low_u64();

            // If initialization code is successful, code-deposit cost is paid.
            let code_length: u64 = contract_code
                .len()
                .try_into()
                .map_err(|_| VMError::Internal(InternalError::ConversionError))?;
            let code_deposit_cost = code_length
                .checked_mul(200)
                .ok_or(VMError::Internal(InternalError::OperationOverflow))?;

            report.add_gas_with_max(code_deposit_cost, max_gas)?;
            // Charge 22100 gas for each storage variable set (???)

            // Assign bytecode to the new contract
            let contract_address = initial_call_frame.to;
            let mut created_contract = self.get_account(&contract_address);

            created_contract.info.bytecode = contract_code;

            self.cache.add_account(&contract_address, &created_contract);
        }
        Ok(())
    }

    /// ## Description
    /// Changes to the state post-execution, if this fails we won't be able to determine a final state.
    /// Errors here are mostly related to balance overflows.
    /// TODO: Determine the right behavior when a balance overflow occurs.
    /// 1. Refund unused gas to sender
    /// 2. Transfer value to recipient or return value to sender depending on execution result.
    /// 3. Pay priority fee to coinbase
    pub fn post_execution_changes(
        &mut self,
        initial_call_frame: &CallFrame,
        report: &TransactionReport,
    ) -> Result<(), VMError> {
        // 1. Refund unused gas to sender
        let sender_address = self.env.origin;
        let mut sender_account = self.get_account(&sender_address);

        let max_gas = self.env.gas_limit.low_u64();
        let consumed_gas = report.gas_used;
        let refunded_gas = report.gas_refunded.min(
            consumed_gas
                .checked_div(5)
                .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
        );
        // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"

        let gas_to_return = max_gas
            .checked_sub(consumed_gas)
            .and_then(|gas| gas.checked_add(refunded_gas))
            .ok_or(VMError::Internal(InternalError::UndefinedState(0)))?;

        let gas_return_amount = self
            .env
            .gas_price
            .checked_mul(U256::from(gas_to_return))
            .ok_or(VMError::Internal(InternalError::UndefinedState(1)))?;

        sender_account.info.balance = sender_account
            .info
            .balance
            .checked_add(gas_return_amount)
            .ok_or(VMError::Internal(InternalError::UndefinedState(2)))?;

        // 2. Transfer value to recipient or return value to sender depending on execution result.
        let recipient_address = initial_call_frame.to;
        let mut recipient_account = self.get_account(&recipient_address);

        if report.is_success() {
            // transfer value to recipient
            recipient_account.info.balance = recipient_account
                .info
                .balance
                .checked_add(initial_call_frame.msg_value)
                .ok_or(VMError::Internal(InternalError::UndefinedState(3)))?;
        } else {
            // return value to sender
            sender_account.info.balance = sender_account
                .info
                .balance
                .checked_add(initial_call_frame.msg_value)
                .ok_or(VMError::Internal(InternalError::UndefinedState(4)))?;
        }

        self.cache.add_account(&sender_address, &sender_account);
        self.cache
            .add_account(&recipient_address, &recipient_account);

        // 3. Pay priority fee to coinbase
        let coinbase_address = self.env.coinbase;
        let priority_fee_per_gas = self
            .env
            .gas_price
            .checked_sub(self.env.base_fee_per_gas)
            .ok_or(VMError::Internal(InternalError::UndefinedState(5)))?;
        let coinbase_fee = (U256::from(consumed_gas))
            .checked_mul(priority_fee_per_gas)
            .ok_or(VMError::Internal(InternalError::UndefinedState(6)))?;

        let mut coinbase_account = self.get_account(&coinbase_address);

        coinbase_account.info.balance = coinbase_account
            .info
            .balance
            .checked_add(coinbase_fee)
            .ok_or(VMError::Internal(InternalError::UndefinedState(7)))?;

        self.cache.add_account(&coinbase_address, &coinbase_account);

        Ok(())
    }

    pub fn add_intrinsic_gas(&mut self, initial_call_frame: &mut CallFrame) -> Result<(), VMError> {
        // Intrinsic gas is the gas consumed by the transaction before the execution of the opcodes. Section 6.2 in the Yellow Paper.

        // Intrinsic Gas = Calldata cost + Create cost + Base cost + Access list cost
        let mut intrinsic_gas: U256 = U256::zero();

        // Calldata Cost
        // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
        let mut calldata_cost: U256 = U256::zero();
        for byte in &initial_call_frame.calldata {
            if *byte != 0 {
                calldata_cost = calldata_cost
                    .checked_add(U256::from(16))
                    .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
            } else {
                calldata_cost = calldata_cost
                    .checked_add(U256::from(4))
                    .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
            }
        }

        intrinsic_gas = intrinsic_gas
            .checked_add(calldata_cost)
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;

        // Base Cost
        intrinsic_gas = intrinsic_gas
            .checked_add(TX_BASE_COST)
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;

        // Create Cost
        if self.is_create() {
            intrinsic_gas = intrinsic_gas
                .checked_add(CREATE_BASE_COST)
                .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;

            let number_of_words: u64 = initial_call_frame
                .calldata
                .chunks(WORD_SIZE)
                .len()
                .try_into()
                .map_err(|_| VMError::Internal(InternalError::ConversionError))?;

            intrinsic_gas = intrinsic_gas
                .checked_add(
                    U256::from(number_of_words)
                        .checked_mul(U256::from(2))
                        .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?,
                )
                .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
        }

        // Access List Cost
        // TODO: Implement access list cost.

        self.increase_consumed_gas(initial_call_frame, intrinsic_gas)
            .map_err(|_| VMError::IntrinsicGasTooLow)?;

        Ok(())
    }

    pub fn transact(&mut self) -> Result<TransactionReport, VMError> {
        let mut initial_call_frame = self
            .call_frames
            .pop()
            .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;

        // Validates transaction and performs pre-exeuction changes.
        // Errors don't revert execution, they are propagated because transaction is invalid.
        self.validate_transaction(&mut initial_call_frame)?;

        self.update_effective_gas_price()?; // This is for gas_price to be a reasonable value

        // We need cache before execution because if a create transaction reverts in post-execution then we need to revert whatever the init-code did.
        let cache_before_execution = self.cache.clone();
        let mut report = self.execute(&mut initial_call_frame)?;

        // Validation of create transaction post execution
        // If it fails, it reverts contract creation.
        match self.create_post_execution(&mut initial_call_frame, &mut report) {
            Ok(_) => {}
            Err(error) => {
                if error.is_internal() {
                    return Err(error);
                } else {
                    report.result = TxResult::Revert(error);
                    report.gas_refunded = 0;
                    report.gas_used = self.env.gas_limit.low_u64();
                    self.cache = cache_before_execution;
                    self.cache.accounts.remove(&initial_call_frame.to);
                }
            }
        };

        self.post_execution_changes(&initial_call_frame, &report)?;

        report.new_state.clone_from(&self.cache.accounts);

        Ok(report)
    }

    pub fn current_call_frame_mut(&mut self) -> Result<&mut CallFrame, VMError> {
        self.call_frames.last_mut().ok_or(VMError::Internal(
            InternalError::CouldNotAccessLastCallframe,
        ))
    }

    pub fn gas_price_or_max_fee_per_gas(&self) -> U256 {
        self.env.tx_max_fee_per_gas.unwrap_or(self.env.gas_price)
    }

    // TODO: Improve and test REVERT behavior for XCALL opcodes. Issue: https://github.com/lambdaclass/lambda_ethereum_rust/issues/1061
    #[allow(clippy::too_many_arguments)]
    pub fn generic_call(
        &mut self,
        current_call_frame: &mut CallFrame,
        gas_limit: U256,
        value: U256,
        msg_sender: Address,
        to: Address,
        code_address: Address,
        _should_transfer_value: bool,
        is_static: bool,
        args_offset: usize,
        args_size: usize,
        ret_offset: usize,
        ret_size: usize,
    ) -> Result<OpcodeSuccess, VMError> {
        let mut sender_account = self.get_account(&current_call_frame.msg_sender);

        if sender_account.info.balance < value {
            current_call_frame.stack.push(U256::from(REVERT_FOR_CALL))?;
            return Ok(OpcodeSuccess::Continue);
        }

        let mut recipient_account = self.get_account(&to);

        // transfer value
        sender_account.info.balance = sender_account
            .info
            .balance
            .checked_sub(value)
            .ok_or(VMError::BalanceUnderflow)?;
        recipient_account.info.balance = recipient_account
            .info
            .balance
            .checked_add(value)
            .ok_or(VMError::BalanceOverflow)?;

        let code_address_bytecode = self.get_account(&code_address).info.bytecode;

        if code_address_bytecode.is_empty() {
            current_call_frame
                .stack
                .push(U256::from(SUCCESS_FOR_CALL))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Stop));
        }

        // self.cache.increment_account_nonce(&code_address); // Internal call doesn't increment account nonce.

        let calldata = current_call_frame
            .memory
            .load_range(args_offset, args_size)?
            .into();

        // I don't know if this gas limit should be calculated before or after consuming gas
        let mut potential_remaining_gas = current_call_frame
            .gas_limit
            .checked_sub(current_call_frame.gas_used)
            .ok_or(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))?;
        potential_remaining_gas = potential_remaining_gas
            .checked_sub(potential_remaining_gas.checked_div(64.into()).ok_or(
                VMError::Internal(InternalError::ArithmeticOperationOverflow),
            )?)
            .ok_or(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))?;
        let gas_limit = std::cmp::min(gas_limit, potential_remaining_gas);

        let new_depth = current_call_frame
            .depth
            .checked_add(1)
            .ok_or(VMError::StackOverflow)?; // Maybe could be depthOverflow but in concept is quite similar

        let mut new_call_frame = CallFrame::new(
            msg_sender,
            to,
            code_address,
            code_address_bytecode,
            value,
            calldata,
            is_static,
            gas_limit,
            U256::zero(),
            new_depth,
        );

        // TODO: Increase this to 1024
        if new_call_frame.depth > 10 {
            current_call_frame.stack.push(U256::from(REVERT_FOR_CALL))?;
            // return Ok(OpcodeSuccess::Result(ResultReason::Revert));
            return Err(VMError::StackOverflow); // This is wrong but it is for testing purposes.
        }

        current_call_frame.sub_return_data_offset = ret_offset;
        current_call_frame.sub_return_data_size = ret_size;

        // Update sender account and recipient in cache
        self.cache
            .add_account(&current_call_frame.msg_sender, &sender_account);
        self.cache.add_account(&to, &recipient_account);

        // self.call_frames.push(new_call_frame.clone());
        let tx_report = self.execute(&mut new_call_frame)?;

        // Add gas used by the sub-context to the current one after it's execution.
        current_call_frame.gas_used = current_call_frame
            .gas_used
            .checked_add(tx_report.gas_used.into())
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
        current_call_frame.logs.extend(tx_report.logs);
        current_call_frame
            .memory
            .store_n_bytes(ret_offset, &tx_report.output, ret_size)?;
        current_call_frame.sub_return_data = tx_report.output;

        // What to do, depending on TxResult
        match tx_report.result {
            TxResult::Success => {
                current_call_frame
                    .stack
                    .push(U256::from(SUCCESS_FOR_CALL))?;
            }
            TxResult::Revert(_) => {
                // Push 0 to stack
                current_call_frame.stack.push(U256::from(REVERT_FOR_CALL))?;
            }
        }

        Ok(OpcodeSuccess::Continue)
    }

    /// Calculates the address of a new conctract using the CREATE opcode as follow
    ///
    /// address = keccak256(rlp([sender_address,sender_nonce]))[12:]
    pub fn calculate_create_address(
        sender_address: Address,
        sender_nonce: u64,
    ) -> Result<Address, VMError> {
        let mut encoded = Vec::new();
        (sender_address, sender_nonce).encode(&mut encoded);
        let mut hasher = Keccak256::new();
        hasher.update(encoded);
        Ok(Address::from_slice(hasher.finalize().get(12..).ok_or(
            VMError::Internal(InternalError::CouldNotComputeCreateAddress),
        )?))
    }

    /// Calculates the address of a new contract using the CREATE2 opcode as follow
    ///
    /// initialization_code = memory[offset:offset+size]
    ///
    /// address = keccak256(0xff + sender_address + salt + keccak256(initialization_code))[12:]
    ///
    pub fn calculate_create2_address(
        sender_address: Address,
        initialization_code: &Bytes,
        salt: U256,
    ) -> Result<Address, VMError> {
        let init_code_hash = keccak(initialization_code);
        let mut salt_bytes = [0; 32];
        salt.to_big_endian(&mut salt_bytes);

        let generated_address = Address::from_slice(
            keccak(
                [
                    &[0xff],
                    sender_address.as_bytes(),
                    &salt_bytes,
                    init_code_hash.as_bytes(),
                ]
                .concat(),
            )
            .as_bytes()
            .get(12..)
            .ok_or(VMError::Internal(
                InternalError::CouldNotComputeCreate2Address,
            ))?,
        );
        Ok(generated_address)
    }

    /// Common behavior for CREATE and CREATE2 opcodes
    ///
    /// Could be used for CREATE type transactions
    // TODO: Improve and test REVERT behavior for CREATE. Issue: https://github.com/lambdaclass/lambda_ethrex/issues/1061
    pub fn create(
        &mut self,
        value_in_wei_to_send: U256,
        code_offset_in_memory: U256,
        code_size_in_memory: U256,
        salt: Option<U256>,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
        let code_size_in_memory = code_size_in_memory
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        if code_size_in_memory > MAX_CODE_SIZE * 2 {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        }
        if current_call_frame.is_static {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        }

        if !self.cache.is_account_cached(&current_call_frame.msg_sender) {
            self.cache_from_db(&current_call_frame.msg_sender);
        };

        let sender_account = self
            .cache
            .get_mut_account(current_call_frame.msg_sender)
            .ok_or(VMError::Internal(InternalError::AccountNotFound))?;

        if sender_account.info.balance < value_in_wei_to_send {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        }

        let Some(new_nonce) = sender_account.info.nonce.checked_add(1) else {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        };
        sender_account.info.nonce = new_nonce;

        let code_offset_in_memory = code_offset_in_memory
            .try_into()
            .map_err(|_err| VMError::VeryLargeNumber)?;

        let code = Bytes::from(
            current_call_frame
                .memory
                .load_range(code_offset_in_memory, code_size_in_memory)?,
        );

        let new_address = match salt {
            Some(salt) => Self::calculate_create2_address(current_call_frame.to, &code, salt)?,
            None => Self::calculate_create_address(
                current_call_frame.msg_sender,
                sender_account.info.nonce,
            )?,
        };

        if self.cache.accounts.contains_key(&new_address) {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        }

        let new_account = Account::new(U256::zero(), code.clone(), 0, Default::default());
        self.cache.add_account(&new_address, &new_account);

        current_call_frame
            .stack
            .push(address_to_word(new_address))?;

        self.generic_call(
            current_call_frame,
            U256::MAX,
            value_in_wei_to_send,
            current_call_frame.msg_sender,
            new_address,
            new_address,
            true,
            false,
            code_offset_in_memory,
            code_size_in_memory,
            code_offset_in_memory,
            code_size_in_memory,
        )?;

        // Erases the success value in the stack result of calling generic call, probably this should be refactored soon...
        current_call_frame
            .stack
            .pop()
            .map_err(|_| VMError::StackUnderflow)?;

        Ok(OpcodeSuccess::Continue)
    }

    /// Increases gas consumption of CallFrame and Environment, returning an error if the callframe gas limit is reached.
    pub fn increase_consumed_gas(
        &mut self,
        current_call_frame: &mut CallFrame,
        gas: U256,
    ) -> Result<(), VMError> {
        let potential_consumed_gas = current_call_frame
            .gas_used
            .checked_add(gas)
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;
        if potential_consumed_gas > current_call_frame.gas_limit {
            return Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded));
        }

        current_call_frame.gas_used = potential_consumed_gas;
        self.env.consumed_gas = self
            .env
            .consumed_gas
            .checked_add(gas)
            .ok_or(VMError::OutOfGas(OutOfGasError::ConsumedGasOverflow))?;

        Ok(())
    }

    pub fn cache_from_db(&mut self, address: &Address) {
        let acc_info = self.db.get_account_info(*address);
        self.cache.add_account(
            address,
            &Account {
                info: acc_info.clone(),
                storage: HashMap::new(),
            },
        );
    }

    /// Gets account, first checking the cache and then the database (caching in the second case)
    pub fn get_account(&mut self, address: &Address) -> Account {
        match self.cache.get_account(*address) {
            Some(acc) => acc.clone(),
            None => {
                let acc_info = self.db.get_account_info(*address);
                let acc = Account {
                    info: acc_info,
                    storage: HashMap::new(),
                };
                self.cache.add_account(address, &acc);
                acc
            }
        }
    }

    /// Gets storage slot, first checking the cache and then the database (caching in the second case)
    pub fn get_storage_slot(&mut self, address: &Address, key: H256) -> StorageSlot {
        match self.cache.get_storage_slot(*address, key) {
            Some(slot) => slot,
            None => {
                let value = self.db.get_storage_slot(*address, key);
                let slot = StorageSlot {
                    original_value: value,
                    current_value: value,
                };
                let mut acc = self.get_account(address);
                acc.storage.insert(key, slot.clone());
                self.cache.add_account(address, &acc);
                slot
            }
        }
    }
}
