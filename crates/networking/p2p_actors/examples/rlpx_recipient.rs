use commonware_runtime::Runner;
use ethereum_p2p::peer;
use k256::SecretKey;
use std::{net::TcpListener, str::FromStr};
use tracing_subscriber::{filter::Directive, EnvFilter, FmtSubscriber};

fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(Directive::from_str("info").unwrap())
                .from_env_lossy(),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();

    let (executor, _runtime) = commonware_runtime::tokio::Executor::default();

    executor.start(async move {
        let listener = TcpListener::bind("127.0.0.1:30303").unwrap();
        let config = peer::Config::new(
            SecretKey::from_slice(
                &hex::decode("cac6764060352ecaf276908d732cf48ba4b1a3f00d24d32036123c25cdc83838")
                    .unwrap(),
            )
            .unwrap(),
        );

        // TODO: This will accept only one connection. Should be a loop
        let (stream, _) = listener.accept().unwrap();
        let (actor, _) = peer::Actor::new_as_recipient(stream, config).unwrap();
        actor.run().await.unwrap();
    })
}
