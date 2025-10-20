use std::{cell::RefCell, rc::Rc};

use crate::{
    db::gen_db::GeneralizedDatabase,
    errors::{ContextResult, InternalError},
    hooks::{
        DefaultHook,
        default_hook::{self, set_bytecode_and_code_address},
        fee_token_hook::FeeTokenHook,
        hook::Hook,
    },
    opcodes::Opcode,
    tracing::LevmCallTracer,
    vm::{VM, VMType},
};

use bytes::Bytes;
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        EIP1559Transaction, PrivilegedL2Transaction, Transaction, TxKind, fee_config::FeeConfig,
    },
    utils::{keccak, u256_to_big_endian},
};

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

pub struct L2Hook {
    pub fee_config: FeeConfig,
}

impl Hook for L2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
        let sender_address = vm.env.origin;
        // dbg!(sender_address);
        // #[allow(clippy::unwrap_used)]
        // let some_address =
        //     Address::from_slice(&hex::decode("4417092b70a3e5f10dc504d0947dd256b965fc62").unwrap());
        #[allow(clippy::unwrap_used)]
        let fee_token =
            Address::from_slice(&hex::decode("00e29d532f1c62a923ee51ee439bfc1500b1ce4d").unwrap());

        // dbg!(balance_of(&mut vm.db.clone(), fee_token, some_address))?;

        // 0x70a08231
        let balance_of_selector = vec![0x70, 0xa0, 0x82, 0x31];
        let mut data = vec![];
        data.extend_from_slice(&balance_of_selector);
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&sender_address.0);
        dbg!("{:?}", &hex::encode(&data));
        #[allow(clippy::unwrap_used)]
        let a = vm
            .db
            .get_account(Address::from_slice(
                &hex::decode("4417092b70a3e5f10dc504d0947dd256b965fc62").unwrap(),
            ))?
            .info
            .nonce;
        dbg!(a);

        let mut db_clone = vm.db.clone();
        let tx_check_balance = EIP1559Transaction {
            chain_id: 65536999,
            nonce: a,
            max_priority_fee_per_gas: 9999999,
            max_fee_per_gas: 9999999,
            gas_limit: 9999999,
            to: TxKind::Call(fee_token),
            value: U256::zero(),
            data: data.into(),
            ..Default::default()
        };
        let tx_check_balance = Transaction::EIP1559Transaction(tx_check_balance);
        let mut env_clone = vm.env.clone();
        env_clone.is_privileged = false;
        let mut new_vm = VM::new(
            env_clone,
            &mut db_clone,
            &tx_check_balance,
            LevmCallTracer::disabled(),
            VMType::L1,
        )?;
        new_vm.hooks = vec![Rc::new(RefCell::new(FeeTokenHook {
            fee_token_address: fee_token,
        }))];
        set_bytecode_and_code_address(&mut new_vm)?;
        let b = new_vm.execute()?;
        println!("{:?}", &b.output);
        if !vm.env.is_privileged {
            return DefaultHook.prepare_execution(vm);
        }

        let sender_address = vm.env.origin;
        let sender_balance = vm.db.get_account(sender_address)?.info.balance;

        let mut tx_should_fail = false;

        // The bridge is allowed to mint ETH.
        // This is done by not decreasing it's balance when it's the source of a transfer.
        // For other privileged transactions, insufficient balance can't cause an error
        // since they must always be accepted, and an error would mark them as invalid
        // Instead, we make them revert by inserting a revert2
        if sender_address != COMMON_BRIDGE_L2_ADDRESS {
            let value = vm.current_call_frame.msg_value;
            if value > sender_balance {
                tx_should_fail = true;
            } else {
                // This should never fail, since we just checked the balance is enough.
                vm.decrease_account_balance(sender_address, value)
                    .map_err(|_| {
                        InternalError::Custom(
                            "Insufficient funds in privileged transaction".to_string(),
                        )
                    })?;
            }
        }

        // if fork > prague: default_hook::validate_min_gas_limit
        // NOT CHECKED: the l1 makes spamming privileged transactions not economical

        // (1) GASLIMIT_PRICE_PRODUCT_OVERFLOW
        // NOT CHECKED: privileged transactions do not pay for gas

        // (2) INSUFFICIENT_MAX_FEE_PER_BLOB_GAS
        // NOT CHECKED: the blob price does not matter, privileged transactions do not support blobs

        // (4) INSUFFICIENT_MAX_FEE_PER_GAS
        // NOT CHECKED: privileged transactions do not pay for gas, the gas price is irrelevant

        // (5) INITCODE_SIZE_EXCEEDED
        // NOT CHECKED: privileged transactions can't be of "create" type

        // (6) INTRINSIC_GAS_TOO_LOW
        // CHANGED: the gas should be charged, but the transaction shouldn't error
        if vm.add_intrinsic_gas().is_err() {
            tx_should_fail = true;
        }

        // (7) NONCE_IS_MAX
        // NOT CHECKED: privileged transactions don't use the account nonce

        // (8) PRIORITY_GREATER_THAN_MAX_FEE_PER_GAS
        // NOT CHECKED: privileged transactions do not pay for gas, the gas price is irrelevant

        // (9) SENDER_NOT_EOA
        // NOT CHECKED: contracts can also send privileged transactions

        // (10) GAS_ALLOWANCE_EXCEEDED
        // CHECKED: we don't want to exceed block limits
        default_hook::validate_gas_allowance(vm)?;

        // Transaction is type 3 if tx_max_fee_per_blob_gas is Some
        // NOT CHECKED: privileged transactions are not type 3

        // Transaction is type 4 if authorization_list is Some
        // NOT CHECKED: privileged transactions are not type 4

        if tx_should_fail {
            // If the transaction failed some validation, but it must still be included
            // To prevent it from taking effect, we force it to revert
            vm.current_call_frame.msg_value = U256::zero();
            vm.current_call_frame
                .set_code(vec![Opcode::INVALID.into()].into())?;
            return Ok(());
        }

        default_hook::transfer_value(vm)?;

        default_hook::set_bytecode_and_code_address(vm)?;

        Ok(())
    }

    fn finalize_execution(
        &mut self,
        vm: &mut VM<'_>,
        ctx_result: &mut ContextResult,
    ) -> Result<(), crate::errors::VMError> {
        if !vm.env.is_privileged {
            DefaultHook.finalize_execution(vm, ctx_result)?;
            // Different from L1, the base fee is not burned
            return pay_to_fee_vault(vm, ctx_result.gas_used, self.fee_config.fee_vault);
        }

        if !ctx_result.is_success() && vm.env.origin != COMMON_BRIDGE_L2_ADDRESS {
            default_hook::undo_value_transfer(vm)?;
        }

        // Even if privileged transactions themselves can't create
        // They can call contracts that use CREATE/CREATE2
        default_hook::delete_self_destruct_accounts(vm)?;

        Ok(())
    }
}

fn pay_to_fee_vault(
    vm: &mut VM<'_>,
    gas_to_pay: u64,
    fee_vault: Option<Address>,
) -> Result<(), crate::errors::VMError> {
    let Some(fee_vault) = fee_vault else {
        // No fee vault configured, base fee is effectively burned
        return Ok(());
    };

    let base_fee = U256::from(gas_to_pay)
        .checked_mul(vm.env.base_fee_per_gas)
        .ok_or(InternalError::Overflow)?;

    vm.increase_account_balance(fee_vault, base_fee)?;
    Ok(())
}

fn balance_of(
    db: &mut GeneralizedDatabase,
    token: Address,
    holder: Address,
) -> Result<U256, InternalError> {
    // abi.encode(holder, uint256(0))
    let mut encoded = [0u8; 64];
    encoded[12..32].copy_from_slice(&holder.0); // address padding
    let storage_key = H256(keccak(encoded).0);

    dbg!(hex::encode(&encoded));
    dbg!(token);
    dbg!(holder);
    dbg!(storage_key);
    // Asegúrate de que la cuenta esté cacheada
    let contract = db.get_account(token)?;
    let storage = dbg!(&contract.storage);
    Ok(storage.get(&storage_key).cloned().unwrap_or(U256::zero()))
}
