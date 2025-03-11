#![allow(async_fn_in_trait)]

use super::connections::Connections;
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex;
use tracing::Level;

pub trait Actor
where
    Self: Sized,
{
    type Error: Error + Send + 'static;

    type Connections: Connections + 'static;

    /// Returns the name of the actor.
    fn name() -> &'static str {
        #[allow(clippy::unwrap_used)]
        std::any::type_name::<Self>().split("::").last().unwrap()
    }

    fn on_init(&self, _connections: Arc<Mutex<Self::Connections>>) {}

    fn _on_init(&self, _connections: Arc<Mutex<Self::Connections>>) {
        tracing::debug!("initializing...");
        self.on_init(_connections);
        tracing::debug!("initialized");
    }

    fn should_stop(&self) -> bool;

    async fn loop_body(&mut self, connections: Arc<Mutex<Self::Connections>>);

    fn on_exit(&self, _connections: Arc<Mutex<Self::Connections>>) {}

    fn _on_exit(&self, _connections: Arc<Mutex<Self::Connections>>) {
        tracing::debug!("running final tasks before stopping...");
        self.on_exit(_connections);
        tracing::debug!("stopped");
    }

    // fn send_after(
    //     connections: &mut SpineConnections,
    //     duration: Duration,
    // ) -> tokio::task::JoinHandle<Result<(), Self::Error>> {
    //     tokio::spawn(async move {
    //         let mut interval = tokio::time::interval(duration);
    //         loop {
    //             interval.tick().await;
    //             connections.send(message.clone()).await?;
    //         }
    //     })
    // }

    async fn run(mut self, connections: Self::Connections) {
        let conn = Arc::new(Mutex::new(connections));
        let _s = tracing::span!(Level::INFO, "", actor = Self::name()).entered();

        self._on_init(conn.clone());

        loop {
            if self.should_stop() {
                break;
            }

            self.loop_body(conn.clone()).await;
        }

        self._on_exit(conn);
    }
}
