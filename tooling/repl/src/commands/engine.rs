use super::{CommandDef, ParamDef, ParamType};

const FORK_CHOICE_UPDATED_V3: &[ParamDef] = &[
    ParamDef {
        name: "fork_choice_state",
        param_type: ParamType::Object,
        required: true,
        default_value: None,
        description: "ForkChoiceState {headBlockHash, safeBlockHash, finalizedBlockHash}",
    },
    ParamDef {
        name: "payload_attributes",
        param_type: ParamType::Object,
        required: false,
        default_value: None,
        description: "PayloadAttributesV3 {timestamp, prevRandao, suggestedFeeRecipient, parentBeaconBlockRoot, withdrawals}",
    },
];

const GET_PAYLOAD_V5: &[ParamDef] = &[ParamDef {
    name: "payload_id",
    param_type: ParamType::HexData,
    required: true,
    default_value: None,
    description: "Payload identifier returned by forkchoiceUpdated",
}];

const NEW_PAYLOAD_V4: &[ParamDef] = &[
    ParamDef {
        name: "execution_payload",
        param_type: ParamType::Object,
        required: true,
        default_value: None,
        description: "ExecutionPayload object",
    },
    ParamDef {
        name: "versioned_hashes",
        param_type: ParamType::Array,
        required: true,
        default_value: None,
        description: "Array of blob versioned hashes",
    },
    ParamDef {
        name: "parent_beacon_block_root",
        param_type: ParamType::Hash,
        required: true,
        default_value: None,
        description: "Parent beacon block root",
    },
    ParamDef {
        name: "execution_requests",
        param_type: ParamType::Array,
        required: true,
        default_value: None,
        description: "Array of execution requests (EIP-7685)",
    },
];

pub fn commands() -> Vec<CommandDef> {
    vec![
        CommandDef {
            namespace: "engine",
            name: "forkchoiceUpdatedV3",
            rpc_method: "engine_forkchoiceUpdatedV3",
            params: FORK_CHOICE_UPDATED_V3,
            description: "Update fork choice state and optionally trigger payload building",
        },
        CommandDef {
            namespace: "engine",
            name: "getPayloadV5",
            rpc_method: "engine_getPayloadV5",
            params: GET_PAYLOAD_V5,
            description: "Get execution payload by ID",
        },
        CommandDef {
            namespace: "engine",
            name: "newPayloadV4",
            rpc_method: "engine_newPayloadV4",
            params: NEW_PAYLOAD_V4,
            description: "Submit a new execution payload for validation",
        },
    ]
}
