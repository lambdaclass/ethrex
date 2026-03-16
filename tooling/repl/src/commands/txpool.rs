use super::{CommandDef, ParamDef};

const NO_PARAMS: &[ParamDef] = &[];

pub fn commands() -> Vec<CommandDef> {
    vec![
        CommandDef {
            namespace: "txpool",
            name: "content",
            rpc_method: "txpool_content",
            params: NO_PARAMS,
            description: "Returns all pending and queued transactions",
        },
        CommandDef {
            namespace: "txpool",
            name: "status",
            rpc_method: "txpool_status",
            params: NO_PARAMS,
            description: "Returns the number of pending and queued transactions",
        },
    ]
}
