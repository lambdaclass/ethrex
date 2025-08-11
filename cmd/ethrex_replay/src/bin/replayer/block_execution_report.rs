use std::fmt::Display;

use ethrex_common::types::Block;
use ethrex_replay::networks::Network;

pub struct BlockExecutionReport {
    pub network: Network,
    pub number: u64,
    pub gas: u64,
    pub txs: u64,
    pub execution_result: String,
}

impl BlockExecutionReport {
    pub fn new(block: Block, network: Network, execution_result: String) -> Self {
        Self {
            network,
            number: block.header.number,
            gas: block.header.gas_used,
            txs: block.body.transactions.len() as u64,
            execution_result,
        }
    }
}

impl Display for BlockExecutionReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Network::PublicNetwork(_) = self.network {
            write!(
                f,
                "[{network}] Block #{number}, Gas Used: {gas}, Tx Count: {txs}, Execution Result: {execution_result} | https://{network}.etherscan.io/block/{number}",
                network = self.network,
                number = self.number,
                gas = self.gas,
                txs = self.txs,
                execution_result = self.execution_result,
            )
        } else {
            write!(
                f,
                "[{}] Block #{}, Gas Used: {}, Tx Count: {}, Execution Result: {}",
                self.network, self.number, self.gas, self.txs, self.execution_result
            )
        }
    }
}
