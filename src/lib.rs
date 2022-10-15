//! factor is a Rust framework to build concurrent services using the Actor Model.
//!
//! Actors are worker units that communicate exclusively using messages.
//! factor uses futures::executor::ThreadPool for concurrency, so actors share
//! threads and tasks of same actor may be executing in different threads.
//!
//! ## factor provides:
//! - "Typed" messages.
//! - "Ask" and "Tell" patterns.
//! - Uses "futures" for asynchronous message handling.
//! - Concurrency using "futures::executor::ThreadPool".
//! - Unbounded channels for messages (this might change.)
//! - "ActorPool" for running a CPU bound computation service on multiple dedicated threads.
//! - Async response.
//! - Granular locks when possible.
//! - Runs on stable Rust 1.60+
//! - "Ipc-cluster" with remote ipc-nodes.
//! - Task and Message Scheduling with "schedule_once", "schedule_once_at", and "schedule_repeat".
//! - Quick "LocalPool" access.
//! - Simple functional message handlers "[experimental.may.get.removed]".
//!

#![deny(unreachable_pub, private_in_public)]
#![forbid(unsafe_code)]

// modules of the crate
mod actor;
mod common;
mod message;
mod system;

// public interface of the crate. Use the prelude for glob import.
pub use actor::builder::{self, ActorBuilder, ActorBuilderConfig};
pub use actor::receiver::{
    self, ActorReceiver, ActorReceiverContext, BasicContext, FnHandlerContext,
};
pub use actor::{ActorAddr, ActorUniqueKey, ActorWeakAddr, Addr, DiscoveryKey, MessageAddr};
pub use message::{
    handler::{
        MessageHandler, MessageResponse, MessageResponseType, ReplyTo, ReplyToRef, ResponseFuture,
        ResponseResult,
    },
    Message, MessageSendError,
};
pub use system::{
    executor::{ExecutorTask, ExecutorTaskFactory, RemoteJoinHandle, ScheduledTaskHandle},
    ActorState, ActorSystemCofig, SystemCommand, SystemEvent, SystemMessage, SystemRef,
};

#[cfg(all(unix, feature = "ipc-cluster"))]
pub use crate::{
    message::{
        handler::{
            MessageClusterHandler, MessageClusterResponse, ReplyToCluster, ReplyToRefCluster,
        },
        MessageCluster,
    },
    system::cluster::{NodeCreationConfig, NodeId, RemoteMessageTypeProvider},
};

pub mod prelude {
    //! The 'factor' prelude.
    //!
    //!
    //! ```
    //! # #![allow(unused_imports)]
    //! use factor::prelude::*;
    //! ```

    #[doc(hidden)]
    pub use crate::{
        actor::{
            builder::{self, ActorBuilder, ActorBuilderConfig},
            receiver::{ActorReceiver, ActorReceiverContext, BasicContext, FnHandlerContext},
            ActorAddr, ActorUniqueKey, ActorWeakAddr, DiscoveryKey,
        },
        message::{
            handler::{MessageHandler, MessageResponseType, ResponseFuture, ResponseResult},
            Message, MessageSendError,
        },
        system::{
            executor::ScheduledTaskHandle, ActorSystem, SystemCommand, SystemMessage, SystemRef,
        },
    };

    #[doc(hidden)]
    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub use crate::{
        message::{handler::MessageClusterHandler, MessageCluster},
        system::cluster::{NodeCreationConfig, NodeId, RemoteMessageTypeProvider},
    };
}

////////////////////////////////////////////////////////////////////////////////

use futures::executor::LocalPool;
use futures::Future;
use log::trace;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static FACTOR_THREAD_RESOURCES: RefCell<FactorLocalResources> = RefCell::new(FactorLocalResources::new());
}

/// Thread local resources
struct FactorLocalResources {
    pool: Box<LocalPool>,
}
impl FactorLocalResources {
    fn new() -> Self {
        Self {
            pool: Box::new(LocalPool::new()),
        }
    }
}

/// Initialize the system per process when running in single node configuration.
///
/// Initialization allowed only once per process. Subsequent calls after the first
/// call will result in panic.
pub fn init_system(name: Option<String>) -> system::SystemRef {
    init_system_cluster(name, None)
}

/// Initialize the cluster-node and system per process when running in cluster configuration.
///
/// Initialization allowed only once per process. Subsequent calls after the first
/// call will result in panic.
///
///
/// Usage:
/// ```ignore
///
/// // Register message and message-handler pairs for all remote message
/// // types (see examples/example_ipc_remote_ask for reference). We could
/// // provide a macro to make this ergonomic.
/// let mut provider = RemoteMessageTypeProvider::new();
/// provider.register::<MockActor, MockMessage>();
/// let node_id = 0; // the node with node_id = 0 is considered as the main node.
/// let system = init_cluster(node_id, Some("main_node".to_owned()), provider);
///
/// ```
///
/// `factor` provides only ipc-remote actors where cluster-nodes are process-nodes,
/// this is the only cluster model that factor intends to provide. `factor` doesn't
/// intend to provide any other cluster model e.g. multi-machine (physical/virtual)
/// cluster for static or dynamic horizontal-scaling, etc. as there are better and
/// established solutions already available that `factor` could utilize.
///
/// This execution model is targeted towards applications that need to keep adding
/// dedicated resources in separate processes. This model is different from:
///     * a cluster running with a pre-defined sets of node and all nodes sharing
///       the application load (static)
///     * a cluster adding new nodes when a load-balancing threshold is reached
///       and it needs more resources (dynamic).
///
/// `factor` cluster provides an execution model in which an actor-group (a group of
/// related actors) is launched in a child process of the main process (or main-node).
/// By default worker nodes don't store or receive information about other worker
/// nodes, as number of worker-nodes will keep on growing and usually different
/// worker nodes will have unrelated actors. A worker node retrieves the information
/// about another worker node only when required (e.g. to resolve the address
/// of an actor that is running in a different worker node.)
///
#[cfg(all(unix, feature = "ipc-cluster"))]
pub async fn init_cluster(
    node_id: u16,
    name: Option<String>,
    r_msg_type_provider: RemoteMessageTypeProvider,
) -> system::SystemRef {
    system::cluster::init_cluster(node_id, name, r_msg_type_provider).await
}

/// Initialize the system per process.
pub(crate) fn init_system_cluster(name: Option<String>, node_id: Option<u16>) -> system::SystemRef {
    static INITIALIZED: AtomicBool = AtomicBool::new(false);

    if INITIALIZED.load(Ordering::Acquire) {
        panic!("factor_process_system_already_initialized")
    }

    trace!("factor_process_system_initialized");
    INITIALIZED.store(true, Ordering::Relaxed);

    system::ActorSystem::create_system(name, node_id)
}

/// Spawn tasks into the local-pool.
///
/// NOTE: This function is provided as an convenience API and internally uses [LocalPool](futures::executor::LocalPool).
/// So it has the same limitations as `LocalPool`, i.e. a subsequent call to local_run()/local_run_unitll()
/// will panic if called in a thread that is within the dynamic range of another executor e.g. ThreadPool
/// executor. For details refer to [futures_executor::enter](futures_executor::enter).
///
pub fn local_spawn<Fut>(fut: Fut) -> Result<(), futures::task::SpawnError>
where
    Fut: 'static + futures::Future<Output = ()>,
{
    FACTOR_THREAD_RESOURCES
        .with(|f| futures::task::LocalSpawnExt::spawn_local(&f.borrow().pool.spawner(), fut))
}

/// Run all tasks in the local-pool to completion.
///
/// NOTE: This function is provided as an convenience API and internally uses [LocalPool](futures::executor::LocalPool).
/// So it has the same limitations as LocalPool, i.e. a subsequent call to local_run()/local_run_unitll()
/// will panic if called in a thread that is within the dynamic range of another executor e.g. ThreadPool
/// executor. For details refer to [futures_executor::enter](futures_executor::enter).
///
pub fn local_run() {
    FACTOR_THREAD_RESOURCES.with(|f| f.borrow_mut().pool.as_mut().run())
}

/// Runs all the tasks in the pool until the given future completes.
///
/// NOTE: This function is provided as an convenience API and internally uses [LocalPool](futures::executor::LocalPool).
/// So it has the same limitations as LocalPool, i.e. a subsequent call to local_run()/local_run_unitll()
/// will panic if called in a thread that is within the dynamic range of another executor e.g. ThreadPool
/// executor. For details refer to [futures_executor::enter](futures_executor::enter).
///
pub fn local_run_until<F: Future>(future: F) -> F::Output {
    FACTOR_THREAD_RESOURCES.with(|f| f.borrow_mut().pool.as_mut().run_until(future))
}

pub fn block_on<F: Future>(future: F) -> F::Output {
    futures::executor::block_on(future)
}
