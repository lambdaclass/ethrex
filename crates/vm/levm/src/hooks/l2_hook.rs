use crate::{
    errors::{ContextResult, InternalError},
    hooks::{DefaultHook, default_hook, hook::Hook},
    opcodes::Opcode,
    vm::VM,
};

use ethrex_common::{Address, H160, U256};

pub const COMMON_BRIDGE_L2_ADDRESS: Address = H160([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xff, 0xff,
]);

pub struct L2Hook {}

impl Hook for L2Hook {
    fn prepare_execution(&mut self, vm: &mut VM<'_>) -> Result<(), crate::errors::VMError> {
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
            return DefaultHook.finalize_execution(vm, ctx_result);
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
