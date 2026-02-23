use super::{CommandDef, ParamDef, ParamType};

const NO_PARAMS: &[ParamDef] = &[];

const LEVEL: &[ParamDef] = &[ParamDef {
    name: "level",
    param_type: ParamType::StringParam,
    required: true,
    default_value: None,
    description: "Log level (trace, debug, info, warn, error)",
}];

const ENODE: &[ParamDef] = &[ParamDef {
    name: "enode",
    param_type: ParamType::StringParam,
    required: true,
    default_value: None,
    description: "Enode URL of peer to add",
}];

pub fn commands() -> Vec<CommandDef> {
    vec![
        CommandDef {
            namespace: "admin",
            name: "nodeInfo",
            rpc_method: "admin_nodeInfo",
            params: NO_PARAMS,
            description: "Returns node information",
        },
        CommandDef {
            namespace: "admin",
            name: "peers",
            rpc_method: "admin_peers",
            params: NO_PARAMS,
            description: "Returns connected peers",
        },
        CommandDef {
            namespace: "admin",
            name: "setLogLevel",
            rpc_method: "admin_setLogLevel",
            params: LEVEL,
            description: "Sets the node log level",
        },
        CommandDef {
            namespace: "admin",
            name: "addPeer",
            rpc_method: "admin_addPeer",
            params: ENODE,
            description: "Adds a peer by enode URL",
        },
    ]
}
