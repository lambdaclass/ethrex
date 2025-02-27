use ethrex_common::{types::Receipt, U256};
use ethrex_vm::{db::EvmState, ExecutionResult};

use crate::utils::transaction::Transaction;
use std::{fmt::Debug, ops::Deref, sync::Arc};

#[derive(Clone, Debug)]
pub struct SimulatedTx {
    /// original tx
    pub tx: Arc<Transaction>,
    pub result: ExecutionResult,
    pub state: EvmState,
    /// Coinbase balance diff, after_sim - before_sim
    pub payment: U256,
    /// Cache the depositor account prior to the state transition for the deposit nonce.
    /// Note: this is only used for deposit transactions.
    pub deposit_nonce: Option<u64>,
}

impl SimulatedTx {
    pub fn new(
        tx: Arc<Transaction>,
        result: ExecutionResult,
        state: EvmState,
        payment: U256,
        deposit_nonce: Option<u64>,
    ) -> Self {
        Self {
            tx,
            result,
            state,
            payment,
            deposit_nonce,
        }
    }

    pub fn receipt(&self, cumulative_gas_used: u64) -> Receipt {
        Receipt::new(
            self.tx.tx_type(),
            self.result.is_success(),
            cumulative_gas_used,
            self.result.logs(),
        )
    }

    pub fn gas_used(&self) -> u64 {
        self.result.gas_used()
    }
}

impl AsRef<ExecutionResult> for SimulatedTx {
    fn as_ref(&self) -> &ExecutionResult {
        &self.result
    }
}
impl Deref for SimulatedTx {
    type Target = Arc<Transaction>;

    fn deref(&self) -> &Self::Target {
        &self.tx
    }
}

// impl TransactionSenderInfo for SimulatedTx {
//     fn sender(&self) -> Address {
//         self.sender
//     }

//     fn nonce(&self) -> u64 {
//         self.tx.nonce()
//     }
// }
