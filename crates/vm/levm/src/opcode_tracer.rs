use bytes::Bytes;
use ethrex_common::{
    Address, H256, U256,
    tracing::{MemoryChunk, OpcodeStep, OpcodeTraceResult},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Configuration for the opcode (EIP-3155) tracer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpcodeTracerConfig {
    /// When true, stack values are not included in each step.
    pub disable_stack: bool,
    /// When true, memory contents are included in each step.
    pub enable_memory: bool,
    /// When true, storage diffs at SLOAD/SSTORE steps are not captured.
    pub disable_storage: bool,
    /// When true, return data from the previous sub-call is included.
    pub enable_return_data: bool,
    /// Maximum number of log entries to collect.  0 = unlimited.
    pub limit: usize,
}

/// Per-step opcode tracer for EIP-3155 output.
///
/// Use `LevmOpcodeTracer::disabled()` when tracing is not wanted;
/// the dispatch-loop guard is a single `if self.opcode_tracer.active` branch
/// with no other overhead on the fast path.
#[derive(Debug)]
pub struct LevmOpcodeTracer {
    /// Whether this tracer is active.
    pub active: bool,
    /// Configuration.
    pub cfg: OpcodeTracerConfig,
    /// Collected per-step entries.
    pub logs: Vec<OpcodeStep>,
    /// Final output bytes (from RETURN / REVERT).
    pub output: Bytes,
    /// Top-level error string, if the transaction reverted.
    pub error: Option<String>,
    /// Gas used by the transaction.
    pub gas_used: u64,
    /// Running approximate size counter for limit enforcement.
    /// Currently tracks `logs.len()`.
    pub total_size: usize,
    /// Explicit gas cost written by CALL/CALLCODE/DELEGATECALL/STATICCALL/CREATE/CREATE2
    /// handlers before invoking the child frame.  The dispatch loop prefers this value
    /// over the (incorrect) gas-diff that would include forwarded gas.
    pub last_opcode_gas_cost: Option<u64>,
}

impl LevmOpcodeTracer {
    /// Returns an inactive tracer.  No allocations; zero overhead on the hot path.
    pub fn disabled() -> Self {
        Self {
            active: false,
            cfg: OpcodeTracerConfig::default(),
            logs: Vec::new(),
            output: Bytes::new(),
            error: None,
            gas_used: 0,
            total_size: 0,
            last_opcode_gas_cost: None,
        }
    }

    /// Returns an active tracer with the given config.
    pub fn new(cfg: OpcodeTracerConfig) -> Self {
        Self {
            active: true,
            cfg,
            logs: Vec::new(),
            output: Bytes::new(),
            error: None,
            gas_used: 0,
            total_size: 0,
            last_opcode_gas_cost: None,
        }
    }

    /// Captures pre-step state, building and buffering an `OpcodeStep` entry.
    ///
    /// Called BEFORE the opcode executes.  `pc` must be the address of the
    /// current opcode (before `advance_pc(1)`).
    ///
    /// `stack_view` must already be bottom-first (caller reverses LEVM's top-first
    /// layout) and empty when `cfg.disable_stack` is true.
    ///
    /// `memory_view` is the live byte slice for the current frame (caller provides
    /// this only when `cfg.enable_memory` is true; otherwise pass `&[]`).
    ///
    /// `storage_kv` is pre-fetched by the caller via `read_storage_for_trace`; it is
    /// `None` for all opcodes except SLOAD/SSTORE (or when storage capture is disabled).
    #[expect(
        clippy::too_many_arguments,
        reason = "all fields are required per-step state from the dispatch-loop hook"
    )]
    pub fn pre_step_capture(
        &mut self,
        pc: u64,
        opcode: u8,
        gas: u64,
        depth: u32,
        refund: u64,
        stack_view: &[U256],
        memory_view: &[u8],
        mem_size: u64,
        return_data: &Bytes,
        storage_kv: Option<(Address, H256, H256)>,
    ) {
        // Enforce limit: stop appending once total_size reaches the cap.
        if self.cfg.limit > 0 && self.total_size >= self.cfg.limit {
            return;
        }

        // Stack: Some(vec) when capture enabled; None when disabled (emits JSON null).
        let stack = if !self.cfg.disable_stack {
            Some(stack_view.to_vec())
        } else {
            None
        };

        // Memory: chunked 32-byte slices when enabled; field omitted otherwise.
        // Emit Some(vec![]) when enabled and memory is empty (EIP-3155 requires
        // the field present whenever enableMemory=true).
        let memory = if self.cfg.enable_memory {
            if memory_view.is_empty() {
                Some(vec![])
            } else {
                let chunks = memory_view
                    .chunks(32)
                    .map(|c| {
                        let mut arr = [0u8; 32];
                        if let Some(dst) = arr.get_mut(..c.len()) {
                            dst.copy_from_slice(c);
                        }
                        MemoryChunk(arr)
                    })
                    .collect();
                Some(chunks)
            }
        } else {
            None
        };

        // Storage: single-entry map for this step only (no accumulation).
        let storage = if let Some((_addr, key, value)) = storage_kv {
            let mut m = BTreeMap::new();
            m.insert(key, value);
            Some(m)
        } else {
            None
        };

        // returnData: actual bytes when enabled; empty Bytes otherwise.
        let return_data_field = if self.cfg.enable_return_data {
            return_data.clone()
        } else {
            Bytes::new()
        };

        let log = OpcodeStep {
            pc,
            op: opcode,
            gas,
            gas_cost: 0, // patched in finalize_step
            mem_size,
            depth,
            return_data: return_data_field,
            refund,
            stack,
            memory,
            storage,
            error: None, // patched in finalize_step
        };

        self.logs.push(log);
        self.total_size = self.logs.len();
    }

    /// Patches the most-recently-buffered entry with the actual gas cost and any
    /// step-level error string.  Called immediately after the opcode handler returns.
    pub fn finalize_step(&mut self, gas_cost: u64, error: Option<&str>) {
        if let Some(log) = self.logs.last_mut() {
            log.gas_cost = gas_cost;
            log.error = error.map(str::to_owned);
        }
    }

    /// Assembles the final `OpcodeTraceResult` after the transaction finishes.
    pub fn take_result(&mut self) -> OpcodeTraceResult {
        OpcodeTraceResult {
            pass: self.error.is_none(),
            gas_used: self.gas_used,
            output: std::mem::take(&mut self.output),
            steps: std::mem::take(&mut self.logs),
        }
    }
}

// ─── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Database,
        environment::{EVMConfig, Environment},
        errors::{DatabaseError, ExecutionReport},
        tracing::LevmCallTracer,
        vm::{VM, VMType},
    };
    use bytes::Bytes;
    use ethrex_common::{
        Address, H256, U256,
        tracing::opcode_name,
        types::{
            Account, AccountState, ChainConfig, Code, CodeMetadata, EIP1559Transaction, Fork,
            Transaction, TxKind,
        },
    };
    use ethrex_crypto::NativeCrypto;
    use rustc_hash::FxHashMap;
    use std::sync::Arc;

    // ── Minimal in-memory database ────────────────────────────────────────

    struct TestDb {
        accounts: FxHashMap<Address, Account>,
    }

    impl TestDb {
        fn new() -> Self {
            Self {
                accounts: FxHashMap::default(),
            }
        }

        fn with_account(mut self, addr: Address, acc: Account) -> Self {
            self.accounts.insert(addr, acc);
            self
        }
    }

    impl Database for TestDb {
        fn get_account_state(&self, address: Address) -> Result<AccountState, DatabaseError> {
            use ethrex_common::constants::EMPTY_TRIE_HASH;
            Ok(self
                .accounts
                .get(&address)
                .map(|acc| AccountState {
                    nonce: acc.info.nonce,
                    balance: acc.info.balance,
                    storage_root: *EMPTY_TRIE_HASH,
                    code_hash: acc.info.code_hash,
                })
                .unwrap_or_default())
        }

        fn get_storage_value(&self, address: Address, key: H256) -> Result<U256, DatabaseError> {
            Ok(self
                .accounts
                .get(&address)
                .and_then(|acc| acc.storage.get(&key).copied())
                .unwrap_or_default())
        }

        fn get_block_hash(&self, _block_number: u64) -> Result<H256, DatabaseError> {
            Ok(H256::zero())
        }

        fn get_chain_config(&self) -> Result<ChainConfig, DatabaseError> {
            Ok(ChainConfig::default())
        }

        fn get_account_code(&self, code_hash: H256) -> Result<Code, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(acc.code.clone());
                }
            }
            Ok(Code::default())
        }

        fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, DatabaseError> {
            for acc in self.accounts.values() {
                if acc.info.code_hash == code_hash {
                    return Ok(CodeMetadata {
                        length: acc.code.bytecode.len() as u64,
                    });
                }
            }
            Ok(CodeMetadata { length: 0 })
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────────

    const GAS_LIMIT: u64 = 1_000_000;
    const SENDER_ADDR: u64 = 0x1000;
    const CONTRACT_ADDR: u64 = 0x2000;

    fn run_bytecode(
        bytecode: Bytes,
        cfg: OpcodeTracerConfig,
    ) -> (LevmOpcodeTracer, ExecutionReport) {
        let sender = Address::from_low_u64_be(SENDER_ADDR);
        let contract = Address::from_low_u64_be(CONTRACT_ADDR);

        let code = Code::from_bytecode(bytecode, &NativeCrypto);
        let acc = Account::new(U256::zero(), code, 1, FxHashMap::default());
        let sender_acc = Account::new(
            U256::from(1_000_000_000_u64),
            Code::default(),
            0,
            FxHashMap::default(),
        );

        let db = TestDb::new()
            .with_account(contract, acc)
            .with_account(sender, sender_acc);

        let accounts_map: FxHashMap<Address, Account> = db.accounts.clone().into_iter().collect();
        let mut gen_db = crate::db::gen_db::GeneralizedDatabase::new_with_account_state(
            Arc::new(db),
            accounts_map,
        );

        let fork = Fork::Cancun;
        let blob_schedule = EVMConfig::canonical_values(fork);
        let env = Environment {
            origin: sender,
            gas_limit: GAS_LIMIT,
            config: EVMConfig::new(fork, blob_schedule),
            block_number: 1,
            coinbase: Address::from_low_u64_be(0xCCC),
            timestamp: 1000,
            prev_randao: Some(H256::zero()),
            difficulty: U256::zero(),
            slot_number: U256::zero(),
            chain_id: U256::from(1),
            base_fee_per_gas: U256::from(1000),
            base_blob_fee_per_gas: U256::from(1),
            gas_price: U256::from(1000),
            block_excess_blob_gas: None,
            block_blob_gas_used: None,
            tx_blob_hashes: vec![],
            tx_max_priority_fee_per_gas: None,
            tx_max_fee_per_gas: Some(U256::from(1000)),
            tx_max_fee_per_blob_gas: None,
            tx_nonce: 0,
            block_gas_limit: GAS_LIMIT * 2,
            is_privileged: false,
            fee_token: None,
            disable_balance_check: false,
        };

        let tx = Transaction::EIP1559Transaction(EIP1559Transaction {
            to: TxKind::Call(contract),
            value: U256::zero(),
            data: Bytes::new(),
            gas_limit: GAS_LIMIT,
            max_fee_per_gas: 1000,
            max_priority_fee_per_gas: 1,
            ..Default::default()
        });

        let mut vm = VM::new(
            env,
            &mut gen_db,
            &tx,
            LevmCallTracer::disabled(),
            VMType::L1,
            &NativeCrypto,
        )
        .unwrap();

        vm.opcode_tracer = LevmOpcodeTracer::new(cfg);
        let report = vm.execute().unwrap();

        let tracer = std::mem::replace(&mut vm.opcode_tracer, LevmOpcodeTracer::disabled());
        (tracer, report)
    }

    // ── PUSH1/PUSH1/ADD/STOP test ─────────────────────────────────────────

    /// `PUSH1 0x01 PUSH1 0x02 ADD STOP`
    /// Expected: 4 entries, pc=[0,2,4,5], op=[PUSH1,PUSH1,ADD,STOP],
    /// gas_cost=[3,3,3,0], depth=1, stack evolves correctly.
    #[test]
    fn test_opcode_tracer_push_add_stop() {
        // Bytecode: 0x60 0x01 0x60 0x02 0x01 0x00
        let bytecode = Bytes::from(vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
        let (tracer, _report) = run_bytecode(bytecode, OpcodeTracerConfig::default());
        let logs = &tracer.logs;

        assert_eq!(logs.len(), 4, "expected 4 log entries");

        // pc values
        assert_eq!(logs[0].pc, 0, "PUSH1 0x01 pc=0");
        assert_eq!(logs[1].pc, 2, "PUSH1 0x02 pc=2");
        assert_eq!(logs[2].pc, 4, "ADD pc=4");
        assert_eq!(logs[3].pc, 5, "STOP pc=5");

        // opcode names
        assert_eq!(opcode_name(logs[0].op), "PUSH1");
        assert_eq!(opcode_name(logs[1].op), "PUSH1");
        assert_eq!(opcode_name(logs[2].op), "ADD");
        assert_eq!(opcode_name(logs[3].op), "STOP");

        // gas_cost
        assert_eq!(logs[0].gas_cost, 3, "PUSH1 costs 3 gas");
        assert_eq!(logs[1].gas_cost, 3, "PUSH1 costs 3 gas");
        assert_eq!(logs[2].gas_cost, 3, "ADD costs 3 gas");
        assert_eq!(logs[3].gas_cost, 0, "STOP costs 0 gas");

        // depth = 1 (top frame)
        for log in logs {
            assert_eq!(log.depth, 1);
        }

        // Stack after PUSH1 0x01 (before PUSH1 0x02 executes):
        // At step 0 (PUSH1 0x01 pre-step), stack is empty.
        assert_eq!(
            logs[0].stack.as_ref().unwrap(),
            &vec![] as &Vec<U256>,
            "stack empty before first PUSH1"
        );

        // After PUSH1 0x01 executes, stack = [0x1]. Captured at step 1 (pre PUSH1 0x02).
        assert_eq!(
            logs[1].stack.as_ref().unwrap(),
            &vec![U256::from(1u64)],
            "stack=[0x1] before second PUSH1"
        );

        // After PUSH1 0x02 executes, stack = [0x1, 0x2] (bottom-first). Captured at step 2.
        assert_eq!(
            logs[2].stack.as_ref().unwrap(),
            &vec![U256::from(1u64), U256::from(2u64)],
            "stack=[0x1,0x2] before ADD"
        );

        // After ADD executes, stack = [0x3]. Captured at step 3 (pre STOP).
        assert_eq!(
            logs[3].stack.as_ref().unwrap(),
            &vec![U256::from(3u64)],
            "stack=[0x3] before STOP"
        );
    }

    // ── SSTORE storage capture test ───────────────────────────────────────

    /// `PUSH1 0x2a PUSH1 0x01 SSTORE STOP`
    /// SSTORE step: key=0x01, new_value=0x2a.
    /// EIP-3155: at SSTORE step, storage = Some({H256(0x01): H256(0x2a)}) — single entry only.
    /// Steps before SSTORE and STOP emit storage=None.
    #[test]
    fn test_opcode_tracer_sstore_storage_capture() {
        // Bytecode: PUSH1 0x2a, PUSH1 0x01, SSTORE, STOP
        // 0x60 0x2a 0x60 0x01 0x55 0x00
        let bytecode = Bytes::from(vec![0x60, 0x2a, 0x60, 0x01, 0x55, 0x00]);
        let cfg = OpcodeTracerConfig {
            disable_storage: false,
            ..Default::default()
        };
        let (tracer, _report) = run_bytecode(bytecode, cfg);
        let logs = &tracer.logs;

        assert_eq!(
            logs.len(),
            4,
            "expected 4 entries: PUSH1, PUSH1, SSTORE, STOP"
        );

        // PUSH1 0x2a (pc=0)
        assert_eq!(opcode_name(logs[0].op), "PUSH1");
        assert!(
            logs[0].storage.is_none(),
            "PUSH1 step: storage should be None"
        );

        // PUSH1 0x01 (pc=2)
        assert_eq!(opcode_name(logs[1].op), "PUSH1");
        assert!(
            logs[1].storage.is_none(),
            "PUSH1 step: storage should be None"
        );

        // SSTORE (pc=4)
        assert_eq!(opcode_name(logs[2].op), "SSTORE");
        let sstore_storage = logs[2]
            .storage
            .as_ref()
            .expect("SSTORE step must have storage");

        // EIP-3155: single entry {key: 0x01, value: 0x2a}
        assert_eq!(
            sstore_storage.len(),
            1,
            "storage must contain exactly one entry"
        );
        let key = H256::from_low_u64_be(0x01);
        let val = H256::from_low_u64_be(0x2a);
        assert!(
            sstore_storage.contains_key(&key),
            "storage map must contain key 0x01"
        );
        assert_eq!(sstore_storage[&key], val, "storage[0x01] must be 0x2a");

        // STOP (pc=5): storage should be None (not SLOAD/SSTORE)
        assert_eq!(opcode_name(logs[3].op), "STOP");
        assert!(
            logs[3].storage.is_none(),
            "STOP step: storage should be None"
        );
    }
}
