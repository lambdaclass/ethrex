use crate::peer;
use std::{future::Future, pin::Pin, time::Duration};

#[derive(Debug, Clone, Default)]
pub enum AutoShutdown {
    /// Auto-shutdown is disabled. Significant children are not accepted.
    #[default]
    Never,
    /// The supervisor will automatically shut itself down when any significant
    /// child terminates.
    AnySignificant,
    /// The supervisor will automatically shut itself down when all significant
    /// children have terminated, that is, when the last active significant
    /// child terminates.
    AllSignificant,
}

#[derive(Debug, Clone, Default)]
pub enum RestartStrategy {
    /// If a child process terminates, only that process is restarted.
    #[default]
    OneForOne,
    /// If a child process terminates, all remaining child processes are
    /// terminated. Subsequently, all child processes, including the terminated
    /// one, are restarted.
    OneForAll,
    /// If a child process terminates, the child processes after the terminated
    /// process in start order are terminated. Subsequently, the terminated
    /// child process and the remaining child processes are restarted.
    RestForOne,
    // SimpleOneForOne,
}

#[derive(Debug, Clone)]
pub struct ChildSpec {
    /// Used to identify the child specification internally by the supervisor.
    pub id: &'static str,
    /// Defines the function call used to start the child process.
    #[allow(clippy::type_complexity)]
    pub start: fn() -> Pin<Box<dyn Future<Output = Result<(), peer::Error>> + Send>>,
    /// Defines when a terminated child process is to be restarted.
    pub restart: RestartType,
    /// defines whether a child is considered significant for automatic self-shutdown of the supervisor.
    pub significant: bool,
    /// Defines how a child process is to be terminated.
    pub shutdown: Shutdown,
    /// Specifies whether the child process is a supervisor or a worker.
    pub r#type: ChildType,
}

#[derive(Debug, Clone, Default)]
pub enum RestartType {
    /// The child process is always restarted.
    #[default]
    Permanent,
    /// The child process is restarted only if it terminates abnormally.
    Temporary,
    /// The child process is restarted only if it terminates abnormally, that
    /// is, with an exit reason other than normal, shutdown.
    Transient,
}

/// Defines how a child process is to be terminated.
#[derive(Debug, Clone)]
pub enum Shutdown {
    /// The child process is terminated by sending a normal exit signal.
    BrutalKill,
    /// The child process is terminated by sending a shutdown signal.
    Timeout(Duration),
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::Timeout(Duration::from_secs(5))
    }
}

/// Specifies whether the child process is a supervisor or a worker.
#[derive(Debug, Clone, Default)]
pub enum ChildType {
    /// A child process that is supervised by the supervisor.
    #[default]
    Worker,
    /// A child process that is supervised by the supervisor and is also a supervisor.
    SupervisorWorker,
}

pub(super) struct ChildState {
    pub handle: commonware_runtime::Handle<Result<(), peer::Error>>,
    pub spec: ChildSpec,
    pub restarts: usize,
    pub stopped: bool,
}
