pub mod call_frame;
pub mod constants;
pub mod db;
pub mod debug;
pub mod environment;
pub mod errors;
pub mod execution_handlers;
pub mod gas_cost;
pub mod hooks;
pub mod memory;
pub mod opcode_handlers;
pub mod opcodes;
pub mod precompiles;
pub mod tracing;
pub mod utils;
pub mod vm;
pub use environment::*;
use ethrex_common::Address;
pub mod l2_precompiles;

use std::str::FromStr;

lazy_static::lazy_static! {
    pub static ref PROBLEMATIC_ADDRESS: Address = Address::from_str("0x455e5aa18469bc6ccef49594645666c587a3a71b").unwrap();
}
