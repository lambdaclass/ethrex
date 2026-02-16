pub mod batches;
pub mod blocks;
pub mod chain_status;
pub mod l1_to_l2_messages;
pub mod l2_to_l1_messages;
pub mod mempool;
pub mod node_status;
pub mod rich_accounts;
pub mod tabs;

pub use batches::BatchesTable;
pub use blocks::BlocksTable;
pub use chain_status::GlobalChainStatusTable;
pub use l1_to_l2_messages::L1ToL2MessagesTable;
pub use l2_to_l1_messages::L2ToL1MessagesTable;
pub use mempool::MempoolTable;
pub use node_status::NodeStatusTable;

pub const ETHREX_LOGO: &str = r#"
███████╗████████╗██╗░░██╗██████╗░███████╗██╗░░██╗
██╔════╝╚══██╔══╝██║░░██║██╔══██╗██╔════╝╚██╗██╔╝
█████╗░░░░░██║░░░███████║██████╔╝█████╗░░░╚███╔╝░
██╔══╝░░░░░██║░░░██╔══██║██╔══██╗██╔══╝░░░██╔██╗░
███████╗░░░██║░░░██║░░██║██║░░██║███████╗██╔╝╚██╗
╚══════╝░░░╚═╝░░░╚═╝░░╚═╝╚═╝░░╚═╝╚══════╝╚═╝░░╚═╝"#;

pub const HASH_LENGTH_IN_DIGITS: u16 = 66; // 64 hex characters + 2 for "0x" prefix
pub const ADDRESS_LENGTH_IN_DIGITS: u16 = 42; // 40 hex characters + 2 for "0x" prefix
pub const NUMBER_LENGTH_IN_DIGITS: u16 = 9; // 1e8
pub const TX_NUMBER_LENGTH_IN_DIGITS: u16 = 4;
pub const GAS_USED_LENGTH_IN_DIGITS: u16 = 8; // 1e7
pub const BLOCK_SIZE_LENGTH_IN_DIGITS: u16 = 6; // 1e6

pub const LATEST_BLOCK_STATUS_TABLE_LENGTH_IN_DIGITS: u16 = NUMBER_LENGTH_IN_DIGITS
    + TX_NUMBER_LENGTH_IN_DIGITS
    + HASH_LENGTH_IN_DIGITS
    + ADDRESS_LENGTH_IN_DIGITS
    + GAS_USED_LENGTH_IN_DIGITS
    + GAS_USED_LENGTH_IN_DIGITS
    + BLOCK_SIZE_LENGTH_IN_DIGITS;
