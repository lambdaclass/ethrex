use super::{CommandDef, ParamDef, ParamType};

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

const HASH_OPTIONS: &[ParamDef] = &[
    ParamDef {
        name: "hash",
        param_type: ParamType::Hash,
        required: true,
        default_value: None,
        description: "Transaction hash",
    },
    ParamDef {
        name: "options",
        param_type: ParamType::Object,
        required: false,
        default_value: None,
        description: "Trace options",
    },
];

const BLOCK_OPTIONS: &[ParamDef] = &[
    ParamDef {
        name: "block",
        param_type: ParamType::BlockId,
        required: true,
        default_value: None,
        description: "Block identifier",
    },
    ParamDef {
        name: "options",
        param_type: ParamType::Object,
        required: false,
        default_value: None,
        description: "Trace options",
    },
];

pub fn commands() -> Vec<CommandDef> {
    vec![
        CommandDef {
            namespace: "debug",
            name: "getRawHeader",
            rpc_method: "debug_getRawHeader",
            params: BLOCK_ONLY,
            description: "Returns the RLP-encoded block header",
        },
        CommandDef {
            namespace: "debug",
            name: "getRawBlock",
            rpc_method: "debug_getRawBlock",
            params: BLOCK_ONLY,
            description: "Returns the RLP-encoded block",
        },
        CommandDef {
            namespace: "debug",
            name: "getRawTransaction",
            rpc_method: "debug_getRawTransaction",
            params: HASH_ONLY,
            description: "Returns the RLP-encoded transaction",
        },
        CommandDef {
            namespace: "debug",
            name: "getRawReceipts",
            rpc_method: "debug_getRawReceipts",
            params: BLOCK_ONLY,
            description: "Returns the RLP-encoded receipts for a block",
        },
        CommandDef {
            namespace: "debug",
            name: "executionWitness",
            rpc_method: "debug_executionWitness",
            params: BLOCK_ONLY,
            description: "Returns the execution witness for a block",
        },
        CommandDef {
            namespace: "debug",
            name: "getBlockAccessList",
            rpc_method: "debug_getBlockAccessList",
            params: BLOCK_ONLY,
            description: "Returns the access list for a block",
        },
        CommandDef {
            namespace: "debug",
            name: "traceTransaction",
            rpc_method: "debug_traceTransaction",
            params: HASH_OPTIONS,
            description: "Traces a transaction execution",
        },
        CommandDef {
            namespace: "debug",
            name: "traceBlockByNumber",
            rpc_method: "debug_traceBlockByNumber",
            params: BLOCK_OPTIONS,
            description: "Traces all transactions in a block",
        },
    ]
}
