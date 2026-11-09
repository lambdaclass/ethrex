//! `engine_newPayload` executions that are still running, by block hash.
//!
//! A consensus client re-sends `newPayload` when the first call is slow to answer. A
//! re-send that arrives before the block is stored would otherwise be queued behind the
//! first and executed again; instead it waits for the execution already running.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use ethrex_common::H256;
use tokio::sync::watch;

use crate::{types::payload::PayloadStatus, utils::RpcErr};

/// The answer every request for a block gets once its execution has been classified.
pub type PayloadOutcome = Result<PayloadStatus, RpcErr>;

type OutcomeReceiver = watch::Receiver<Option<PayloadOutcome>>;

/// Blocks whose execution is running, each with the channel its outcome is published on.
#[derive(Default)]
pub struct InFlightPayloads {
    executions: Mutex<HashMap<H256, OutcomeReceiver>>,
}

/// How a request takes part in executing its block.
pub enum Role<T> {
    /// No execution was running: this request enqueued the block and owns publishing its
    /// outcome.
    Leader {
        outcome: OutcomeReceiver,
        completion: Completion,
        enqueued: T,
    },
    /// The block is already executing: wait for that execution's outcome.
    Follower { outcome: OutcomeReceiver },
}

impl InFlightPayloads {
    /// Joins the execution already running for `hash`, or starts one by calling `enqueue`.
    ///
    /// `enqueue` runs only when no execution is running, while the in-flight set is
    /// locked, so two requests can't both enqueue the same block. It must hand the block
    /// off without blocking. If it fails, nothing is registered and its error is returned.
    pub fn join_or_start<T, E>(
        self: &Arc<Self>,
        hash: H256,
        enqueue: impl FnOnce() -> Result<T, E>,
    ) -> Result<Role<T>, E> {
        let mut executions = self
            .executions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(outcome) = executions.get(&hash) {
            return Ok(Role::Follower {
                outcome: outcome.clone(),
            });
        }
        let enqueued = enqueue()?;
        let (sender, outcome) = watch::channel(None);
        executions.insert(hash, outcome.clone());
        drop(executions);
        let entry = Entry {
            in_flight: Arc::clone(self),
            hash,
            outcome: outcome.clone(),
        };
        Ok(Role::Leader {
            outcome,
            completion: Completion {
                _entry: entry,
                sender,
            },
            enqueued,
        })
    }
}

/// Publishes a block's outcome to every request waiting on it. Dropping it, whether or
/// not it published, takes the block out of the in-flight set; waiters left without an
/// outcome get an error.
pub struct Completion {
    // Fields drop in declaration order: the block leaves the in-flight set before the
    // sender closes, so a request arriving as an abandoned execution ends starts a new
    // one instead of joining it.
    _entry: Entry,
    sender: watch::Sender<Option<PayloadOutcome>>,
}

impl Completion {
    pub fn publish(self, outcome: PayloadOutcome) {
        self.sender.send_replace(Some(outcome));
    }
}

/// A block's registration in the in-flight set, removed when dropped.
struct Entry {
    in_flight: Arc<InFlightPayloads>,
    hash: H256,
    outcome: OutcomeReceiver,
}

impl Drop for Entry {
    fn drop(&mut self) {
        let mut executions = self
            .in_flight
            .executions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Only remove our own registration, never a newer execution's for the same block.
        if executions
            .get(&self.hash)
            .is_some_and(|current| current.same_channel(&self.outcome))
        {
            executions.remove(&self.hash);
        }
    }
}

/// Waits for the outcome published on `outcome`.
pub async fn wait(mut outcome: OutcomeReceiver) -> PayloadOutcome {
    let published = match outcome.wait_for(Option::is_some).await {
        Ok(value) => Option::clone(&value),
        Err(_) => None,
    };
    published.unwrap_or_else(|| {
        Err(RpcErr::Internal(
            "payload execution ended without a result".to_string(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::types::payload::PayloadValidationStatus;

    fn join(in_flight: &Arc<InFlightPayloads>, hash: H256, enqueues: &Cell<u32>) -> Role<()> {
        let joined: Result<Role<()>, RpcErr> = in_flight.join_or_start(hash, || {
            enqueues.set(enqueues.get() + 1);
            Ok(())
        });
        joined.unwrap()
    }

    fn valid(hash: H256) -> PayloadOutcome {
        Ok(PayloadStatus::valid_with_hash(hash))
    }

    fn status(outcome: PayloadOutcome) -> (PayloadValidationStatus, Option<H256>) {
        let status = outcome.unwrap();
        (status.status, status.latest_valid_hash)
    }

    #[tokio::test]
    async fn duplicates_join_the_running_execution() {
        let in_flight = Arc::new(InFlightPayloads::default());
        let hash = H256::repeat_byte(1);
        let enqueues = Cell::new(0);

        let Role::Leader {
            outcome: leader,
            completion,
            ..
        } = join(&in_flight, hash, &enqueues)
        else {
            panic!("the first request for a block must lead its execution");
        };
        let followers: Vec<_> = (0..2)
            .map(|_| match join(&in_flight, hash, &enqueues) {
                Role::Follower { outcome } => outcome,
                Role::Leader { .. } => panic!("a duplicate started a second execution"),
            })
            .collect();
        assert_eq!(enqueues.get(), 1);

        completion.publish(valid(hash));
        assert_eq!(
            status(wait(leader).await),
            (PayloadValidationStatus::Valid, Some(hash))
        );
        for follower in followers {
            assert_eq!(
                status(wait(follower).await),
                (PayloadValidationStatus::Valid, Some(hash))
            );
        }
    }

    #[test]
    fn a_published_execution_leaves_the_in_flight_set() {
        let in_flight = Arc::new(InFlightPayloads::default());
        let hash = H256::repeat_byte(2);
        let enqueues = Cell::new(0);

        let Role::Leader { completion, .. } = join(&in_flight, hash, &enqueues) else {
            panic!("the first request for a block must lead its execution");
        };
        completion.publish(valid(hash));

        assert!(
            matches!(join(&in_flight, hash, &enqueues), Role::Leader { .. }),
            "a request after the outcome was published joined the finished execution"
        );
        assert_eq!(enqueues.get(), 2);
    }

    #[tokio::test]
    async fn an_abandoned_execution_fails_its_waiters_and_leaves_the_in_flight_set() {
        let in_flight = Arc::new(InFlightPayloads::default());
        let hash = H256::repeat_byte(3);
        let enqueues = Cell::new(0);

        let Role::Leader { completion, .. } = join(&in_flight, hash, &enqueues) else {
            panic!("the first request for a block must lead its execution");
        };
        let Role::Follower { outcome: follower } = join(&in_flight, hash, &enqueues) else {
            panic!("a duplicate started a second execution");
        };
        // The task that owned the completion panicked or was cancelled.
        drop(completion);

        assert!(matches!(wait(follower).await, Err(RpcErr::Internal(_))));
        assert!(
            matches!(join(&in_flight, hash, &enqueues), Role::Leader { .. }),
            "a request after the execution was abandoned joined it"
        );
    }

    #[tokio::test]
    async fn a_leader_whose_caller_went_away_still_answers_its_followers() {
        let in_flight = Arc::new(InFlightPayloads::default());
        let hash = H256::repeat_byte(4);
        let enqueues = Cell::new(0);

        let Role::Leader {
            outcome: leader,
            completion,
            ..
        } = join(&in_flight, hash, &enqueues)
        else {
            panic!("the first request for a block must lead its execution");
        };
        let Role::Follower { outcome: follower } = join(&in_flight, hash, &enqueues) else {
            panic!("a duplicate started a second execution");
        };
        // The consensus client that sent the first request disconnected.
        drop(leader);
        completion.publish(valid(hash));

        assert_eq!(
            status(wait(follower).await),
            (PayloadValidationStatus::Valid, Some(hash))
        );
    }
}
