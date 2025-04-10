use ethrex_common::Address;
use ethrex_l2_sdk::secret_key_deserializer;
use secp256k1::SecretKey;
use serde::Deserialize;

pub const DEPLOYER_PREFIX: &str = "DEPLOYER_";

#[derive(Deserialize, Debug)]
pub struct DeployerConfig {
    l1_address: Address,
    #[serde(deserialize_with = "secret_key_deserializer")]
    l1_private_key: SecretKey,
    risc0_contract_verifier: Address,
    sp1_contract_verifier: Address,
    pico_contract_verifier: Address,
    sp1_deploy_verifier: bool,
    pico_deploy_verifier: bool,
    salt_is_zero: bool,
}

impl DeployerConfig {
    pub fn to_env(&self) -> String {
        format!(
            "
{DEPLOYER_PREFIX}L1_ADDRESS=0x{:#x}
{DEPLOYER_PREFIX}L1_PRIVATE_KEY=0x{}
{DEPLOYER_PREFIX}RISC0_CONTRACT_VERIFIER=0x{:#x}
{DEPLOYER_PREFIX}SP1_CONTRACT_VERIFIER=0x{:#x}
{DEPLOYER_PREFIX}PICO_CONTRACT_VERIFIER=0x{:#x}
{DEPLOYER_PREFIX}SP1_DEPLOY_VERIFIER={}
{DEPLOYER_PREFIX}PICO_DEPLOY_VERIFIER={}
{DEPLOYER_PREFIX}SALT_IS_ZERO={}
",
            self.l1_address,
            self.l1_private_key.display_secret(),
            self.risc0_contract_verifier,
            self.sp1_contract_verifier,
            self.pico_contract_verifier,
            self.sp1_deploy_verifier,
            self.pico_deploy_verifier,
            self.salt_is_zero
        )
    }
}
