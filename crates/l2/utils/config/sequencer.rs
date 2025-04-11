use serde::Deserialize;

use super::{
    block_producer::BlockProducerConfig, committer::CommitterConfig, deployer::DeployerConfig,
    eth::EthConfig, l1_watcher::L1WatcherConfig, prover_server::ProverServerConfig, L2Config,
};

#[derive(Deserialize, Debug)]
pub struct SequencerConfig {
    deployer: DeployerConfig,
    eth: EthConfig,
    watcher: L1WatcherConfig,
    proposer: BlockProducerConfig,
    committer: CommitterConfig,
    prover_server: ProverServerConfig,
}

impl L2Config for SequencerConfig {
    const PREFIX: &str = "";

    fn to_env(&self) -> String {
        let mut env_representation = String::new();

        env_representation.push_str(&self.deployer.to_env());
        env_representation.push_str(&self.eth.to_env());
        env_representation.push_str(&self.watcher.to_env());
        env_representation.push_str(&self.proposer.to_env());
        env_representation.push_str(&self.committer.to_env());
        env_representation.push_str(&self.prover_server.to_env());

        env_representation
    }
}
