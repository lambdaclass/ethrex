use super::{CommandDef, ParamDef};

const NO_PARAMS: &[ParamDef] = &[];

pub fn commands() -> Vec<CommandDef> {
    vec![CommandDef {
        namespace: "web3",
        name: "clientVersion",
        rpc_method: "web3_clientVersion",
        params: NO_PARAMS,
        description: "Returns the client version",
    }]
}
