use ethrex_rpc::utils::{RpcErr, RpcNamespace, RpcRequest};

#[test]
fn from_prefix_recognizes_ethrex_namespace() {
    assert_eq!(
        RpcNamespace::from_prefix("ethrex"),
        Some(RpcNamespace::Ethrex)
    );
}

#[test]
fn ethrex_method_resolves_to_ethrex_namespace() {
    let req = RpcRequest::new("ethrex_simulateFrameTransaction", None);
    assert_eq!(req.namespace().unwrap(), RpcNamespace::Ethrex);
}

#[test]
fn unknown_namespace_is_method_not_found() {
    let req = RpcRequest::new("bogus_method", None);
    assert!(matches!(req.namespace(), Err(RpcErr::MethodNotFound(_))));
}
