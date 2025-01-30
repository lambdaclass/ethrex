use commonware_runtime::Runner;
use ethereum_p2p::peer;
use k256::SecretKey;
use std::{
    net::{IpAddr, Ipv4Addr},
    str::FromStr,
};
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
        let config = peer::Config::new(
            SecretKey::from_slice(
                &hex::decode("0d4651652b79491eb129dbcaa1d805a3b34c84ea2e321aefd6e0dde57cf34c02")
                    .unwrap(),
            )
            .unwrap(),
        );

        let (actor, _mailbox) = peer::Actor::new_as_initiator(
            config,
            ethrex_core::H512::from_slice(&hex::decode("d9b75ed9fe287be6af0aab3b5a3734989ceb1fe6c09a8545a2dba2af3d8e8c4e257d65df00f875dda24eb449aa16093ab050cb077296a6d6a4a9676d040e1955").unwrap()),
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            30303,
        ).unwrap();

        actor.run().await.unwrap();
    })
}
