use super::{CommandDef, ParamDef, ParamType};

const NO_PARAMS: &[ParamDef] = &[];

const ADDR_BLOCK: &[ParamDef] = &[
    ParamDef {
        name: "address",
        param_type: ParamType::Address,
        required: true,
        default_value: None,
        description: "Account address",
    },
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: false,
        default_value: Some("latest"),
        description: "Block identifier",
    },
];

const BLOCK_FULLTXS: &[ParamDef] = &[
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: true,
        default_value: None,
        description: "Block identifier",
    },
    ParamDef {
        name: "full_txs",
        param_type: ParamType::Bool,
        required: false,
        default_value: Some("false"),
        description: "Return full transactions if true",
    },
];

const HASH_FULLTXS: &[ParamDef] = &[
    ParamDef {
        name: "hash",
        param_type: ParamType::Hash,
        required: true,
        default_value: None,
        description: "Block hash",
    },
    ParamDef {
        name: "full_txs",
        param_type: ParamType::Bool,
        required: false,
        default_value: Some("false"),
        description: "Return full transactions if true",
    },
];

const BLOCK_ONLY: &[ParamDef] = &[ParamDef {
    name: "block",
    param_type: ParamType::BlockId,
    required: true,
    default_value: None,
    description: "Block identifier",
}];

const HASH_ONLY: &[ParamDef] = &[ParamDef {
    name: "hash",
    param_type: ParamType::Hash,
    required: true,
    default_value: None,
    description: "Transaction hash",
}];

const TX_OBJECT_BLOCK: &[ParamDef] = &[
    ParamDef {
        name: "tx_object",
        param_type: ParamType::Object,
        required: true,
        default_value: None,
        description: "Transaction call object",
    },
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: false,
        default_value: Some("latest"),
        description: "Block identifier",
    },
];

const FILTER_OBJECT: &[ParamDef] = &[ParamDef {
    name: "filter",
    param_type: ParamType::Object,
    required: true,
    default_value: None,
    description: "Filter object",
}];

const FILTER_ID: &[ParamDef] = &[ParamDef {
    name: "filter_id",
    param_type: ParamType::Uint,
    required: true,
    default_value: None,
    description: "Filter identifier",
}];

const GET_STORAGE_AT: &[ParamDef] = &[
    ParamDef {
        name: "address",
        param_type: ParamType::Address,
        required: true,
        default_value: None,
        description: "Account address",
    },
    ParamDef {
        name: "slot",
        param_type: ParamType::Hash,
        required: true,
        default_value: None,
        description: "Storage slot",
    },
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: false,
        default_value: Some("latest"),
        description: "Block identifier",
    },
];

const BLOCK_INDEX: &[ParamDef] = &[
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: true,
        default_value: None,
        description: "Block identifier",
    },
    ParamDef {
        name: "index",
        param_type: ParamType::Uint,
        required: true,
        default_value: None,
        description: "Transaction index",
    },
];

const HASH_INDEX: &[ParamDef] = &[
    ParamDef {
        name: "hash",
        param_type: ParamType::Hash,
        required: true,
        default_value: None,
        description: "Block hash",
    },
    ParamDef {
        name: "index",
        param_type: ParamType::Uint,
        required: true,
        default_value: None,
        description: "Transaction index",
    },
];

const SEND_RAW_TX: &[ParamDef] = &[ParamDef {
    name: "data",
    param_type: ParamType::HexData,
    required: true,
    default_value: None,
    description: "Signed transaction data",
}];

const FEE_HISTORY: &[ParamDef] = &[
    ParamDef {
        name: "block_count",
        param_type: ParamType::Uint,
        required: true,
        default_value: None,
        description: "Number of blocks",
    },
    ParamDef {
        name: "newest_block",
        param_type: ParamType::BlockId,
        required: true,
        default_value: None,
        description: "Newest block",
    },
    ParamDef {
        name: "reward_percentiles",
        param_type: ParamType::Array,
        required: false,
        default_value: None,
        description: "Reward percentile values",
    },
];

const BLOCK_HASH_ONLY: &[ParamDef] = &[ParamDef {
    name: "hash",
    param_type: ParamType::Hash,
    required: true,
    default_value: None,
    description: "Block hash",
}];

const GET_PROOF: &[ParamDef] = &[
    ParamDef {
        name: "address",
        param_type: ParamType::Address,
        required: true,
        default_value: None,
        description: "Account address",
    },
    ParamDef {
        name: "storage_keys",
        param_type: ParamType::Array,
        required: true,
        default_value: None,
        description: "Storage keys to prove",
    },
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: false,
        default_value: Some("latest"),
        description: "Block identifier",
    },
];

pub fn commands() -> Vec<CommandDef> {
    vec![
        CommandDef {
            namespace: "eth",
            name: "blockNumber",
            rpc_method: "eth_blockNumber",
            params: NO_PARAMS,
            description: "Returns the current block number",
        },
        CommandDef {
            namespace: "eth",
            name: "chainId",
            rpc_method: "eth_chainId",
            params: NO_PARAMS,
            description: "Returns the chain ID",
        },
        CommandDef {
            namespace: "eth",
            name: "syncing",
            rpc_method: "eth_syncing",
            params: NO_PARAMS,
            description: "Returns syncing status or false",
        },
        CommandDef {
            namespace: "eth",
            name: "gasPrice",
            rpc_method: "eth_gasPrice",
            params: NO_PARAMS,
            description: "Returns the current gas price in wei",
        },
        CommandDef {
            namespace: "eth",
            name: "maxPriorityFeePerGas",
            rpc_method: "eth_maxPriorityFeePerGas",
            params: NO_PARAMS,
            description: "Returns the current max priority fee per gas",
        },
        CommandDef {
            namespace: "eth",
            name: "blobBaseFee",
            rpc_method: "eth_blobBaseFee",
            params: NO_PARAMS,
            description: "Returns the current blob base fee",
        },
        CommandDef {
            namespace: "eth",
            name: "accounts",
            rpc_method: "eth_accounts",
            params: NO_PARAMS,
            description: "Returns list of addresses owned by the client",
        },
        CommandDef {
            namespace: "eth",
            name: "getBalance",
            rpc_method: "eth_getBalance",
            params: ADDR_BLOCK,
            description: "Returns the balance of an account",
        },
        CommandDef {
            namespace: "eth",
            name: "getCode",
            rpc_method: "eth_getCode",
            params: ADDR_BLOCK,
            description: "Returns the code at an address",
        },
        CommandDef {
            namespace: "eth",
            name: "getStorageAt",
            rpc_method: "eth_getStorageAt",
            params: GET_STORAGE_AT,
            description: "Returns the value at a storage slot",
        },
        CommandDef {
            namespace: "eth",
            name: "getTransactionCount",
            rpc_method: "eth_getTransactionCount",
            params: ADDR_BLOCK,
            description: "Returns the number of transactions sent from an address",
        },
        CommandDef {
            namespace: "eth",
            name: "getBlockByNumber",
            rpc_method: "eth_getBlockByNumber",
            params: BLOCK_FULLTXS,
            description: "Returns a block by its number",
        },
        CommandDef {
            namespace: "eth",
            name: "getBlockByHash",
            rpc_method: "eth_getBlockByHash",
            params: HASH_FULLTXS,
            description: "Returns a block by its hash",
        },
        CommandDef {
            namespace: "eth",
            name: "getBlockTransactionCountByNumber",
            rpc_method: "eth_getBlockTransactionCountByNumber",
            params: BLOCK_ONLY,
            description: "Returns transaction count in a block by number",
        },
        CommandDef {
            namespace: "eth",
            name: "getBlockTransactionCountByHash",
            rpc_method: "eth_getBlockTransactionCountByHash",
            params: BLOCK_HASH_ONLY,
            description: "Returns transaction count in a block by hash",
        },
        CommandDef {
            namespace: "eth",
            name: "getTransactionByBlockNumberAndIndex",
            rpc_method: "eth_getTransactionByBlockNumberAndIndex",
            params: BLOCK_INDEX,
            description: "Returns a transaction by block number and index",
        },
        CommandDef {
            namespace: "eth",
            name: "getTransactionByBlockHashAndIndex",
            rpc_method: "eth_getTransactionByBlockHashAndIndex",
            params: HASH_INDEX,
            description: "Returns a transaction by block hash and index",
        },
        CommandDef {
            namespace: "eth",
            name: "getTransactionByHash",
            rpc_method: "eth_getTransactionByHash",
            params: HASH_ONLY,
            description: "Returns a transaction by its hash",
        },
        CommandDef {
            namespace: "eth",
            name: "getTransactionReceipt",
            rpc_method: "eth_getTransactionReceipt",
            params: HASH_ONLY,
            description: "Returns the receipt of a transaction",
        },
        CommandDef {
            namespace: "eth",
            name: "getBlockReceipts",
            rpc_method: "eth_getBlockReceipts",
            params: BLOCK_ONLY,
            description: "Returns all receipts for a block",
        },
        CommandDef {
            namespace: "eth",
            name: "call",
            rpc_method: "eth_call",
            params: TX_OBJECT_BLOCK,
            description: "Executes a call without creating a transaction",
        },
        CommandDef {
            namespace: "eth",
            name: "estimateGas",
            rpc_method: "eth_estimateGas",
            params: TX_OBJECT_BLOCK,
            description: "Estimates gas needed for a transaction",
        },
        CommandDef {
            namespace: "eth",
            name: "sendRawTransaction",
            rpc_method: "eth_sendRawTransaction",
            params: SEND_RAW_TX,
            description: "Submits a signed transaction",
        },
        CommandDef {
            namespace: "eth",
            name: "feeHistory",
            rpc_method: "eth_feeHistory",
            params: FEE_HISTORY,
            description: "Returns fee history for a range of blocks",
        },
        CommandDef {
            namespace: "eth",
            name: "getLogs",
            rpc_method: "eth_getLogs",
            params: FILTER_OBJECT,
            description: "Returns logs matching a filter",
        },
        CommandDef {
            namespace: "eth",
            name: "newFilter",
            rpc_method: "eth_newFilter",
            params: FILTER_OBJECT,
            description: "Creates a new log filter",
        },
        CommandDef {
            namespace: "eth",
            name: "getFilterChanges",
            rpc_method: "eth_getFilterChanges",
            params: FILTER_ID,
            description: "Returns filter changes since last poll",
        },
        CommandDef {
            namespace: "eth",
            name: "uninstallFilter",
            rpc_method: "eth_uninstallFilter",
            params: FILTER_ID,
            description: "Removes a filter",
        },
        CommandDef {
            namespace: "eth",
            name: "createAccessList",
            rpc_method: "eth_createAccessList",
            params: TX_OBJECT_BLOCK,
            description: "Creates an EIP-2930 access list",
        },
        CommandDef {
            namespace: "eth",
            name: "getProof",
            rpc_method: "eth_getProof",
            params: GET_PROOF,
            description: "Returns the Merkle proof for an account",
        },
    ]
}
