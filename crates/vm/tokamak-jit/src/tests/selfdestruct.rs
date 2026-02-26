//! SELFDESTRUCT E2E tests via interpreter.
//!
//! Tests:
//! - Basic SELFDESTRUCT to a target address
//! - SELFDESTRUCT with zero balance
//! - SELFDESTRUCT to self (self-beneficiary)
//! - Double SELFDESTRUCT (previously_destroyed flag)
//!
//! These tests exercise the SELFDESTRUCT opcode through the LEVM interpreter.
//! The JIT Host::selfdestruct() path is tested separately with revmc-backend.

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use ethrex_common::constants::EMPTY_TRIE_HASH;
    use ethrex_common::types::{Account, BlockHeader, Code};
    use ethrex_common::{Address, U256};
    use ethrex_levm::db::gen_db::GeneralizedDatabase;
    use ethrex_levm::tracing::LevmCallTracer;
    use ethrex_levm::vm::{VM, VMType};
    use rustc_hash::FxHashMap;

    use crate::tests::test_helpers::{CONTRACT_ADDR, SENDER_ADDR, make_test_env, make_test_tx};

    /// Target address for SELFDESTRUCT beneficiary.
    const TARGET_ADDR: u64 = 0x200;

    /// Create DB with custom balances to avoid U256::MAX overflow on transfer.
    fn make_db_with_balances(
        entries: Vec<(Address, U256, Code, FxHashMap<ethrex_common::H256, U256>)>,
    ) -> GeneralizedDatabase {
        let store = ethrex_storage::Store::new("", ethrex_storage::EngineType::InMemory)
            .expect("in-memory store");
        let header = BlockHeader {
            state_root: *EMPTY_TRIE_HASH,
            ..Default::default()
        };
        let vm_db: ethrex_vm::DynVmDatabase = Box::new(
            ethrex_blockchain::vm::StoreVmDatabase::new(store, header).expect("StoreVmDatabase"),
        );

        let mut cache = FxHashMap::default();
        for (addr, balance, code, storage) in entries {
            cache.insert(addr, Account::new(balance, code, 0, storage));
        }

        GeneralizedDatabase::new_with_account_state(Arc::new(vm_db), cache)
    }

    /// Build bytecode: PUSH20 <target_address> SELFDESTRUCT
    fn make_selfdestruct_bytecode(target: Address) -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0x73); // PUSH20
        code.extend_from_slice(target.as_bytes());
        code.push(0xff); // SELFDESTRUCT
        code
    }

    /// Build bytecode: PUSH20 <target> SELFDESTRUCT PUSH20 <target> SELFDESTRUCT STOP
    fn make_double_selfdestruct_bytecode(target: Address) -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0x73);
        code.extend_from_slice(target.as_bytes());
        code.push(0xff);
        code.push(0x73);
        code.extend_from_slice(target.as_bytes());
        code.push(0xff);
        code.push(0x00); // STOP
        code
    }

    #[test]
    fn test_selfdestruct_basic_success() {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);
        let target_addr = Address::from_low_u64_be(TARGET_ADDR);

        let bytecode = make_selfdestruct_bytecode(target_addr);
        let balance = U256::from(1_000_000u64);

        let mut db = make_db_with_balances(vec![
            (
                contract_addr,
                balance,
                Code::from_bytecode(Bytes::from(bytecode)),
                FxHashMap::default(),
            ),
            (
                sender_addr,
                balance,
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
            (
                target_addr,
                U256::zero(),
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
        ]);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation should succeed");

        let report = vm.execute().expect("execution should succeed");
        assert!(
            report.is_success(),
            "SELFDESTRUCT should succeed: {:?}",
            report.result
        );
    }

    #[test]
    fn test_selfdestruct_to_self() {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);

        let bytecode = make_selfdestruct_bytecode(contract_addr);
        let balance = U256::from(500_000u64);

        let mut db = make_db_with_balances(vec![
            (
                contract_addr,
                balance,
                Code::from_bytecode(Bytes::from(bytecode)),
                FxHashMap::default(),
            ),
            (
                sender_addr,
                balance,
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
        ]);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation should succeed");

        let report = vm.execute().expect("execution should succeed");
        assert!(
            report.is_success(),
            "SELFDESTRUCT to self should succeed: {:?}",
            report.result
        );
    }

    #[test]
    fn test_selfdestruct_to_nonexistent_target() {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);
        let target_addr = Address::from_low_u64_be(0xDEAD);

        let bytecode = make_selfdestruct_bytecode(target_addr);
        let balance = U256::from(1_000_000u64);

        // Target NOT in the state — tests the new-account creation path
        let mut db = make_db_with_balances(vec![
            (
                contract_addr,
                balance,
                Code::from_bytecode(Bytes::from(bytecode)),
                FxHashMap::default(),
            ),
            (
                sender_addr,
                balance,
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
        ]);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation should succeed");

        let report = vm.execute().expect("execution should succeed");
        assert!(
            report.is_success(),
            "SELFDESTRUCT to nonexistent target should succeed: {:?}",
            report.result
        );
    }

    #[test]
    fn test_double_selfdestruct() {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);
        let target_addr = Address::from_low_u64_be(TARGET_ADDR);

        let bytecode = make_double_selfdestruct_bytecode(target_addr);
        let balance = U256::from(1_000_000u64);

        let mut db = make_db_with_balances(vec![
            (
                contract_addr,
                balance,
                Code::from_bytecode(Bytes::from(bytecode)),
                FxHashMap::default(),
            ),
            (
                sender_addr,
                balance,
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
            (
                target_addr,
                U256::zero(),
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
        ]);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation should succeed");

        let report = vm.execute().expect("execution should succeed");
        assert!(
            report.is_success(),
            "Double SELFDESTRUCT should succeed: {:?}",
            report.result
        );
    }

    #[test]
    fn test_selfdestruct_gas_consumed() {
        let contract_addr = Address::from_low_u64_be(CONTRACT_ADDR);
        let sender_addr = Address::from_low_u64_be(SENDER_ADDR);
        let target_addr = Address::from_low_u64_be(TARGET_ADDR);

        let bytecode = make_selfdestruct_bytecode(target_addr);
        let balance = U256::from(1_000_000u64);

        let mut db = make_db_with_balances(vec![
            (
                contract_addr,
                balance,
                Code::from_bytecode(Bytes::from(bytecode)),
                FxHashMap::default(),
            ),
            (
                sender_addr,
                balance,
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
            (
                target_addr,
                U256::zero(),
                Code::from_bytecode(Bytes::new()),
                FxHashMap::default(),
            ),
        ]);

        let env = make_test_env(sender_addr);
        let tx = make_test_tx(contract_addr, Bytes::new());
        let mut vm = VM::new(env, &mut db, &tx, LevmCallTracer::disabled(), VMType::L1)
            .expect("VM creation should succeed");

        let report = vm.execute().expect("execution should succeed");
        assert!(report.is_success());
        // SELFDESTRUCT costs: 5000 base + cold address surcharge + PUSH20 (3)
        // Plus intrinsic gas (21000).
        assert!(
            report.gas_used > 21_000,
            "gas_used should include intrinsic + opcode costs, got {}",
            report.gas_used
        );
    }
}
