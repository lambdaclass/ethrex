use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::client::HeimdallClient;
use super::types::{EventRecord, Milestone, Span};
use crate::bor_config::BorConfig;

const MILESTONE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SPAN_POLL_INTERVAL: Duration = Duration::from_secs(5);
const STATE_SYNC_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Maximum number of state sync events to fetch per poll.
const STATE_SYNC_FETCH_LIMIT: u64 = 50;

/// Shared state populated by the Heimdall background poller.
///
/// Consumers (e.g., the block builder or verifier) read this via the
/// `Arc<RwLock<HeimdallPollerState>>` handle.
#[derive(Debug, Default)]
pub struct HeimdallPollerState {
    pub latest_milestone: Option<Milestone>,
    pub current_span: Option<Span>,
    pub next_span: Option<Span>,
    pub pending_state_sync_events: Vec<EventRecord>,
    pub last_state_sync_id: u64,
}

/// Background service that polls Heimdall for milestones, spans, and state
/// sync events, keeping shared state up to date for other components.
pub struct HeimdallPoller {
    client: HeimdallClient,
    bor_config: BorConfig,
    state: Arc<RwLock<HeimdallPollerState>>,
    cancel_token: CancellationToken,
}

impl HeimdallPoller {
    pub fn new(
        heimdall_url: &str,
        bor_config: BorConfig,
        state: Arc<RwLock<HeimdallPollerState>>,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            client: HeimdallClient::new(heimdall_url, cancel_token.clone()),
            bor_config,
            state,
            cancel_token,
        }
    }

    /// Spawns the background polling loop. Returns a handle that can be awaited
    /// for clean shutdown. Cancel the token to stop the poller.
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!("Heimdall poller started");
            self.run().await;
            info!("Heimdall poller stopped");
        })
    }

    async fn run(&self) {
        let mut milestone_tick = tokio::time::interval(MILESTONE_POLL_INTERVAL);
        let mut span_tick = tokio::time::interval(SPAN_POLL_INTERVAL);
        let mut state_sync_tick = tokio::time::interval(STATE_SYNC_POLL_INTERVAL);

        // Don't let ticks pile up if a poll takes longer than the interval.
        milestone_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        span_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        state_sync_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = self.cancel_token.cancelled() => {
                    return;
                }
                _ = milestone_tick.tick() => {
                    self.poll_milestone().await;
                }
                _ = span_tick.tick() => {
                    self.poll_spans().await;
                }
                _ = state_sync_tick.tick() => {
                    self.poll_state_sync_events().await;
                }
            }
        }
    }

    async fn poll_milestone(&self) {
        match self.client.fetch_latest_milestone().await {
            Ok(milestone) => {
                debug!(
                    id = milestone.id,
                    end_block = milestone.end_block,
                    "updated latest milestone"
                );
                self.state.write().await.latest_milestone = Some(milestone);
            }
            Err(e) => {
                warn!("failed to fetch latest milestone: {e}");
            }
        }
    }

    async fn poll_spans(&self) {
        match self.client.fetch_latest_span().await {
            Ok(latest) => {
                let mut state = self.state.write().await;

                let should_update_current = state
                    .current_span
                    .as_ref()
                    .is_none_or(|s| s.id != latest.id);

                if should_update_current {
                    debug!(
                        span_id = latest.id,
                        start = latest.start_block,
                        end = latest.end_block,
                        "updated current span"
                    );
                    // The old current span's successor is now current; clear next.
                    state.next_span = None;
                    state.current_span = Some(latest);
                }

                // Pre-fetch next span if we don't have it yet.
                // We fetch it once the current span is known so it's ready
                // before the span boundary is reached.
                if state.next_span.is_none()
                    && let Some(current) = &state.current_span
                {
                    let next_id = current.id + 1;
                    // Drop the lock before the network call.
                    drop(state);
                    self.prefetch_next_span(next_id).await;
                }
            }
            Err(e) => {
                warn!("failed to fetch latest span: {e}");
            }
        }
    }

    async fn prefetch_next_span(&self, span_id: u64) {
        match self.client.try_fetch_span(span_id).await {
            Ok(span) => {
                debug!(
                    span_id = span.id,
                    start = span.start_block,
                    end = span.end_block,
                    "pre-fetched next span"
                );
                self.state.write().await.next_span = Some(span);
            }
            Err(e) => {
                // Not-found is expected when Heimdall hasn't proposed the next
                // span yet -- only warn on unexpected errors.
                debug!("next span {span_id} not yet available: {e}");
            }
        }
    }

    async fn poll_state_sync_events(&self) {
        let from_id = self.state.read().await.last_state_sync_id + 1;

        // Use current time as the upper bound. Heimdall expects a Unix
        // timestamp; events with record_time <= this value are returned.
        let to_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Apply the confirmation delay from BorConfig (0 if not configured).
        // This avoids fetching events that haven't been confirmed yet.
        let delay = self.bor_config.get_state_sync_delay(0);
        let to_time = to_time.saturating_sub(delay);

        match self
            .client
            .fetch_state_sync_events(from_id, to_time, STATE_SYNC_FETCH_LIMIT)
            .await
        {
            Ok(events) => {
                if !events.is_empty() {
                    let count = events.len();
                    let max_id = events.iter().map(|e| e.id).max().unwrap_or(from_id - 1);
                    debug!(count, max_id, "fetched new state sync events");

                    let mut state = self.state.write().await;
                    state.last_state_sync_id = max_id;
                    state.pending_state_sync_events.extend(events);
                }
            }
            Err(e) => {
                warn!("failed to fetch state sync events: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{Address, H256};

    fn make_milestone(id: u64, start: u64, end: u64) -> Milestone {
        Milestone {
            id,
            start_block: start,
            end_block: end,
            hash: H256::from_low_u64_be(id),
        }
    }

    fn make_span(id: u64, start: u64, end: u64) -> Span {
        Span {
            id,
            start_block: start,
            end_block: end,
            selected_producers: vec![],
            validators: vec![],
        }
    }

    fn make_event(id: u64) -> EventRecord {
        EventRecord {
            id,
            contract: Address::zero(),
            data: String::new(),
            tx_hash: H256::zero(),
            log_index: 0,
            bor_chain_id: "137".to_string(),
            record_time: "2023-01-01T00:00:00Z".to_string(),
        }
    }

    // ---- HeimdallPollerState default ----

    #[test]
    fn state_default_values() {
        let state = HeimdallPollerState::default();
        assert!(state.latest_milestone.is_none());
        assert!(state.current_span.is_none());
        assert!(state.next_span.is_none());
        assert!(state.pending_state_sync_events.is_empty());
        assert_eq!(state.last_state_sync_id, 0);
    }

    // ---- Milestone updates ----

    #[test]
    fn milestone_update() {
        let mut state = HeimdallPollerState::default();
        let m = make_milestone(1, 0, 100);
        state.latest_milestone = Some(m);

        let ms = state.latest_milestone.as_ref().unwrap();
        assert_eq!(ms.id, 1);
        assert_eq!(ms.end_block, 100);

        // Replace with a newer milestone
        state.latest_milestone = Some(make_milestone(2, 101, 200));
        let ms = state.latest_milestone.as_ref().unwrap();
        assert_eq!(ms.id, 2);
        assert_eq!(ms.end_block, 200);
    }

    // ---- Span transitions ----

    #[test]
    fn span_transition_current_and_next() {
        let mut state = HeimdallPollerState::default();

        // Set initial span
        state.current_span = Some(make_span(1, 256, 6655));
        assert_eq!(state.current_span.as_ref().unwrap().id, 1);
        assert!(state.next_span.is_none());

        // Pre-fetch next span
        state.next_span = Some(make_span(2, 6656, 13055));
        assert_eq!(state.next_span.as_ref().unwrap().id, 2);

        // Transition: new span becomes current, next is cleared
        // (This mirrors the logic in poll_spans)
        let new_current = state.next_span.take().unwrap();
        state.current_span = Some(new_current);
        state.next_span = None;

        assert_eq!(state.current_span.as_ref().unwrap().id, 2);
        assert!(state.next_span.is_none());
    }

    #[test]
    fn span_update_only_when_id_changes() {
        let mut state = HeimdallPollerState::default();
        let span = make_span(5, 256, 6655);
        state.current_span = Some(span.clone());
        state.next_span = Some(make_span(6, 6656, 13055));

        // Simulate poll returning same span id — should not clear next_span
        let latest_id = 5;
        let should_update = state
            .current_span
            .as_ref()
            .is_none_or(|s| s.id != latest_id);
        assert!(!should_update);
        // next_span should remain untouched
        assert!(state.next_span.is_some());
    }

    // ---- Pending state sync events accumulation ----

    #[test]
    fn pending_events_accumulate() {
        let mut state = HeimdallPollerState::default();

        // First batch
        let events_1 = vec![make_event(1), make_event(2)];
        let max_id_1 = events_1.iter().map(|e| e.id).max().unwrap();
        state.last_state_sync_id = max_id_1;
        state.pending_state_sync_events.extend(events_1);
        assert_eq!(state.pending_state_sync_events.len(), 2);
        assert_eq!(state.last_state_sync_id, 2);

        // Second batch
        let events_2 = vec![make_event(3), make_event(4), make_event(5)];
        let max_id_2 = events_2.iter().map(|e| e.id).max().unwrap();
        state.last_state_sync_id = max_id_2;
        state.pending_state_sync_events.extend(events_2);
        assert_eq!(state.pending_state_sync_events.len(), 5);
        assert_eq!(state.last_state_sync_id, 5);

        // Verify ordering is preserved
        let ids: Vec<u64> = state
            .pending_state_sync_events
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn pending_events_can_be_drained() {
        let mut state = HeimdallPollerState::default();
        state
            .pending_state_sync_events
            .extend(vec![make_event(1), make_event(2), make_event(3)]);
        state.last_state_sync_id = 3;

        // Consumer drains events
        let drained: Vec<EventRecord> = state.pending_state_sync_events.drain(..).collect();
        assert_eq!(drained.len(), 3);
        assert!(state.pending_state_sync_events.is_empty());
        // last_state_sync_id is NOT reset — it tracks the high watermark
        assert_eq!(state.last_state_sync_id, 3);
    }

    // ---- Concurrent access via Arc<RwLock> ----

    #[tokio::test]
    async fn concurrent_read_write() {
        let state = Arc::new(RwLock::new(HeimdallPollerState::default()));

        // Writer sets a milestone
        {
            let mut w = state.write().await;
            w.latest_milestone = Some(make_milestone(1, 0, 100));
        }

        // Multiple concurrent readers
        let mut handles = Vec::new();
        for _ in 0..10 {
            let state_clone = state.clone();
            handles.push(tokio::spawn(async move {
                let r = state_clone.read().await;
                let ms = r.latest_milestone.as_ref().unwrap();
                assert_eq!(ms.id, 1);
                assert_eq!(ms.end_block, 100);
            }));
        }

        for h in handles {
            h.await.unwrap();
        }
    }

    #[tokio::test]
    async fn concurrent_writers_sequential() {
        let state = Arc::new(RwLock::new(HeimdallPollerState::default()));

        // Spawn writers sequentially to verify no data corruption
        for i in 1..=5u64 {
            let state_clone = state.clone();
            let handle = tokio::spawn(async move {
                let mut w = state_clone.write().await;
                w.latest_milestone = Some(make_milestone(i, i * 100, i * 100 + 99));
                w.last_state_sync_id = i * 10;
            });
            handle.await.unwrap();
        }

        // Final state should reflect the last writer
        let r = state.read().await;
        assert_eq!(r.latest_milestone.as_ref().unwrap().id, 5);
        assert_eq!(r.last_state_sync_id, 50);
    }

    #[tokio::test]
    async fn reader_writer_interleaved() {
        let state = Arc::new(RwLock::new(HeimdallPollerState::default()));

        // Writer adds events
        {
            let mut w = state.write().await;
            w.pending_state_sync_events
                .extend(vec![make_event(1), make_event(2)]);
            w.last_state_sync_id = 2;
        }

        // Reader checks state
        {
            let r = state.read().await;
            assert_eq!(r.pending_state_sync_events.len(), 2);
            assert_eq!(r.last_state_sync_id, 2);
        }

        // Another writer adds more
        {
            let mut w = state.write().await;
            w.pending_state_sync_events.push(make_event(3));
            w.last_state_sync_id = 3;
        }

        // Reader sees updated state
        {
            let r = state.read().await;
            assert_eq!(r.pending_state_sync_events.len(), 3);
            assert_eq!(r.last_state_sync_id, 3);
        }
    }
}
