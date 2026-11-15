//! The local EVM over a pinned store.
//!
//! A run reads the store through [`PartialDb`], which records every key it reads and every
//! key it lacks. A missing value reads as empty for the rest of that run only: the result is
//! withheld whenever a miss was recorded, so an invented value never leaves this module.
//! The caller proves the misses at the same anchor and runs again; round trips scale with
//! dependency depth, not key count.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use ethrex_common::types::{
    AccountState, ChainConfig, Code, CodeMetadata, Fork, GenericTransaction,
};
use ethrex_common::{Address, H256, U256};
use ethrex_crypto::NativeCrypto;
use ethrex_vm::backends::ExecutionGasCap;
use ethrex_vm::backends::levm::get_max_allowed_gas_limit;
use ethrex_vm::{Evm, EvmError, ExecutionResult, VmDatabase};

use crate::state::{Key, Miss, Store};

/// Free gas given at the beginning of a call, added to the optimistic estimate.
const CALL_STIPEND: u64 = 2_300;
/// Relative error at which gas estimation stops bisecting.
const ESTIMATE_ERROR_RATIO: f64 = 0.015;

/// What a run produced, with the keys it read.
#[derive(Clone, Debug)]
pub struct Executed<T> {
    /// The result.
    pub result: T,
    /// Every held key the run read. Learned by the node so they stay warm.
    pub read: HashSet<Key>,
}

/// The keys a run lacked, in discovery order, deduplicated. Non-empty by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misses {
    first: Miss,
    rest: Vec<Miss>,
}

impl Misses {
    /// A single miss.
    pub const fn one(miss: Miss) -> Self {
        Self {
            first: miss,
            rest: Vec::new(),
        }
    }

    fn from_vec(mut misses: Vec<Miss>) -> Option<Self> {
        if misses.is_empty() {
            return None;
        }
        let first = misses.remove(0);
        Some(Self {
            first,
            rest: misses,
        })
    }

    /// The misses in discovery order.
    pub fn iter(&self) -> impl Iterator<Item = Miss> + '_ {
        std::iter::once(self.first).chain(self.rest.iter().copied())
    }

    /// How many keys were missing.
    pub fn len(&self) -> usize {
        self.rest.len().saturating_add(1)
    }

    /// Never true: the type holds at least one miss.
    pub fn is_empty(&self) -> bool {
        false
    }
}

/// Why a gas estimate could not be given.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EstimateError {
    /// The call reverts even with the most gas allowed.
    #[error("execution reverted")]
    Reverted {
        /// The revert data.
        data: bytes::Bytes,
    },
    /// The call halts even with the most gas allowed.
    #[error("execution halted: {reason}")]
    Halted {
        /// The halt reason.
        reason: String,
    },
    /// The transaction was refused before execution (funds, nonce, intrinsic gas).
    #[error("{0}")]
    Refused(String),
}

#[derive(Default)]
struct Log {
    misses: Vec<Miss>,
    read: HashSet<Key>,
}

impl Log {
    fn miss(&mut self, miss: Miss) {
        if !self.misses.contains(&miss) {
            self.misses.push(miss);
        }
    }
}

/// The store as the EVM reads it. Clone shares the log.
#[derive(Clone)]
pub struct PartialDb {
    store: Arc<Store>,
    chain_config: ChainConfig,
    log: Arc<Mutex<Log>>,
}

impl PartialDb {
    /// A database over `store`.
    pub fn new(store: Arc<Store>, chain_config: ChainConfig) -> Self {
        Self {
            store,
            chain_config,
            log: Arc::new(Mutex::new(Log::default())),
        }
    }

    fn with_log<R>(&self, f: impl FnOnce(&mut Log) -> R) -> R {
        let mut guard = match self.log.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// Takes the recorded keys. `Err` carries the misses; `Ok` the keys read.
    fn finish(&self) -> Result<HashSet<Key>, Misses> {
        self.with_log(|log| {
            let misses = std::mem::take(&mut log.misses);
            let read = std::mem::take(&mut log.read);
            match Misses::from_vec(misses) {
                Some(misses) => Err(misses),
                None => Ok(read),
            }
        })
    }
}

impl VmDatabase for PartialDb {
    fn get_account_state(&self, address: Address) -> Result<Option<AccountState>, EvmError> {
        match self.store.account(address) {
            Ok(account) => {
                self.with_log(|log| log.read.insert(Key::Account(address)));
                Ok(account.map(|a| a.to_account_state()))
            }
            Err(miss) => {
                self.with_log(|log| log.miss(miss));
                Ok(None)
            }
        }
    }

    fn get_storage_slot(&self, address: Address, key: H256) -> Result<Option<U256>, EvmError> {
        let slot = U256::from_big_endian(key.as_bytes());
        match self.store.slot(address, slot) {
            Ok(value) => {
                self.with_log(|log| log.read.insert(Key::Slot(address, slot)));
                Ok(Some(value))
            }
            // One storage proof carries its account proof, so a missing account is asked
            // for as the slot.
            Err(Miss::Account(_) | Miss::Storage(..)) => {
                self.with_log(|log| log.miss(Miss::Storage(address, slot)));
                Ok(None)
            }
            Err(miss) => {
                self.with_log(|log| log.miss(miss));
                Ok(None)
            }
        }
    }

    fn get_block_hash(&self, block_number: u64) -> Result<H256, EvmError> {
        match self.store.block_hash(block_number) {
            Ok(hash) => Ok(hash),
            Err(miss) => {
                self.with_log(|log| log.miss(miss));
                Ok(H256::zero())
            }
        }
    }

    fn get_chain_config(&self) -> Result<ChainConfig, EvmError> {
        Ok(self.chain_config)
    }

    fn get_account_code(&self, code_hash: H256) -> Result<Code, EvmError> {
        match self.store.code(code_hash) {
            Ok(code) => Ok(code),
            Err(miss) => {
                self.with_log(|log| log.miss(miss));
                Ok(Code::from_bytecode_unchecked(
                    bytes::Bytes::new(),
                    code_hash,
                ))
            }
        }
    }

    fn get_code_metadata(&self, code_hash: H256) -> Result<CodeMetadata, EvmError> {
        let code = self.get_account_code(code_hash)?;
        Ok(CodeMetadata {
            length: u64::try_from(code.len()).unwrap_or(u64::MAX),
        })
    }
}

/// A request for the local EVM, bound to the anchor it runs at.
pub struct Runner {
    db: PartialDb,
    gas_cap: u64,
}

impl Runner {
    /// A runner over `store` for the chain `chain_config` describes. `gas_cap` bounds the
    /// gas a call may use when the request names none.
    pub fn new(store: Arc<Store>, chain_config: ChainConfig, gas_cap: u64) -> Self {
        Self {
            db: PartialDb::new(store, chain_config),
            gas_cap,
        }
    }

    fn fork(&self) -> Fork {
        self.db
            .chain_config
            .fork(self.db.store.anchor().header().timestamp)
    }

    /// The most gas a call may use: the fork's per-transaction cap, the block gas limit
    /// and the configured cap, whichever is lowest.
    fn ceiling(&self) -> u64 {
        let header = self.db.store.anchor().header();
        get_max_allowed_gas_limit(header.gas_limit, self.fork()).min(self.gas_cap)
    }

    /// Caps `gas` so the sender can pay for it, when the request is priced. A sender the
    /// store lacks is recorded as a miss by the read and leaves the gas unchanged.
    fn affordable_gas(&self, tx: &GenericTransaction, gas: u64) -> u64 {
        let header = self.db.store.anchor().header();
        let base_fee = header.base_fee_per_gas.unwrap_or_default();
        let price = if !tx.gas_price.is_zero() {
            tx.gas_price
        } else if let Some(max_fee) = tx.max_fee_per_gas {
            let tip = tx
                .max_priority_fee_per_gas
                .unwrap_or(max_fee)
                .min(max_fee.saturating_sub(base_fee));
            U256::from(base_fee.saturating_add(tip))
        } else {
            return gas;
        };
        if price.is_zero() {
            return gas;
        }
        let Ok(Some(account)) = self.db.get_account_state(tx.from) else {
            return gas;
        };
        let spendable = account.balance.saturating_sub(tx.value);
        let affordable = spendable / price;
        if affordable < U256::from(gas) {
            affordable.try_into().unwrap_or(gas)
        } else {
            gas
        }
    }

    fn simulate(
        &self,
        tx: &GenericTransaction,
        cap: ExecutionGasCap,
    ) -> Result<ExecutionResult, EvmError> {
        let header = self.db.store.anchor().header();
        let mut evm = Evm::new_for_l1(self.db.clone(), Arc::new(NativeCrypto));
        evm.simulate_tx_from_generic(tx, header, cap)
    }

    /// Runs `tx` as `eth_call` does: at the anchor, with the anchor block's environment,
    /// without charging the sender.
    pub fn call(
        &self,
        tx: &GenericTransaction,
    ) -> Result<Executed<Result<ExecutionResult, EvmError>>, Misses> {
        let mut tx = tx.clone();
        let gas = tx.gas.unwrap_or_else(|| self.ceiling());
        tx.gas = Some(self.affordable_gas(&tx, gas));
        let result = self.simulate(&tx, ExecutionGasCap::Lifted);
        let read = self.db.finish()?;
        Ok(Executed { result, read })
    }

    /// Estimates the gas `tx` needs: a run at the ceiling settles refusals and reverts,
    /// then an optimistic probe and a bisection narrow the limit to 1.5%.
    pub fn estimate_gas(
        &self,
        tx: &GenericTransaction,
    ) -> Result<Executed<Result<u64, EstimateError>>, Misses> {
        let mut tx = tx.clone();
        let requested = tx.gas;
        let mut highest = requested.map_or(self.ceiling(), |g| g.min(self.ceiling()));
        highest = self.affordable_gas(&tx, highest);
        tx.gas = Some(highest);

        let first = match self.simulate(&tx, ExecutionGasCap::Enforced) {
            Ok(result) => result,
            Err(err) => {
                let read = self.db.finish()?;
                return Ok(Executed {
                    result: Err(EstimateError::Refused(err.to_string())),
                    read,
                });
            }
        };
        let (gas_used, gas_refunded) = match first {
            ExecutionResult::Success {
                gas_used,
                gas_refunded,
                ..
            } => (gas_used, gas_refunded),
            ExecutionResult::Revert { output, .. } => {
                let read = self.db.finish()?;
                return Ok(Executed {
                    result: Err(EstimateError::Reverted { data: output }),
                    read,
                });
            }
            ExecutionResult::Halt { reason, .. } => {
                let read = self.db.finish()?;
                return Ok(Executed {
                    result: Err(EstimateError::Halted { reason }),
                    read,
                });
            }
        };

        // A run at exactly the gas it used succeeds for plain calls; it fails when a
        // 63/64ths rule or a refund hid the real need, and the bisection takes over.
        let mut lowest = gas_used.saturating_sub(1);
        if self.succeeds_with(&mut tx, gas_used) {
            let read = self.db.finish()?;
            return Ok(Executed {
                result: Ok(gas_used),
                read,
            });
        }
        let optimistic = gas_used
            .saturating_add(gas_refunded)
            .saturating_add(CALL_STIPEND)
            .saturating_mul(64)
            / 63;
        if optimistic > lowest && optimistic < highest {
            if self.succeeds_with(&mut tx, optimistic) {
                highest = optimistic;
            } else {
                lowest = optimistic;
            }
        }
        while lowest.saturating_add(1) < highest {
            let gap = highest.saturating_sub(lowest);
            if (gap as f64) / (highest as f64) < ESTIMATE_ERROR_RATIO {
                break;
            }
            let mid = lowest.saturating_add(gap / 2);
            if self.succeeds_with(&mut tx, mid) {
                highest = mid;
            } else {
                lowest = mid;
            }
        }
        let read = self.db.finish()?;
        Ok(Executed {
            result: Ok(highest),
            read,
        })
    }

    fn succeeds_with(&self, tx: &mut GenericTransaction, gas: u64) -> bool {
        tx.gas = Some(gas);
        matches!(
            self.simulate(tx, ExecutionGasCap::Enforced),
            Ok(ExecutionResult::Success { .. })
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::state::{Key, State};
    use crate::testing::{World, deploy_code, eth, reader_runtime};
    use crate::working::{WORKING_SET_CAP, WorkingSet};
    use ethrex_common::evm::calculate_create_address;
    use ethrex_common::types::{Genesis, TxKind};

    fn chain_config() -> ChainConfig {
        let genesis: Genesis =
            serde_json::from_str(include_str!("../../../fixtures/genesis/l1-bal.json")).unwrap();
        genesis.config
    }

    const GAS_CAP: u64 = 50_000_000;

    /// Proves every miss at the store's anchor and inserts it, as the node does.
    async fn resolve<T>(
        world: &World,
        store: &mut Arc<Store>,
        step: impl Fn(Arc<Store>) -> Result<Executed<T>, Misses>,
    ) -> (Executed<T>, usize) {
        for round in 0..64 {
            match step(Arc::clone(store)) {
                Ok(executed) => return (executed, round),
                Err(misses) => {
                    let anchor = store.anchor().clone();
                    let inner = Arc::make_mut(store);
                    for miss in misses.iter() {
                        match miss {
                            Miss::Account(address) => {
                                inner
                                    .insert(&world.prove(&anchor, Key::Account(address)).await)
                                    .unwrap();
                            }
                            Miss::Storage(address, slot) => {
                                inner
                                    .insert(&world.prove(&anchor, Key::Slot(address, slot)).await)
                                    .unwrap();
                            }
                            Miss::Code(hash) => inner.insert(&world.prove_code(hash)).unwrap(),
                            Miss::BlockHash(number) => {
                                panic!("unexpected block hash miss {number}")
                            }
                        }
                    }
                }
            }
        }
        panic!("discovery did not converge");
    }

    #[tokio::test]
    async fn call_discovers_its_keys_and_converges_to_the_world_value() {
        let mut world = World::new().await;
        let alice = world.signer(0).address;
        // Deploy a reader whose slot 5 holds 55 (written by its init code).
        let initcode = deploy_code(&[(5, 55)], &reader_runtime());
        world.mine(vec![world.deploy(0, 0, initcode)]).await;
        assert!(world.rejected.is_empty(), "{:?}", world.rejected);
        let reader = calculate_create_address(alice, 0);

        let head = world.verified(world.head());
        let mut store = Arc::new(Store::new(head));
        let tx = GenericTransaction {
            from: alice,
            to: TxKind::Call(reader),
            input: bytes::Bytes::from(U256::from(5).to_big_endian().to_vec()),
            ..Default::default()
        };
        let (executed, rounds) = resolve(&world, &mut store, |store| {
            Runner::new(store, chain_config(), GAS_CAP).call(&tx)
        })
        .await;
        assert!((1..=4).contains(&rounds), "took {rounds} rounds");
        let ExecutionResult::Success { output, .. } = executed.result.unwrap() else {
            panic!("call failed");
        };
        assert_eq!(U256::from_big_endian(&output), U256::from(55));
        assert!(executed.read.contains(&Key::Account(reader)));
        assert!(executed.read.contains(&Key::Slot(reader, U256::from(5))));
        assert!(executed.read.contains(&Key::Account(alice)));
        assert!(store.holds(Key::Slot(reader, U256::from(5))));
    }

    #[tokio::test]
    async fn a_miss_withholds_even_a_successful_looking_result() {
        let world = World::new().await;
        let alice = world.signer(0).address;
        let head = world.verified(world.head());
        let store = Arc::new(Store::new(head));
        let tx = GenericTransaction {
            from: alice,
            to: TxKind::Call(Address::repeat_byte(0xaa)),
            ..Default::default()
        };
        let result = Runner::new(store, chain_config(), GAS_CAP).call(&tx);
        let misses = result.expect_err("an empty store misses everything");
        let missed: Vec<Miss> = misses.iter().collect();
        assert!(missed.contains(&Miss::Account(alice)), "{missed:?}");
        assert!(
            missed.contains(&Miss::Account(Address::repeat_byte(0xaa))),
            "{missed:?}"
        );
    }

    #[tokio::test]
    async fn estimate_gas_is_enough_and_tight() {
        let world = World::new().await;
        let alice = world.signer(0).address;
        let bob = world.signer(1).address;
        let head = world.verified(world.head());
        let mut store = Arc::new(Store::new(head));
        let tx = GenericTransaction {
            from: alice,
            to: TxKind::Call(bob),
            value: eth(1),
            ..Default::default()
        };
        let (executed, _) = resolve(&world, &mut store, |store| {
            Runner::new(store, chain_config(), GAS_CAP).estimate_gas(&tx)
        })
        .await;
        let estimate = executed.result.unwrap();
        assert!((12_000..=60_000).contains(&estimate), "estimate {estimate}");

        let runner = Runner::new(Arc::clone(&store), chain_config(), GAS_CAP);
        let mut exact = tx.clone();
        exact.gas = Some(estimate);
        assert!(matches!(
            runner.call(&exact).unwrap().result,
            Ok(ExecutionResult::Success { .. })
        ));
        let mut short = tx.clone();
        short.gas = Some(estimate * 9 / 10);
        assert!(!matches!(
            runner.call(&short).unwrap().result,
            Ok(ExecutionResult::Success { .. })
        ));
    }

    #[tokio::test]
    async fn estimate_reports_reverts_and_refusals() {
        let world = World::new().await;
        let alice = world.signer(0).address;
        let head = world.verified(world.head());
        let mut store = Arc::new(Store::new(head));
        // A plain transfer worth more than alice holds is refused.
        let tx = GenericTransaction {
            from: alice,
            to: TxKind::Call(Address::repeat_byte(0xaa)),
            value: eth(2_000_000_000),
            ..Default::default()
        };
        let (executed, _) = resolve(&world, &mut store, |store| {
            Runner::new(store, chain_config(), GAS_CAP).estimate_gas(&tx)
        })
        .await;
        assert!(
            matches!(executed.result, Err(EstimateError::Refused(_))),
            "{:?}",
            executed.result
        );
    }

    #[tokio::test]
    async fn state_at_a_retained_block_serves_old_values() {
        let mut world = World::new().await;
        let alice = world.signer(0).address;
        let bob = world.signer(1).address;
        let anchor = world.verified(world.head());
        let working = WorkingSet::new(WORKING_SET_CAP, [Key::Account(bob)], Vec::new());
        let mut state = State::new(anchor.clone(), working);
        let fact = world.prove(&anchor, Key::Account(bob)).await;
        state.reanchor(anchor.clone(), &[fact]).unwrap();
        let before = state.store().account(bob).unwrap().unwrap().balance;

        let hash = world.mine(vec![world.transfer(0, 0, bob, eth(1))]).await;
        let block = crate::verify::bal(world.verified(hash), world.bal(hash)).unwrap();
        state.advance(block).unwrap();
        assert_eq!(
            state.store().account(bob).unwrap().unwrap().balance,
            before + eth(1)
        );
        let old = state.snapshot_at(anchor.hash()).unwrap();
        assert_eq!(old.account(bob).unwrap().unwrap().balance, before);
        let _ = alice;
    }
}
