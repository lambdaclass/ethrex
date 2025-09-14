use std::fmt::Display;

use clap::{Parser, ValueEnum};
use ere_dockerized::ErezkVM;
use ethrex_rpc::types::block_identifier::{BlockIdentifier, BlockTag};
use reqwest::Url;
use zkvm_interface::ProverResourceType;

#[derive(Parser)]
pub struct Options {
    #[arg(long, value_enum)]
    pub zkvm: ZKVM,
    #[arg(long, value_enum)]
    pub resource: Resource,
    #[arg(long, value_enum)]
    pub action: Action,
    #[arg(long, value_parser = parse_block_identifier, default_value = "latest", help = "Block identifier (number or tag: earliest, finalized, safe, latest, pending)")]
    pub block: BlockIdentifier,
    #[arg(
        long,
        default_value_t = false,
        help = "Run block after block endlessly"
    )]
    pub endless: bool,
    #[arg(long, value_name = "URL", env = "SLACK_WEBHOOK_URL")]
    pub slack_webhook_url: Option<Url>,
}

fn parse_block_identifier(s: &str) -> Result<BlockIdentifier, String> {
    if let Ok(num) = s.parse::<u64>() {
        Ok(BlockIdentifier::Number(num))
    } else {
        match s {
            "earliest" => Ok(BlockIdentifier::Tag(BlockTag::Earliest)),
            "finalized" => Ok(BlockIdentifier::Tag(BlockTag::Finalized)),
            "safe" => Ok(BlockIdentifier::Tag(BlockTag::Safe)),
            "latest" => Ok(BlockIdentifier::Tag(BlockTag::Latest)),
            "pending" => Ok(BlockIdentifier::Tag(BlockTag::Pending)),
            _ => Err(format!("Invalid block identifier: {s}")),
        }
    }
}

#[expect(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, ValueEnum)]
pub enum ZKVM {
    Jolt,
    Nexus,
    OpenVM,
    Pico,
    Risc0,
    SP1,
    Ziren,
    Zisk,
}

impl Display for ZKVM {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ZKVM::Jolt => "Jolt",
            ZKVM::Nexus => "Nexus",
            ZKVM::OpenVM => "OpenVM",
            ZKVM::Pico => "Pico",
            ZKVM::Risc0 => "RISC0",
            ZKVM::SP1 => "SP1",
            ZKVM::Ziren => "Ziren",
            ZKVM::Zisk => "ZisK",
        };
        write!(f, "{s}")
    }
}

impl From<ZKVM> for ErezkVM {
    fn from(value: ZKVM) -> Self {
        match value {
            ZKVM::Jolt => ErezkVM::Jolt,
            ZKVM::Nexus => ErezkVM::Nexus,
            ZKVM::OpenVM => ErezkVM::OpenVM,
            ZKVM::Pico => ErezkVM::Pico,
            ZKVM::Risc0 => ErezkVM::Risc0,
            ZKVM::SP1 => ErezkVM::SP1,
            ZKVM::Ziren => ErezkVM::Ziren,
            ZKVM::Zisk => ErezkVM::Zisk,
        }
    }
}

#[expect(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, ValueEnum)]
pub enum Resource {
    CPU,
    GPU,
}

impl Display for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Resource::CPU => "CPU",
            Resource::GPU => "GPU",
        };
        write!(f, "{s}")
    }
}

impl From<Resource> for ProverResourceType {
    fn from(value: Resource) -> Self {
        match value {
            Resource::CPU => ProverResourceType::Cpu,
            Resource::GPU => ProverResourceType::Gpu,
        }
    }
}

#[derive(Clone, Debug, ValueEnum)]
pub enum Action {
    Execute,
    Prove,
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Action::Execute => "Execute",
            Action::Prove => "Prove",
        };
        write!(f, "{s}")
    }
}
