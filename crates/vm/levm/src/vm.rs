use crate::{
    call_frame::CallFrame,
    constants::*,
    errors::{OpcodeSuccess, ResultReason, TransactionReport, TxResult, VMError},
    opcodes::Opcode,
    primitives::{Address, Bytes, H256, U256},
};
use ethereum_rust_rlp;
use ethereum_rust_rlp::encode::RLPEncode;
use ethereum_types::H160;
use keccak_hash::keccak;
use sha3::{Digest, Keccak256};
use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
};

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Account {
    pub address: Address,
    pub balance: U256,
    pub bytecode: Bytes,
    pub storage: HashMap<U256, StorageSlot>,
    pub nonce: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageSlot {
    pub original_value: U256,
    pub current_value: U256,
    pub is_cold: bool,
}

impl Account {
    pub fn new(
        address: Address,
        balance: U256,
        bytecode: Bytes,
        nonce: u64,
        storage: HashMap<U256, StorageSlot>,
    ) -> Self {
        Self {
            address,
            balance,
            bytecode,
            storage,
            nonce,
        }
    }

    pub fn has_code(&self) -> bool {
        !(self.bytecode.is_empty()
            || self.bytecode_hash() == H256::from_str(EMPTY_CODE_HASH_STR).unwrap())
    }

    pub fn bytecode_hash(&self) -> H256 {
        keccak(self.bytecode.as_ref())
    }

    pub fn is_empty(&self) -> bool {
        self.balance.is_zero() && self.nonce == 0 && self.bytecode.is_empty()
    }

    pub fn with_balance(mut self, balance: U256) -> Self {
        self.balance = balance;
        self
    }

    pub fn with_bytecode(mut self, bytecode: Bytes) -> Self {
        self.bytecode = bytecode;
        self
    }

    pub fn with_storage(mut self, storage: HashMap<U256, StorageSlot>) -> Self {
        self.storage = storage;
        self
    }

    pub fn with_nonce(mut self, nonce: u64) -> Self {
        self.nonce = nonce;
        self
    }

    pub fn increment_nonce(&mut self) {
        self.nonce += 1;
    }
}

pub type Storage = HashMap<U256, H256>;

#[derive(Clone, Debug, Default)]
pub struct Db {
    pub accounts: HashMap<Address, Account>,
    // contracts: HashMap<B256, Bytecode>,
    pub block_hashes: HashMap<U256, H256>,
}

impl Db {
    pub fn read_account_storage(&self, address: &Address, key: &U256) -> Option<StorageSlot> {
        self.accounts
            .get(address)
            .and_then(|account| account.storage.get(key))
            .cloned()
    }

    pub fn write_account_storage(&mut self, address: &Address, key: U256, slot: StorageSlot) {
        self.accounts
            .entry(*address)
            .or_default()
            .storage
            .insert(key, slot);
    }

    pub fn get_account_bytecode(&self, address: &Address) -> Bytes {
        self.accounts
            .get(address)
            .map_or(Bytes::new(), |acc| acc.bytecode.clone())
    }

    pub fn balance(&mut self, address: &Address) -> U256 {
        self.accounts
            .get(address)
            .map_or(U256::zero(), |acc| acc.balance)
    }

    pub fn add_account(&mut self, address: Address, account: Account) {
        self.accounts.insert(address, account);
    }

    pub fn increment_account_nonce(&mut self, address: &Address) {
        if let Some(acc) = self.accounts.get_mut(address) {
            acc.increment_nonce()
        }
    }

    /// Returns the account associated with the given address.
    /// If the account does not exist in the Db, it creates a new one with the given address.
    pub fn get_account(&mut self, address: &Address) -> Result<&Account, VMError> {
        if self.accounts.contains_key(address) {
            return Ok(self.accounts.get(address).unwrap());
        }

        let new_account = Account {
            address: *address,
            ..Default::default()
        };

        self.accounts.insert(*address, new_account);

        Ok(self.accounts.get(address).unwrap())
    }
}

#[derive(Debug, Clone, Default)]
// TODO: https://github.com/lambdaclass/ethereum_rust/issues/604
pub struct Substate {
    pub warm_addresses: HashSet<Address>,
}

#[derive(Debug, Default, Clone)]
pub struct Environment {
    /// The sender address of the transaction that originated
    /// this execution.
    pub origin: Address,
    pub consumed_gas: U256,
    pub refunded_gas: U256,
    pub gas_limit: U256,
    pub block_number: U256,
    pub coinbase: Address,
    pub timestamp: U256,
    pub prev_randao: Option<H256>,
    pub chain_id: U256,
    pub base_fee_per_gas: U256,
    pub gas_price: U256,
    pub block_excess_blob_gas: Option<U256>,
    pub block_blob_gas_used: Option<U256>,
    pub tx_blob_hashes: Option<Vec<H256>>,
}

#[derive(Debug, Clone, Default)]
pub struct VM {
    pub call_frames: Vec<CallFrame>,
    pub env: Environment,
    /// Information that is acted upon immediately following the
    /// transaction.
    pub accrued_substate: Substate,
    /// Mapping between addresses (160-bit identifiers) and account
    /// states.
    pub db: Db,
}

fn address_to_word(address: Address) -> U256 {
    // This unwrap can't panic, as Address are 20 bytes long and U256 use 32 bytes
    U256::from_str(&format!("{address:?}")).unwrap()
}

pub fn word_to_address(word: U256) -> Address {
    let mut bytes = [0u8; 32];
    word.to_big_endian(&mut bytes);
    Address::from_slice(&bytes[12..])
}

impl VM {
    #[allow(clippy::too_many_arguments)]
    fn call_type_transaction(
        to: Address,
        msg_sender: Address,
        value: U256,
        calldata: Bytes,
        gas_limit: U256,
        block_number: U256,
        coinbase: Address,
        timestamp: U256,
        prev_randao: Option<H256>,
        chain_id: U256,
        base_fee_per_gas: U256,
        gas_price: U256,
        db: Db,
        block_blob_gas_used: Option<U256>,
        block_excess_blob_gas: Option<U256>,
        tx_blob_hashes: Option<Vec<H256>>,
    ) -> Result<VM, VMError> {
        let bytecode = db.get_account_bytecode(&to);

        let initial_call_frame = CallFrame::new(
            msg_sender,
            to,
            to,
            None,
            bytecode,
            value,
            calldata.clone(),
            false,
            gas_limit,
            TX_BASE_COST,
            0,
        );

        let env = Environment {
            consumed_gas: TX_BASE_COST,
            origin: msg_sender,
            refunded_gas: U256::zero(),
            gas_limit,
            block_number,
            coinbase,
            timestamp,
            prev_randao,
            chain_id,
            base_fee_per_gas,
            gas_price,
            block_blob_gas_used,
            block_excess_blob_gas,
            tx_blob_hashes,
        };

        Ok(VM {
            call_frames: vec![initial_call_frame],
            db,
            env,
            accrued_substate: Substate::default(),
        })
    }

    // Functionality should be:
    // (1) Check whether caller has enough balance to make a transfer
    // (2) Derive the new contract’s address from the caller’s address (passing in the creator account’s nonce)
    // (3) Create the new contract account using the derived contract address (changing the “world state” StateDB)
    // (4) Transfer the initial Ether endowment from caller to the new contract
    // (5) Set input data as contract’s deploy code, then execute it with EVM. The ret variable is the returned contract code
    // (6) Check for error. Or if the contract code is too big, fail. Charge the user gas then set the contract code
    // Source: https://medium.com/@hayeah/diving-into-the-ethereum-vm-part-5-the-smart-contract-creation-process-cb7b6133b855
    #[allow(clippy::too_many_arguments)]
    fn create_type_transaction(
        sender: Address,
        secret_key: H256,
        db: &mut Db,
        value: U256,
        calldata: Bytes,
        block_number: U256,
        coinbase: Address,
        timestamp: U256,
        prev_randao: Option<H256>,
        chain_id: U256,
        base_fee_per_gas: U256,
        gas_price: U256,
        block_blob_gas_used: Option<U256>,
        block_excess_blob_gas: Option<U256>,
        tx_blob_hashes: Option<Vec<H256>>,
        salt: Option<U256>,
    ) -> Result<VM, VMError> {
        let mut db_copy = db.clone();
        let mut sender_account = match db_copy.accounts.get(&sender) {
            Some(acc) => acc,
            None => {
                return Err(VMError::OutOfGas);
            }
        }
        .clone();

        // (1)
        if sender_account.balance < value {
            return Err(VMError::OutOfGas); // Maybe a more personalized error
        }

        sender_account.nonce = sender_account
            .nonce
            .checked_add(1)
            .ok_or(VMError::NonceOverflow)?;

        // (2)
        let new_contract_address = match salt {
            Some(salt) => VM::calculate_create2_address(sender, &calldata, salt),
            None => VM::calculate_create_address(sender, sender_account.nonce),
        };

        // If address is already in db, there's an error
        if db_copy.accounts.contains_key(&new_contract_address) {
            return Err(VMError::AddressAlreadyOccupied);
        }

        // (3)
        let mut created_contract = Account::new(
            new_contract_address,
            value,
            calldata.clone(),
            1,
            Default::default(),
        );
        db_copy.add_account(new_contract_address, created_contract.clone());

        // (4)
        sender_account.balance -= value;
        created_contract.balance += value;

        // (5)
        let code: Bytes = calldata.clone();

        // Call the contract
        let mut vm = VM::new(
            Some(created_contract.address),
            sender,
            value,
            code,
            sender_account.balance,
            block_number,
            coinbase,
            timestamp,
            prev_randao,
            chain_id,
            base_fee_per_gas,
            gas_price,
            &mut db_copy,
            block_blob_gas_used,
            block_excess_blob_gas,
            tx_blob_hashes,
            secret_key,
            None,
        )?;

        let res = vm.transact()?;
        // Don't use a revert bc work with clones, so don't have to save previous state

        let contract_code = res.output;

        // (6)
        if contract_code.len() > MAX_CODE_SIZE {
            return Err(VMError::ContractOutputTooBig);
        }
        // Supposing contract code has contents
        if contract_code[0] == INVALID_CONTRACT_PREFIX {
            return Err(VMError::InvalidInitialByte);
        }

        // If the initialization code completes successfully, a final contract-creation cost is paid,
        // the code-deposit cost, c, proportional to the size of the created contract’s code
        let creation_cost = 200 * contract_code.len();

        sender_account.balance = sender_account
            .balance
            .checked_sub(U256::from(creation_cost))
            .ok_or(VMError::OutOfGas)?;

        created_contract.bytecode = contract_code;

        let mut acc = db_copy.accounts.get_mut(&sender).unwrap();
        *acc = sender_account;
        acc = db_copy.accounts.get_mut(&new_contract_address).unwrap();
        *acc = created_contract;

        *db = db_copy;
        Ok(vm)
    }

    // TODO: Refactor this.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        to: Option<Address>,
        msg_sender: Address,
        value: U256,
        calldata: Bytes,
        gas_limit: U256,
        block_number: U256,
        coinbase: Address,
        timestamp: U256,
        prev_randao: Option<H256>,
        chain_id: U256,
        base_fee_per_gas: U256,
        gas_price: U256,
        db: &mut Db,
        block_blob_gas_used: Option<U256>,
        block_excess_blob_gas: Option<U256>,
        tx_blob_hashes: Option<Vec<H256>>,
        secret_key: H256,
        salt: Option<U256>,
    ) -> Result<Self, VMError> {
        // Maybe this desicion should be made in an upper layer
        match to {
            Some(address) => VM::call_type_transaction(
                address,
                msg_sender,
                value,
                calldata,
                gas_limit,
                block_number,
                coinbase,
                timestamp,
                prev_randao,
                chain_id,
                base_fee_per_gas,
                gas_price,
                db.clone(),
                block_blob_gas_used,
                block_excess_blob_gas,
                tx_blob_hashes,
            ),
            None => VM::create_type_transaction(
                msg_sender,
                secret_key,
                db,
                value,
                calldata,
                block_number,
                coinbase,
                timestamp,
                prev_randao,
                chain_id,
                base_fee_per_gas,
                gas_price,
                block_blob_gas_used,
                block_excess_blob_gas,
                tx_blob_hashes,
                salt,
            ),
        }
    }

    pub fn execute(&mut self, current_call_frame: &mut CallFrame) -> TransactionReport {
        // let mut current_call_frame = self
        //     .call_frames
        //     .pop()
        //     .expect("Fatal Error: This should not happen"); // if this happens during execution, we are cooked 💀
        loop {
            let opcode = current_call_frame.next_opcode().unwrap_or(Opcode::STOP);
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
                _ => Err(VMError::OpcodeNotFound),
            };

            // Gas refunds are applied at the end of a transaction. Should it be implemented here?

            match op_result {
                Ok(OpcodeSuccess::Continue) => {}
                Ok(OpcodeSuccess::Result(_)) => {
                    self.call_frames.push(current_call_frame.clone());
                    return TransactionReport {
                        result: TxResult::Success,
                        new_state: self.db.accounts.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: self.env.refunded_gas.low_u64(),
                        output: current_call_frame.returndata.clone(),
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    };
                }
                Err(error) => {
                    self.call_frames.push(current_call_frame.clone());

                    // CONSUME ALL GAS UNLESS THE ERROR IS FROM REVERT OPCODE
                    if error != VMError::RevertOpcode {
                        let left_gas = current_call_frame.gas_limit - current_call_frame.gas_used;
                        current_call_frame.gas_used += left_gas;
                        self.env.consumed_gas += left_gas;
                    }

                    return TransactionReport {
                        result: TxResult::Revert(error),
                        new_state: self.db.accounts.clone(),
                        gas_used: current_call_frame.gas_used.low_u64(),
                        gas_refunded: self.env.refunded_gas.low_u64(),
                        output: current_call_frame.returndata.clone(),
                        logs: current_call_frame.logs.clone(),
                        created_address: None,
                    };
                }
            }
        }
    }

    /// Based on Ethereum yellow paper's initial tests of intrinsic validity (Section 6). The last version is
    /// Shanghai, so there are probably missing Cancun validations. The intrinsic validations are:
    ///
    /// (1) The transaction is well-formed RLP, with no additional trailing bytes;
    /// (2) The transaction signature is valid;
    /// (3) The transaction nonce is valid (equivalent to the sender account's
    /// current nonce);
    /// (4) The sender account has no contract code deployed (see EIP-3607).
    /// (5) The gas limit is no smaller than the intrinsic gas, used by the
    /// transaction;
    /// (6) The sender account balance contains at least the cost, required in
    /// up-front payment;
    /// (7) The max fee per gas, in the case of type 2 transactions, or gasPrice,
    /// in the case of type 0 and type 1 transactions, is greater than or equal to
    /// the block’s base fee;
    /// (8) For type 2 transactions, max priority fee per fas, must be no larger
    /// than max fee per fas.
    fn validate_transaction(&self) -> Result<(), VMError> {
        // Validations (1), (2), (3), (5), and (8) are assumed done in upper layers.
        let sender_account = match self.db.accounts.get(&self.env.origin) {
            Some(acc) => acc,
            None => return Err(VMError::AddressDoesNotMatchAnAccount),
            // This is a check for completeness. However if it were a none and
            // it was not caught it would be caught in clause 6.
        };
        // (4)
        if sender_account.has_code() {
            return Err(VMError::SenderAccountShouldNotHaveBytecode);
        }
        // (6)
        if sender_account.balance < self.call_frames[0].msg_value {
            return Err(VMError::SenderBalanceShouldContainTransferValue);
        }
        // (7)
        if self.env.gas_price < self.env.base_fee_per_gas {
            return Err(VMError::GasPriceIsLowerThanBaseFee);
        }
        Ok(())
    }

    pub fn transact(&mut self) -> Result<TransactionReport, VMError> {
        self.validate_transaction()?;

        let initial_gas = Default::default();

        self.env.consumed_gas = initial_gas;

        let mut current_call_frame = self.call_frames.pop().unwrap();
        Ok(self.execute(&mut current_call_frame))
    }

    pub fn current_call_frame_mut(&mut self) -> &mut CallFrame {
        self.call_frames.last_mut().unwrap()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn generic_call(
        &mut self,
        current_call_frame: &mut CallFrame,
        gas_limit: U256,
        value: U256,
        msg_sender: Address,
        to: Address,
        code_address: Address,
        delegate: Option<Address>,
        _should_transfer_value: bool,
        is_static: bool,
        args_offset: usize,
        args_size: usize,
        ret_offset: usize,
        ret_size: usize,
    ) -> Result<OpcodeSuccess, VMError> {
        // check balance
        if self.db.balance(&current_call_frame.msg_sender) < value {
            current_call_frame.stack.push(U256::from(REVERT_FOR_CALL))?;
            return Ok(OpcodeSuccess::Continue);
        }

        // transfer value
        // transfer(&current_call_frame.msg_sender, &address, value);

        let code_address_bytecode = self.db.get_account_bytecode(&code_address);
        if code_address_bytecode.is_empty() {
            // should stop
            current_call_frame
                .stack
                .push(U256::from(SUCCESS_FOR_CALL))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Stop));
        }

        self.db.increment_account_nonce(&code_address);

        let calldata = current_call_frame
            .memory
            .load_range(args_offset, args_size)
            .into();

        let gas_limit = std::cmp::min(
            gas_limit,
            (current_call_frame.gas_limit - current_call_frame.gas_used) / 64 * 63,
        );

        let mut new_call_frame = CallFrame::new(
            msg_sender,
            to,
            code_address,
            delegate,
            code_address_bytecode,
            value,
            calldata,
            is_static,
            gas_limit,
            U256::zero(),
            current_call_frame.depth + 1,
        );

        current_call_frame.sub_return_data_offset = ret_offset;
        current_call_frame.sub_return_data_size = ret_size;

        // self.call_frames.push(new_call_frame.clone());
        let tx_report = self.execute(&mut new_call_frame);

        current_call_frame.gas_used += tx_report.gas_used.into(); // We add the gas used by the sub-context to the current one after it's execution.
        current_call_frame.logs.extend(tx_report.logs);
        current_call_frame
            .memory
            .store_n_bytes(ret_offset, &tx_report.output, ret_size);
        current_call_frame.sub_return_data = tx_report.output;

        // What to do, depending on TxResult
        match tx_report.result {
            TxResult::Success => {
                current_call_frame
                    .stack
                    .push(U256::from(SUCCESS_FOR_CALL))?;
            }
            TxResult::Revert(_error) => {
                // Behavior for revert between contexts goes here if necessary
                // It is also possible to differentiate between RevertOpcode error and other kinds of revert.

                current_call_frame.stack.push(U256::from(REVERT_FOR_CALL))?;
            }
        }

        Ok(OpcodeSuccess::Continue)
    }

    /// Calculates the address of a new conctract using the CREATE opcode as follow
    ///
    /// address = keccak256(rlp([sender_address,sender_nonce]))[12:]
    pub fn calculate_create_address(sender_address: Address, sender_nonce: u64) -> H160 {
        let mut encoded = Vec::new();
        sender_address.encode(&mut encoded);
        sender_nonce.encode(&mut encoded);
        let mut hasher = Keccak256::new();
        hasher.update(encoded);
        Address::from_slice(&hasher.finalize()[12..])
    }

    /// Calculates the address of a new contract using the CREATE2 opcode as follow
    ///
    /// initialization_code = memory[offset:offset+size]
    ///
    /// address = keccak256(0xff + sender_address + salt + keccak256(initialization_code))[12:]
    pub fn calculate_create2_address(
        sender_address: Address,
        initialization_code: &Bytes,
        salt: U256,
    ) -> H160 {
        let mut hasher = Keccak256::new();
        hasher.update(initialization_code.clone());
        let initialization_code_hash = hasher.finalize();
        let mut hasher = Keccak256::new();
        let mut salt_bytes = [0; 32];
        salt.to_big_endian(&mut salt_bytes);
        hasher.update([0xff]);
        hasher.update(sender_address.as_bytes());
        hasher.update(salt_bytes);
        hasher.update(initialization_code_hash);
        Address::from_slice(&hasher.finalize()[12..])
    }

    /// Common behavior for CREATE and CREATE2 opcodes
    ///
    /// Could be used for CREATE type transactions
    pub fn create(
        &mut self,
        value_in_wei_to_send: U256,
        code_offset_in_memory: usize,
        code_size_in_memory: usize,
        salt: Option<U256>,
        current_call_frame: &mut CallFrame,
    ) -> Result<OpcodeSuccess, VMError> {
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

        let sender_account = self
            .db
            .accounts
            .get_mut(&current_call_frame.msg_sender)
            .unwrap();

        if sender_account.balance < value_in_wei_to_send {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        }

        let Some(new_nonce) = sender_account.nonce.checked_add(1) else {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        };
        sender_account.nonce = new_nonce;
        sender_account.balance -= value_in_wei_to_send;
        let code = Bytes::from(
            current_call_frame
                .memory
                .load_range(code_offset_in_memory, code_size_in_memory),
        );

        let new_address = match salt {
            Some(salt) => {
                Self::calculate_create2_address(current_call_frame.msg_sender, &code, salt)
            }
            None => {
                Self::calculate_create_address(current_call_frame.msg_sender, sender_account.nonce)
            }
        };

        if self.db.accounts.contains_key(&new_address) {
            current_call_frame
                .stack
                .push(U256::from(REVERT_FOR_CREATE))?;
            return Ok(OpcodeSuccess::Result(ResultReason::Revert));
        }

        let new_account = Account::new(
            new_address,
            value_in_wei_to_send,
            code.clone(),
            0,
            Default::default(),
        );
        self.db.add_account(new_address, new_account);

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
            None,
            true,
            false,
            code_offset_in_memory,
            code_size_in_memory,
            code_offset_in_memory,
            code_size_in_memory,
        )
    }

    /// Increases gas consumption of CallFrame and Environment, returning an error if the callframe gas limit is reached.
    pub fn increase_consumed_gas(
        &mut self,
        current_call_frame: &mut CallFrame,
        gas: U256,
    ) -> Result<(), VMError> {
        if current_call_frame.gas_used + gas > current_call_frame.gas_limit {
            return Err(VMError::OutOfGas);
        }
        current_call_frame.gas_used += gas;
        self.env.consumed_gas += gas;
        Ok(())
    }
}
