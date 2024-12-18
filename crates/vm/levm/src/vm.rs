use crate::{
    account::{Account, StorageSlot},
    call_frame::CallFrame,
    constants::*,
    db::{
        cache::{self, remove_account},
        CacheDB, Database,
    },
    environment::Environment,
    errors::{
        InternalError, OpcodeSuccess, OutOfGasError, ResultReason, TransactionReport, TxResult,
        TxValidationError, VMError,
    },
    gas_cost::{
        self, fake_exponential, ACCESS_LIST_ADDRESS_COST, ACCESS_LIST_STORAGE_KEY_COST,
        BLOB_GAS_PER_BLOB, CODE_DEPOSIT_COST, CREATE_BASE_COST,
    },
    opcodes::Opcode,
    precompiles::{execute_precompile, is_precompile},
    AccountInfo,
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
    pub touched_accounts: HashSet<Address>,
    pub touched_storage_slots: HashMap<Address, HashSet<H256>>,
    pub created_accounts: HashSet<Address>,
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
    pub cache: CacheDB,
    pub tx_kind: TxKind,
    pub access_list: AccessList,
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

// Taken from cmd/ef_tests/ethrex/types.rs, didn't want to fight dependencies yet
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct AccessListItem {
    pub address: Address,
    pub storage_keys: Vec<H256>,
}

type AccessList = Vec<(Address, Vec<H256>)>;

pub fn get_valid_jump_destinations(code: &Bytes) -> Result<HashSet<usize>, VMError> {
    let mut valid_jump_destinations = HashSet::new();
    let mut pc = 0;

    while let Some(&opcode_number) = code.get(pc) {
        let current_opcode = Opcode::from(opcode_number);

        if current_opcode == Opcode::JUMPDEST {
            // If current opcode is jumpdest, add it to valid destinations set
            valid_jump_destinations.insert(pc);
        } else if (Opcode::PUSH1..=Opcode::PUSH32).contains(&current_opcode) {
            // If current opcode is push, skip as many positions as the size of the push
            let size_to_push =
                opcode_number
                    .checked_sub(u8::from(Opcode::PUSH1))
                    .ok_or(VMError::Internal(
                        InternalError::ArithmeticOperationUnderflow,
                    ))?;
            let skip_length = usize::from(size_to_push.checked_add(1).ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow,
            ))?);
            pc = pc.checked_add(skip_length).ok_or(VMError::Internal(
                InternalError::ArithmeticOperationOverflow, // to fail, pc should be at least usize max - 31
            ))?;
        }

        pc = pc.checked_add(1).ok_or(VMError::Internal(
            InternalError::ArithmeticOperationOverflow, // to fail, code len should be more than usize max
        ))?;
    }

    Ok(valid_jump_destinations)
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
        mut cache: CacheDB,
        access_list: AccessList,
    ) -> Result<Self, VMError> {
        // Maybe this decision should be made in an upper layer

        // Add sender, coinbase and recipient (in the case of a Call) to cache [https://www.evm.codes/about#access_list]
        let mut default_touched_accounts =
            HashSet::from_iter([env.origin, env.coinbase].iter().cloned());

        let mut default_touched_storage_slots: HashMap<Address, HashSet<H256>> = HashMap::new();

        // Add access lists contents to cache
        for (address, keys) in access_list.clone() {
            default_touched_accounts.insert(address);
            let mut warm_slots = HashSet::new();
            for slot in keys {
                warm_slots.insert(slot);
            }
            default_touched_storage_slots.insert(address, warm_slots);
        }

        // Add precompiled contracts addresses to cache.
        // TODO: Use the addresses from precompiles.rs in a future
        for i in 1..=10 {
            default_touched_accounts.insert(Address::from_low_u64_be(i));
        }

        match to {
            TxKind::Call(address_to) => {
                default_touched_accounts.insert(address_to);

                // add address_to to cache
                let recipient_account_info = db.get_account_info(address_to);
                cache::insert_account(
                    &mut cache,
                    address_to,
                    Account::from(recipient_account_info.clone()),
                );

                // CALL tx
                let initial_call_frame = CallFrame::new(
                    env.origin,
                    address_to,
                    address_to,
                    recipient_account_info.bytecode,
                    value,
                    calldata.clone(),
                    false,
                    env.gas_limit,
                    U256::zero(),
                    0,
                    false,
                );

                let substate = Substate {
                    selfdestrutct_set: HashSet::new(),
                    touched_accounts: default_touched_accounts,
                    touched_storage_slots: default_touched_storage_slots,
                    created_accounts: HashSet::new(),
                };

                Ok(Self {
                    call_frames: vec![initial_call_frame],
                    db,
                    env,
                    accrued_substate: substate,
                    cache,
                    tx_kind: to,
                    access_list,
                })
            }
            TxKind::Create => {
                // CREATE tx

                let new_contract_address =
                    VM::calculate_create_address(env.origin, db.get_account_info(env.origin).nonce)
                        .map_err(|_| {
                            VMError::Internal(InternalError::CouldNotComputeCreateAddress)
                        })?;

                default_touched_accounts.insert(new_contract_address);

                let created_contract = Account::new(value, Bytes::new(), 1, HashMap::new());

                cache::insert_account(&mut cache, new_contract_address, created_contract);

                let initial_call_frame = CallFrame::new(
                    env.origin,
                    new_contract_address,
                    new_contract_address,
                    Bytes::new(), // Bytecode is assigned after passing validations.
                    value,
                    calldata, // Calldata is removed after passing validations.
                    false,
                    env.gas_limit,
                    U256::zero(),
                    0,
                    false,
                );

                let substate = Substate {
                    selfdestrutct_set: HashSet::new(),
                    touched_accounts: default_touched_accounts,
                    touched_storage_slots: default_touched_storage_slots,
                    created_accounts: HashSet::from([new_contract_address]),
                };

                Ok(Self {
                    call_frames: vec![initial_call_frame],
                    db,
                    env,
                    accrued_substate: substate,
                    cache,
                    tx_kind: TxKind::Create,
                    access_list,
                })
            }
        }
    }

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

        if is_precompile(&current_call_frame.code_address) {
            let precompile_result = execute_precompile(current_call_frame);

            match precompile_result {
                Ok(output) => {
                    self.call_frames.push(current_call_frame.clone());

                    return Ok(TransactionReport {
                        result: TxResult::Success,
                        new_state: self.cache.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: 0,
                        output,
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    });
                }
                Err(error) => {
                    if error.is_internal() {
                        return Err(error);
                    }

                    self.call_frames.push(current_call_frame.clone());

                    self.restore_state(backup_db, backup_substate, backup_refunded_gas);

                    return Ok(TransactionReport {
                        result: TxResult::Revert(error),
                        new_state: self.cache.clone(),
                        gas_used: current_call_frame.gas_limit.low_u64(),
                        gas_refunded: 0,
                        output: Bytes::new(),
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    });
                }
            }
        }

        loop {
            let opcode = current_call_frame.next_opcode();

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
                    let n_bytes = get_n_value(op, Opcode::PUSH1)?;
                    self.op_push(current_call_frame, n_bytes)
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
                    let depth = get_n_value(op, Opcode::DUP1)?;
                    self.op_dup(current_call_frame, depth)
                }
                // SWAPn
                op if (Opcode::SWAP1..=Opcode::SWAP16).contains(&op) => {
                    let depth = get_n_value(op, Opcode::SWAP1)?;
                    self.op_swap(current_call_frame, depth)
                }
                Opcode::POP => self.op_pop(current_call_frame),
                op if (Opcode::LOG0..=Opcode::LOG4).contains(&op) => {
                    let number_of_topics = get_number_of_topics(op)?;
                    self.op_log(current_call_frame, number_of_topics)
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

            if opcode != Opcode::JUMP && opcode != Opcode::JUMPI {
                current_call_frame.increment_pc()?;
            }

            // Gas refunds are applied at the end of a transaction. Should it be implemented here?

            match op_result {
                Ok(OpcodeSuccess::Continue) => {}
                Ok(OpcodeSuccess::Result(_)) => {
                    self.call_frames.push(current_call_frame.clone());
                    // On successful create check output validity
                    if (self.is_create() && current_call_frame.depth == 0)
                        || current_call_frame.create_op_called
                    {
                        let contract_code = current_call_frame.output.clone();
                        let code_length = contract_code.len();
                        let code_deposit_cost = U256::from(code_length)
                            .checked_mul(CODE_DEPOSIT_COST)
                            .ok_or(VMError::Internal(
                                InternalError::ArithmeticOperationOverflow,
                            ))?;

                        // Revert
                        // If the first byte of code is 0xef
                        // If the code_length > MAX_CODE_SIZE
                        // If current_consumed_gas + code_deposit_cost > gas_limit
                        let validate_create = if code_length > MAX_CODE_SIZE {
                            Err(VMError::ContractOutputTooBig)
                        } else if contract_code.first().unwrap_or(&0) == &INVALID_CONTRACT_PREFIX {
                            Err(VMError::InvalidContractPrefix)
                        } else if self
                            .increase_consumed_gas(current_call_frame, code_deposit_cost)
                            .is_err()
                        {
                            Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded))
                        } else {
                            Ok(current_call_frame.to)
                        };

                        match validate_create {
                            Ok(new_address) => {
                                // Set bytecode to new account if success
                                self.update_account_bytecode(new_address, contract_code)?;
                            }
                            Err(error) => {
                                // Revert if error
                                current_call_frame.gas_used = current_call_frame.gas_limit;
                                self.restore_state(backup_db, backup_substate, backup_refunded_gas);

                                return Ok(TransactionReport {
                                    result: TxResult::Revert(error),
                                    new_state: self.cache.clone(),
                                    gas_used: current_call_frame.gas_used.low_u64(),
                                    gas_refunded: self.env.refunded_gas.low_u64(),
                                    output: current_call_frame.output.clone(),
                                    logs: current_call_frame.logs.clone(),
                                    created_address: None,
                                });
                            }
                        }
                    }

                    return Ok(TransactionReport {
                        result: TxResult::Success,
                        new_state: self.cache.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: self.env.refunded_gas.low_u64(),
                        output: current_call_frame.output.clone(),
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    });
                }
                Err(error) => {
                    self.call_frames.push(current_call_frame.clone());

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
                    }

                    self.restore_state(backup_db, backup_substate, backup_refunded_gas);

                    return Ok(TransactionReport {
                        result: TxResult::Revert(error),
                        new_state: self.cache.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: self.env.refunded_gas.low_u64(),
                        output: current_call_frame.output.clone(), // Bytes::new() if error is not RevertOpcode
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    });
                }
            }
        }
    }

    fn restore_state(
        &mut self,
        backup_cache: CacheDB,
        backup_substate: Substate,
        backup_refunded_gas: U256,
    ) {
        self.cache = backup_cache;
        self.accrued_substate = backup_substate;
        self.env.refunded_gas = backup_refunded_gas;
    }

    fn is_create(&self) -> bool {
        matches!(self.tx_kind, TxKind::Create)
    }

    fn add_intrinsic_gas(&mut self, initial_call_frame: &mut CallFrame) -> Result<(), VMError> {
        // Intrinsic gas is the gas consumed by the transaction before the execution of the opcodes. Section 6.2 in the Yellow Paper.

        // Intrinsic Gas = Calldata cost + Create cost + Base cost + Access list cost
        let mut intrinsic_gas = U256::zero();

        // Calldata Cost
        // 4 gas for each zero byte in the transaction data 16 gas for each non-zero byte in the transaction.
        let calldata_cost =
            gas_cost::tx_calldata(&initial_call_frame.calldata).map_err(VMError::OutOfGas)?;

        intrinsic_gas = intrinsic_gas
            .checked_add(calldata_cost)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        // Base Cost
        intrinsic_gas = intrinsic_gas
            .checked_add(TX_BASE_COST)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        // Create Cost
        if self.is_create() {
            intrinsic_gas = intrinsic_gas
                .checked_add(CREATE_BASE_COST)
                .ok_or(OutOfGasError::ConsumedGasOverflow)?;

            let number_of_words = initial_call_frame.calldata.len().div_ceil(WORD_SIZE);

            intrinsic_gas = intrinsic_gas
                .checked_add(
                    U256::from(number_of_words)
                        .checked_mul(U256::from(2))
                        .ok_or(OutOfGasError::ConsumedGasOverflow)?,
                )
                .ok_or(OutOfGasError::ConsumedGasOverflow)?;
        }

        // Access List Cost
        let mut access_lists_cost = U256::zero();
        for (_, keys) in self.access_list.clone() {
            access_lists_cost = access_lists_cost
                .checked_add(ACCESS_LIST_ADDRESS_COST)
                .ok_or(OutOfGasError::ConsumedGasOverflow)?;
            for _ in keys {
                access_lists_cost = access_lists_cost
                    .checked_add(ACCESS_LIST_STORAGE_KEY_COST)
                    .ok_or(OutOfGasError::ConsumedGasOverflow)?;
            }
        }

        intrinsic_gas = intrinsic_gas
            .checked_add(access_lists_cost)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;

        self.increase_consumed_gas(initial_call_frame, intrinsic_gas)
            .map_err(|_| TxValidationError::IntrinsicGasTooLow)?;

        Ok(())
    }

    /// Gets the max blob gas cost for a transaction that a user is willing to pay.
    fn get_max_blob_gas_cost(&self) -> Result<U256, VMError> {
        let blob_gas_used = U256::from(self.env.tx_blob_hashes.len())
            .checked_mul(BLOB_GAS_PER_BLOB)
            .unwrap_or_default();

        let blob_gas_cost = self
            .env
            .tx_max_fee_per_blob_gas
            .unwrap_or_default()
            .checked_mul(blob_gas_used)
            .ok_or(InternalError::UndefinedState(1))?;

        Ok(blob_gas_cost)
    }

    pub fn get_base_fee_per_blob_gas(&self) -> Result<U256, VMError> {
        fake_exponential(
            MIN_BASE_FEE_PER_BLOB_GAS,
            self.env.block_excess_blob_gas.unwrap_or_default().low_u64(), //Maybe replace unwrap_or_default for sth else later.
            BLOB_BASE_FEE_UPDATE_FRACTION,
        )
    }

    /// ## Description
    /// This method performs validations and returns an error if any of the validations fail.
    /// It also makes pre-execution changes:
    /// - It increases sender nonce
    /// - It substracts up-front-cost from sender balance.
    /// - It adds value to receiver balance.
    /// - It calculates and adds intrinsic gas to the 'gas used' of callframe and environment.
    ///   See 'docs' for more information about validations.
    fn prepare_execution(&mut self, initial_call_frame: &mut CallFrame) -> Result<(), VMError> {
        //TODO: This should revert the transaction, not throw an error. And I don't know if it should be done here...
        // if self.is_create() {
        //     // If address is already in db, there's an error
        //     let new_address_acc = self.db.get_account_info(call_frame.to);
        //     if !new_address_acc.is_empty() {
        //         return Err(VMError::AddressAlreadyOccupied);
        //     }
        // }
        let sender_address = self.env.origin;
        let sender_account = self.get_account(sender_address);

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        let gaslimit_price_product =
            self.env
                .gas_price
                .checked_mul(self.env.gas_limit)
                .ok_or(VMError::TxValidation(
                    TxValidationError::GasLimitPriceProductOverflow,
                ))?;

        // Up front cost is the maximum amount of wei that a user is willing to pay for. Gaslimit * gasprice + value + blob_gas_cost
        let value = initial_call_frame.msg_value;

        // blob gas cost = max fee per blob gas * blob gas used
        // https://eips.ethereum.org/EIPS/eip-4844
        let blob_gas_cost = self.get_max_blob_gas_cost()?;

        // For the transaction to be valid the sender account has to have a balance >= gas_price * gas_limit + value if tx is type 0 and 1
        // balance >= max_fee_per_gas * gas_limit + value + blob_gas_cost if tx is type 2 or 3
        let gas_fee_for_valid_tx = self
            .env
            .tx_max_fee_per_gas
            .unwrap_or(self.env.gas_price)
            .checked_mul(self.env.gas_limit)
            .ok_or(VMError::TxValidation(
                TxValidationError::GasLimitPriceProductOverflow,
            ))?;

        let balance_for_valid_tx = gas_fee_for_valid_tx
            .checked_add(value)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?
            .checked_add(blob_gas_cost)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?;
        if sender_account.info.balance < balance_for_valid_tx {
            return Err(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ));
        }

        // The real cost to deduct is calculated as effective_gas_price * gas_limit + value + blob_gas_cost
        let up_front_cost = gaslimit_price_product
            .checked_add(value)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?
            .checked_add(blob_gas_cost)
            .ok_or(VMError::TxValidation(
                TxValidationError::InsufficientAccountFunds,
            ))?;
        // There is no error specified for overflow in up_front_cost in ef_tests. Maybe we can go with GasLimitPriceProductOverflow or InsufficientAccountFunds.

        // (2) INSUFFICIENT_ACCOUNT_FUNDS
        self.decrease_account_balance(sender_address, up_front_cost)
            .map_err(|_| TxValidationError::InsufficientAccountFunds)?;

        // Transfer value to receiver
        let receiver_address = initial_call_frame.to;
        // msg_value is already transferred into the created contract at creation.
        if !self.is_create() {
            self.increase_account_balance(receiver_address, initial_call_frame.msg_value)?;
        }

        // (3) INSUFFICIENT_MAX_FEE_PER_GAS
        if self.env.tx_max_fee_per_gas.unwrap_or(self.env.gas_price) < self.env.base_fee_per_gas {
            return Err(VMError::TxValidation(
                TxValidationError::InsufficientMaxFeePerGas,
            ));
        }

        // (4) INITCODE_SIZE_EXCEEDED
        if self.is_create() {
            // INITCODE_SIZE_EXCEEDED
            if initial_call_frame.calldata.len() > INIT_CODE_MAX_SIZE {
                return Err(VMError::TxValidation(
                    TxValidationError::InitcodeSizeExceeded,
                ));
            }
        }

        // (5) INTRINSIC_GAS_TOO_LOW
        self.add_intrinsic_gas(initial_call_frame)?;

        // (6) NONCE_IS_MAX
        self.increment_account_nonce(sender_address)
            .map_err(|_| VMError::TxValidation(TxValidationError::NonceIsMax))?;

        // (7) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        if let (Some(tx_max_priority_fee), Some(tx_max_fee_per_gas)) = (
            self.env.tx_max_priority_fee_per_gas,
            self.env.tx_max_fee_per_gas,
        ) {
            if tx_max_priority_fee > tx_max_fee_per_gas {
                return Err(VMError::TxValidation(
                    TxValidationError::PriorityGreaterThanMaxFeePerGas,
                ));
            }
        }

        // (8) SENDER_NOT_EOA
        if sender_account.has_code() {
            return Err(VMError::TxValidation(TxValidationError::SenderNotEOA));
        }

        // (9) GAS_ALLOWANCE_EXCEEDED
        if self.env.gas_limit > self.env.block_gas_limit {
            return Err(VMError::TxValidation(
                TxValidationError::GasAllowanceExceeded,
            ));
        }

        // (10) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        if let Some(tx_max_fee_per_blob_gas) = self.env.tx_max_fee_per_blob_gas {
            if tx_max_fee_per_blob_gas < self.get_base_fee_per_blob_gas()? {
                return Err(VMError::TxValidation(
                    TxValidationError::InsufficientMaxFeePerBlobGas,
                ));
            }
        }

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        if self.env.tx_max_fee_per_blob_gas.is_some() {
            let blob_hashes = &self.env.tx_blob_hashes;

            // (11) TYPE_3_TX_ZERO_BLOBS
            if blob_hashes.is_empty() {
                return Err(VMError::TxValidation(TxValidationError::Type3TxZeroBlobs));
            }

            // (12) TYPE_3_TX_INVALID_BLOB_VERSIONED_HASH
            for blob_hash in blob_hashes {
                let blob_hash = blob_hash.as_bytes();
                if let Some(first_byte) = blob_hash.first() {
                    if !VALID_BLOB_PREFIXES.contains(first_byte) {
                        return Err(VMError::TxValidation(
                            TxValidationError::Type3TxInvalidBlobVersionedHash,
                        ));
                    }
                }
            }

            // (13) TYPE_3_TX_PRE_FORK -> This is not necessary for now because we are not supporting pre-cancun transactions yet. But we should somehow be able to tell the current context.

            // (14) TYPE_3_TX_BLOB_COUNT_EXCEEDED
            if blob_hashes.len() > MAX_BLOB_COUNT {
                return Err(VMError::TxValidation(
                    TxValidationError::Type3TxBlobCountExceeded,
                ));
            }

            // (15) TYPE_3_TX_CONTRACT_CREATION
            if self.is_create() {
                return Err(VMError::TxValidation(
                    TxValidationError::Type3TxContractCreation,
                ));
            }
        }

        if self.is_create() {
            // Assign bytecode to context and empty calldata
            initial_call_frame.assign_bytecode(initial_call_frame.calldata.clone());
            initial_call_frame.calldata = Bytes::new();
        }

        Ok(())
    }

    /// ## Changes post execution
    /// 1. Undo value transfer if the transaction was reverted
    /// 2. Return unused gas + gas refunds to the sender.
    /// 3. Pay coinbase fee
    /// 4. Destruct addresses in selfdestruct set.
    fn post_execution_changes(
        &mut self,
        initial_call_frame: &CallFrame,
        report: &mut TransactionReport,
    ) -> Result<(), VMError> {
        // POST-EXECUTION Changes
        let sender_address = initial_call_frame.msg_sender;
        let receiver_address = initial_call_frame.to;

        // 1. Undo value transfer if the transaction was reverted
        if let TxResult::Revert(_) = report.result {
            // msg_value was not increased in the receiver account when is a create transaction.
            if !self.is_create() {
                self.decrease_account_balance(receiver_address, initial_call_frame.msg_value)?;
            }
            self.increase_account_balance(sender_address, initial_call_frame.msg_value)?;
        }

        // 2. Return unused gas + gas refunds to the sender.
        let max_gas = self.env.gas_limit.low_u64();
        let consumed_gas = report.gas_used;
        let refunded_gas = report.gas_refunded.min(
            consumed_gas
                .checked_div(5)
                .ok_or(VMError::Internal(InternalError::UndefinedState(-1)))?,
        );
        // "The max refundable proportion of gas was reduced from one half to one fifth by EIP-3529 by Buterin and Swende [2021] in the London release"
        report.gas_refunded = refunded_gas;

        let gas_to_return = max_gas
            .checked_sub(consumed_gas)
            .and_then(|gas| gas.checked_add(refunded_gas))
            .ok_or(VMError::Internal(InternalError::UndefinedState(0)))?;

        let wei_return_amount = self
            .env
            .gas_price
            .checked_mul(U256::from(gas_to_return))
            .ok_or(VMError::Internal(InternalError::UndefinedState(1)))?;

        self.increase_account_balance(sender_address, wei_return_amount)?;

        // 3. Pay coinbase fee
        let coinbase_address = self.env.coinbase;

        let gas_to_pay_coinbase = consumed_gas
            .checked_sub(refunded_gas)
            .ok_or(VMError::Internal(InternalError::UndefinedState(2)))?;

        let priority_fee_per_gas = self
            .env
            .gas_price
            .checked_sub(self.env.base_fee_per_gas)
            .ok_or(VMError::GasPriceIsLowerThanBaseFee)?;
        let coinbase_fee = U256::from(gas_to_pay_coinbase)
            .checked_mul(priority_fee_per_gas)
            .ok_or(VMError::BalanceOverflow)?;

        if coinbase_fee != U256::zero() {
            self.increase_account_balance(coinbase_address, coinbase_fee)?;
        };

        // 4. Destruct addresses in selfdestruct set.
        // In Cancun the only addresses destroyed are contracts created in this transaction, so we 'destroy' them by just removing them from the cache, as if they never existed.
        for address in &self.accrued_substate.selfdestrutct_set {
            remove_account(&mut self.cache, address);
        }

        Ok(())
    }

    pub fn transact(&mut self) -> Result<TransactionReport, VMError> {
        let mut initial_call_frame = self
            .call_frames
            .pop()
            .ok_or(VMError::Internal(InternalError::CouldNotPopCallframe))?;

        self.prepare_execution(&mut initial_call_frame)?;

        let mut report = self.execute(&mut initial_call_frame)?;
        if self.is_create() && !report.is_success() {
            remove_account(&mut self.cache, &initial_call_frame.to);
        }

        self.post_execution_changes(&initial_call_frame, &mut report)?;
        // There shouldn't be any errors here but I don't know what the desired behavior is if something goes wrong.

        report.new_state.clone_from(&self.cache);

        Ok(report)
    }

    pub fn current_call_frame_mut(&mut self) -> Result<&mut CallFrame, VMError> {
        self.call_frames.last_mut().ok_or(VMError::Internal(
            InternalError::CouldNotAccessLastCallframe,
        ))
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

    /// Increases gas consumption of CallFrame and Environment, returning an error if the callframe gas limit is reached.
    pub fn increase_consumed_gas(
        &mut self,
        current_call_frame: &mut CallFrame,
        gas: U256,
    ) -> Result<(), VMError> {
        let potential_consumed_gas = current_call_frame
            .gas_used
            .checked_add(gas)
            .ok_or(OutOfGasError::ConsumedGasOverflow)?;
        if potential_consumed_gas > current_call_frame.gas_limit {
            return Err(VMError::OutOfGas(OutOfGasError::MaxGasLimitExceeded));
        }

        current_call_frame.gas_used = potential_consumed_gas;

        Ok(())
    }

    pub fn cache_from_db(&mut self, address: Address) {
        let acc_info = self.db.get_account_info(address);
        cache::insert_account(
            &mut self.cache,
            address,
            Account {
                info: acc_info.clone(),
                storage: HashMap::new(),
            },
        );
    }

    /// Accesses to an account's information.
    ///
    /// Accessed accounts are stored in the `touched_accounts` set.
    /// Accessed accounts take place in some gas cost computation.
    #[must_use]
    pub fn access_account(&mut self, address: Address) -> (AccountInfo, bool) {
        let address_was_cold = self.accrued_substate.touched_accounts.insert(address);
        let account = match cache::get_account(&self.cache, &address) {
            Some(account) => account.info.clone(),
            None => self.db.get_account_info(address),
        };
        (account, address_was_cold)
    }

    /// Accesses to an account's storage slot.
    ///
    /// Accessed storage slots are stored in the `touched_storage_slots` set.
    /// Accessed storage slots take place in some gas cost computation.
    pub fn access_storage_slot(
        &mut self,
        address: Address,
        key: H256,
    ) -> Result<(StorageSlot, bool), VMError> {
        let storage_slot_was_cold = self
            .accrued_substate
            .touched_storage_slots
            .entry(address)
            .or_default()
            .insert(key);
        let storage_slot = match cache::get_account(&self.cache, &address) {
            Some(account) => match account.storage.get(&key) {
                Some(storage_slot) => storage_slot.clone(),
                None => {
                    let value = self.db.get_storage_slot(address, key);
                    StorageSlot {
                        original_value: value,
                        current_value: value,
                    }
                }
            },
            None => {
                let value = self.db.get_storage_slot(address, key);
                StorageSlot {
                    original_value: value,
                    current_value: value,
                }
            }
        };

        // When updating account storage of an account that's not yet cached we need to store the StorageSlot in the account
        // Note: We end up caching the account because it is the most straightforward way of doing it.
        let account = self.get_account_mut(address)?;
        account.storage.insert(key, storage_slot.clone());

        Ok((storage_slot, storage_slot_was_cold))
    }

    pub fn increase_account_balance(
        &mut self,
        address: Address,
        increase: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.balance = account
            .info
            .balance
            .checked_add(increase)
            .ok_or(VMError::BalanceOverflow)?;
        Ok(())
    }

    pub fn decrease_account_balance(
        &mut self,
        address: Address,
        decrease: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.balance = account
            .info
            .balance
            .checked_sub(decrease)
            .ok_or(VMError::BalanceUnderflow)?;
        Ok(())
    }

    pub fn increment_account_nonce(&mut self, address: Address) -> Result<u64, VMError> {
        let account = self.get_account_mut(address)?;
        account.info.nonce = account
            .info
            .nonce
            .checked_add(1)
            .ok_or(VMError::NonceOverflow)?;
        Ok(account.info.nonce)
    }

    pub fn decrement_account_nonce(&mut self, address: Address) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.nonce = account
            .info
            .nonce
            .checked_sub(1)
            .ok_or(VMError::NonceUnderflow)?;
        Ok(())
    }

    pub fn update_account_bytecode(
        &mut self,
        address: Address,
        new_bytecode: Bytes,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        account.info.bytecode = new_bytecode;
        Ok(())
    }

    pub fn update_account_storage(
        &mut self,
        address: Address,
        key: H256,
        new_value: U256,
    ) -> Result<(), VMError> {
        let account = self.get_account_mut(address)?;
        let account_original_storage_slot_value = account
            .storage
            .get(&key)
            .map_or(U256::zero(), |slot| slot.original_value);
        let slot = account.storage.entry(key).or_insert(StorageSlot {
            original_value: account_original_storage_slot_value,
            current_value: new_value,
        });
        slot.current_value = new_value;
        Ok(())
    }

    pub fn get_account_mut(&mut self, address: Address) -> Result<&mut Account, VMError> {
        if !cache::is_account_cached(&self.cache, &address) {
            let account_info = self.db.get_account_info(address);
            let account = Account {
                info: account_info,
                storage: HashMap::new(),
            };
            cache::insert_account(&mut self.cache, address, account.clone());
        }
        cache::get_account_mut(&mut self.cache, &address)
            .ok_or(VMError::Internal(InternalError::AccountNotFound))
    }

    /// Gets account, first checking the cache and then the database (caching in the second case)
    pub fn get_account(&mut self, address: Address) -> Account {
        match cache::get_account(&self.cache, &address) {
            Some(acc) => acc.clone(),
            None => {
                let account_info = self.db.get_account_info(address);
                let account = Account {
                    info: account_info,
                    storage: HashMap::new(),
                };
                cache::insert_account(&mut self.cache, address, account.clone());
                account
            }
        }
    }
}

fn get_n_value(op: Opcode, base_opcode: Opcode) -> Result<usize, VMError> {
    let offset = (usize::from(op))
        .checked_sub(usize::from(base_opcode))
        .ok_or(VMError::InvalidOpcode)?
        .checked_add(1)
        .ok_or(VMError::InvalidOpcode)?;

    Ok(offset)
}

fn get_number_of_topics(op: Opcode) -> Result<u8, VMError> {
    let number_of_topics = (u8::from(op))
        .checked_sub(u8::from(Opcode::LOG0))
        .ok_or(VMError::InvalidOpcode)?;

    Ok(number_of_topics)
}
