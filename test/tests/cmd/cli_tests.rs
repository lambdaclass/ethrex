use clap::Parser as _;
use ethrex::cli::CLI;
use ethrex_rpc::RpcNamespace;

#[test]
fn http_api_parses_ethrex_namespace() {
    let cli = CLI::parse_from(["ethrex", "--http.api", "eth,ethrex"]);
    assert_eq!(
        cli.opts.http_api,
        vec![RpcNamespace::Eth, RpcNamespace::Ethrex]
    );
}
