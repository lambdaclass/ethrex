use std::{collections::HashMap, sync::Arc};

use spawned_concurrency::{
    messages::Unused,
    tasks::{CastResponse, GenServer, send_after},
};
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum DiscoverySideCarError {}

#[derive(Debug, Clone)]
pub struct DiscoverySideCarState {
    kademlia: Arc<Mutex<HashMap<String, String>>>,
    revalidation_period: std::time::Duration,
    lookup_period: std::time::Duration,
}

#[derive(Debug, Clone)]
pub enum InMessage {
    Revalidate,
    Lookup,
    Prune,
}

#[derive(Debug, Clone)]
pub enum OutMessage {
    Done,
}

pub struct DiscoverySideCar;

impl GenServer for DiscoverySideCar {
    type CallMsg = Unused;
    type CastMsg = InMessage;
    type OutMsg = OutMessage;
    type State = DiscoverySideCarState;
    type Error = DiscoverySideCarError;

    fn new() -> Self {
        Self {}
    }

    async fn handle_cast(
        &mut self,
        message: Self::CastMsg,
        handle: &spawned_concurrency::tasks::GenServerHandle<Self>,
        state: Self::State,
    ) -> CastResponse<Self> {
        match message {
            Self::CastMsg::Revalidate => {
                send_after(
                    state.revalidation_period,
                    handle.clone(),
                    Self::CastMsg::Revalidate,
                );
                CastResponse::NoReply(state)
            }
            Self::CastMsg::Lookup => {
                send_after(
                    state.lookup_period,
                    handle.clone(),
                    Self::CastMsg::Revalidate,
                );
                CastResponse::NoReply(state)
            }
            Self::CastMsg::Prune => {
                // Once we have a pruning strategy, we can implement it here.
                // For now, no one is pruned.
                CastResponse::NoReply(state)
            }
        }
    }
}

async fn revalidate(state: &DiscoverySideCarState) {
    // Ping all known nodes and tag as disposable if they do not respond in time.
}

async fn lookup(state: &DiscoverySideCarState) {
    // Ask neighbors for their neighbors and add them to the routing table.
}

async fn prune(state: &DiscoverySideCarState) {
    // Remove nodes tagged as disposable.
}
