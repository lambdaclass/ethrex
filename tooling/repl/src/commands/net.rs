use super::{CommandDef, ParamDef};

const NO_PARAMS: &[ParamDef] = &[];

pub fn commands() -> Vec<CommandDef> {
    vec![
        CommandDef {
            namespace: "net",
            name: "version",
            rpc_method: "net_version",
            params: NO_PARAMS,
            description: "Returns the network ID",
        },
        CommandDef {
            namespace: "net",
            name: "peerCount",
            rpc_method: "net_peerCount",
            params: NO_PARAMS,
            description: "Returns the number of connected peers",
        },
    ]
}
