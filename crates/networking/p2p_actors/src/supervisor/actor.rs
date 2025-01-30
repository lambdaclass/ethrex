use crate::{
    peer,
    supervisor::{
        ingress::{Mailbox, Message},
        types::{AutoShutdown, ChildSpec, ChildState, RestartStrategy, RestartType, Shutdown},
    },
};
use commonware_runtime::Spawner;
use std::{collections::HashMap, time::Duration};
use tokio::{sync::mpsc, time::Instant};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Auto-shutdown disabled. Significant children are not accepted.")]
    SignificantChildrenNotAccepted,
    #[error("Commonware runtime error: {0}")]
    CommonwareRuntimeError(#[from] commonware_runtime::Error),
}

#[derive(Debug, Clone)]
pub struct Config {
    strategy: RestartStrategy,
    intensity: usize,
    period: u64,
    auto_shutdown: AutoShutdown,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            strategy: RestartStrategy::default(),
            intensity: 5,
            period: 30,
            auto_shutdown: AutoShutdown::default(),
        }
    }
}

pub struct Actor {
    runtime: commonware_runtime::tokio::Context,

    mailbox: Mailbox,
    receiver: mpsc::Receiver<Message>,

    strategy: RestartStrategy,
    intensity: usize,
    period: u64,
    auto_shutdown: AutoShutdown,

    children: HashMap<&'static str, ChildState>,
    restart_times: Vec<Instant>,
}

impl Actor {
    pub fn new(runtime: commonware_runtime::tokio::Context, cfg: Config) -> (Self, Mailbox) {
        let (sender, receiver) = mpsc::channel(1);
        let mailbox = Mailbox::new(sender);
        let actor = Self {
            runtime,
            mailbox: mailbox.clone(),
            receiver,
            strategy: cfg.strategy,
            intensity: cfg.intensity,
            period: cfg.period,
            auto_shutdown: cfg.auto_shutdown,
            children: HashMap::new(),
            restart_times: Vec::new(),
        };
        (actor, mailbox)
    }

    async fn handle_child_exit(
        &mut self,
        result: Result<Result<(), peer::Error>, commonware_runtime::Error>,
        spec: ChildSpec,
    ) {
        if spec.significant {
            tracing::error!("Significant child process exited. Shutting down.");
            self.mailbox.terminate().await.unwrap();
            return;
        }

        let should_restart = match spec.restart {
            RestartType::Permanent => true,
            RestartType::Transient => match result {
                Ok(Ok(())) => false,
                Err(_) | Ok(Err(_)) => true,
            },
            RestartType::Temporary => false,
        };

        if should_restart {
            self.restart_times.push(Instant::now());
            self.clean_restart_history();

            // If more than `intensity` number of restarts occur in the last
            // `period` seconds, the supervisor terminates all the child
            // processes and then itself. The termination reason for the
            // supervisor itself in that case will be shutdown.
            if self.restart_times.len() > self.intensity {
                tracing::error!("Maximum restarts exceeded. Shutting down.");
                self.mailbox.terminate().await.unwrap();
                return;
            }

            self.mailbox.start_child(spec).await.unwrap();
        }
    }

    fn clean_restart_history(&mut self) {
        let now = Instant::now();
        self.restart_times
            .retain(|t| now.duration_since(*t) < Duration::from_secs(self.period));
    }

    pub async fn run(mut self) -> Result<(), Error> {
        loop {
            let message = self.receiver.recv().await.unwrap();
            match message {
                Message::Supervise => {
                    let finished: Vec<&str> = self
                        .children
                        .iter()
                        .filter_map(|(id, state)| state.stopped.then_some(*id))
                        .collect();

                    for id in finished {
                        self.mailbox.delete_child(id).await.unwrap();
                    }
                }
                Message::StartChild(child_spec) => {
                    // TODO: Figure out how to pass which actor we want to start
                    let new_child_handle = self.runtime.spawn(child_spec.id, (child_spec.start)());
                    let new_child_state = ChildState {
                        handle: new_child_handle,
                        spec: child_spec,
                        restarts: 0,
                        stopped: false,
                    };
                    self.children
                        .insert(new_child_state.spec.id, new_child_state);
                }
                Message::TerminateChild(child_id) => {
                    let Some(child_to_terminate) = self.children.get_mut(child_id) else {
                        // TODO: Is is ok to ignore this?
                        continue;
                    };

                    // TODO: Stopping a significant child of a supervisor configured for automatic shutdown
                    // will not trigger an automatic shutdown.
                    // https://www.erlang.org/doc/system/sup_princ#stopping-a-child-process
                    if child_to_terminate.spec.significant {}

                    // TODO: Terminate the child. For this we need the child's mailbox in the child state.
                    // We'll probably need to create a Mailbox trait that all actors' Mailboxes implement.
                    // NOTE: By following the Erlang supervisor spec, we should take into account the
                    // child's shutdown strategy (check out the Shutdown enum from the types.rs module).
                    match child_to_terminate.spec.shutdown {
                        Shutdown::BrutalKill => child_to_terminate.handle.abort(),
                        Shutdown::Timeout(_duration) => {
                            // tokio::time::timeout(duration, child_to_terminate.mailbox.terminate()).await;
                        }
                    }

                    child_to_terminate.stopped = true;
                }
                Message::DeleteChild(child_id) => {
                    let Some(child_to_terminate) = self.children.get(child_id) else {
                        // TODO: Is is ok to ignore this?
                        continue;
                    };

                    if !child_to_terminate.stopped {
                        continue;
                    }

                    let Some(removed_child_state) = self.children.remove(child_id) else {
                        continue;
                    };

                    let removed_child_handle_result = removed_child_state.handle.await;

                    self.handle_child_exit(removed_child_handle_result, removed_child_state.spec)
                        .await;
                }
                Message::Terminate => {
                    // TODO: Add the necessary logic to terminate the supervisor and its children.
                    //
                    // Since the supervisor is part of a supervision tree, it is automatically terminated
                    // by its supervisor. When asked to shut down, a supervisor terminates all child
                    // processes in reverse start order according to the respective shutdown specifications
                    // before terminating itself.
                    //
                    // If the supervisor is configured for automatic shutdown on termination of any or all
                    // significant children, it will shut down itself when any or the last active significant
                    // child terminates, respectively. The shutdown itself follows the same procedure as
                    // described above, that is, the supervisor terminates all remaining child processes in
                    // reverse start order before terminating itself.
                    // https://www.erlang.org/doc/system/sup_princ#stopping

                    return Ok(());
                }
            }
        }
    }
}
