#[cfg(all(unix, feature = "ipc-cluster"))]
pub(crate) mod cluster;

pub(crate) mod executor;
pub(crate) mod guardian;

use chrono::{DateTime, Utc};
use futures::Future;
use log::trace;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    sync::{Arc, Weak},
    time::Duration,
};

use self::{
    executor::{
        ActorExecutor, ExecutorScheduleError, ExecutorSpawnError, ExecutorTask,
        ExecutorTaskFactory, RemoteJoinHandle, ScheduledTaskHandle, ThreadPoolExecutor,
    },
    guardian::{ActorGuardianType, BoxedActorGuardian, GuardianAddr, Guardians, Supervisor},
};
use crate::common;
use crate::{
    actor::{
        builder::ActorSpawnItem,
        receiver::{ActorReceiver, BasicContext},
        ActorAddr, ActorId, Addr,
    },
    builder::ActorBuilder,
};

#[cfg(all(unix, feature = "ipc-cluster"))]
use cluster::NodeCreationConfig;
#[cfg(not(feature = "ipc-cluster"))]
use log::warn;

/// System Reference. Cloning only clones the light reference.
pub type SystemRef = Arc<ActorSystem>;
/// System Weak Reference. Upgradable to SystemRef.
pub(crate) type SystemWeakRef = Weak<ActorSystem>;

// System messages are Messages with this enum as payload.
pub enum SystemMessage {
    Notification(SystemNotification),
    Command(SystemCommand),
    Event(SystemEvent),
}

// System notifications to Actor or its Guardian.
pub enum SystemNotification {
    ActorInit,
    ActorFailed,
}

// System commands to Actor.
pub enum SystemCommand {
    ActorPause(bool),
    ActorRestart,
    ActorStop,
}

// PubSub Messages/Events for subscription by ineterested Actor.
pub enum SystemEvent {
    ActorCreated(Addr),
    ActorRestarted(Addr),
    ActorTerminated(Addr),
}

#[allow(dead_code)]
pub enum ActorState {
    Running,
    Paused,
    Stopping,
    Stopped,
}

impl fmt::Display for SystemEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg: &str;
        match self {
            _ => msg = "SystemEvent",
        }

        write!(f, "({})", msg)
    }
}

/// Cluster Node Id
pub(crate) type NodeId = u16;

/// System Id to uniquely identify a system
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemId {
    pub(crate) uuid: common::EntityId,
    pub(crate) node_id: NodeId,
    pub(crate) name: Option<String>,
}

/// Configurations for the system creation.
pub struct ActorSystemCofig {
    pub threadpool_size: Option<usize>,
    pub per_actor_run_msg_limit: u16,
    pub per_actor_time_slice: Duration,
}

impl Default for ActorSystemCofig {
    fn default() -> Self {
        ActorSystemCofig {
            threadpool_size: None,
            per_actor_run_msg_limit: 20,
            per_actor_time_slice: Duration::from_millis(10),
        }
    }
}

/// ActorSystem is the main worker unit and provides the necessary services.
/// Only one system is allowed per process.
pub struct ActorSystem {
    id: SystemId,
    guardians: Guardians,
    executor: Box<ThreadPoolExecutor>,
    per_actor_run_msg_limit: u16,
    per_actor_time_slice: Duration,
    // used when feature ipc-custer is active.
    #[cfg_attr(not(feature = "ipc-cluster"), allow(dead_code))]
    weak_self: SystemWeakRef,
}

impl ActorSystem {
    /// Internal helper function to create the system and the guardians
    fn new_cyclic(
        weak_self: SystemWeakRef,
        config: &ActorSystemCofig,
        name: Option<String>,
        node_id: Option<u16>,
    ) -> Self {
        let node_id = node_id.unwrap_or(0);
        let uuid = common::generate_entity_id().unwrap();
        let id = SystemId {
            uuid,
            node_id,
            name,
        };

        let guardians = Guardians::new(weak_self.clone()).unwrap();

        let executor = Box::new(ThreadPoolExecutor::new(
            config.threadpool_size,
            Some("_system_mt_"),
        ));
        ActorSystem {
            id,
            guardians,
            executor,
            per_actor_run_msg_limit: config.per_actor_run_msg_limit,
            per_actor_time_slice: config.per_actor_time_slice,
            weak_self,
        }
    }

    /// Function to create the system.
    pub(crate) fn create_system(name: Option<String>, node_id: Option<u16>) -> SystemRef {
        let config = ActorSystemCofig::default();
        let sys = Arc::new_cyclic(|w| ActorSystem::new_cyclic(w.clone(), &config, name, node_id));
        sys.run_guardians();
        sys
    }

    /// Get the system id.
    pub fn get_id(&self) -> &SystemId {
        &self.id
    }

    /// Per actor message processing limit for each mailbox loop.
    pub fn get_per_actor_run_msg_limit(&self) -> u16 {
        self.per_actor_run_msg_limit
    }

    /// Per actor maximum time slice for each mailbox loop.
    pub fn get_per_actor_time_slice(&self) -> Duration {
        self.per_actor_time_slice
    }

    /// get the guardian reference (ActorAddr of the delegate guardian actor).
    pub(crate) fn get_guardian_ref(&self, g_type: &ActorGuardianType) -> Option<GuardianAddr> {
        match g_type {
            ActorGuardianType::User => self.guardians.user.get_delegate(),
            ActorGuardianType::System => self.guardians.sys.get_delegate(),
            ActorGuardianType::Remote => self.guardians.remote.get_delegate(),
            _ => None,
        }
    }

    fn run_guardians(&self) {
        self.guardians.sys.run(Arc::downgrade(&self.guardians.sys));
        self.guardians
            .user
            .run(Arc::downgrade(&self.guardians.user));
        self.guardians
            .remote
            .run(Arc::downgrade(&self.guardians.remote));
    }

    /// Spawn a future in the system executor.
    pub fn spawn_ok<Fut>(&self, task: Fut)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        self.executor.spawn_ok(task)
    }

    /// Spawn a future in the system executor and get a handle to the output.
    pub fn spawn_with_handle<Fut>(
        &self,
        task: Fut,
    ) -> Result<RemoteJoinHandle<Fut::Output>, ExecutorSpawnError>
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        self.executor.spawn_with_handle(task)
    }

    /// Schedule a Task once.
    pub fn schedule_once(
        &self,
        task: ExecutorTask,
        delay: Duration,
    ) -> Result<ScheduledTaskHandle, ExecutorScheduleError> {
        self.executor.schedule_once(task, delay)
    }

    /// Schedule a Task at an Utc DateTime.
    pub fn schedule_once_at(
        &self,
        task: ExecutorTask,
        at: DateTime<Utc>,
    ) -> Result<ScheduledTaskHandle, ExecutorScheduleError> {
        self.executor.schedule_once_at(task, at)
    }

    /// Schedule a Task that repeats at regular interval.
    ///
    /// @param factory: task factory.
    /// @param delay: initial delay.
    /// @param interval: repeat interval.
    ///
    pub fn schedule_repeat(
        &self,
        factory: ExecutorTaskFactory,
        delay: Duration,
        interval: Duration,
    ) -> Result<ScheduledTaskHandle, ExecutorScheduleError> {
        self.executor.schedule_repeat(factory, delay, interval)
    }

    /// Run the actor spawn item in the system executor that was created using  
    /// "ActorBuilder::create" or "ActorBuilder::create_fn" or  "ActorBuilder::create_pool" .
    /// This method consumes "item", which is what we want.
    pub fn run_actor<R>(&self, item: ActorSpawnItem<R>) -> ActorAddr<R>
    where
        R: ActorReceiver<Context = BasicContext<R>>,
    {
        let mut item = item;
        match item.executor {
            ActorExecutor::System => {
                if item.looper_tasks.len() == 1 {
                    if let Some(task) = item.looper_tasks.pop() {
                        self.spawn_ok(task.get_future());
                        <BoxedActorGuardian as Supervisor>::add_actor(
                            self.guardians.user.as_ref(),
                            item.address.clone(),
                        );
                    }
                } else {
                    trace!("ActorCore_ActorExecutor_System_error: more than one looper");
                }
            }
            ActorExecutor::Pool(executor) => {
                let mut tasks = Vec::with_capacity(item.looper_tasks.len());
                for task in item.looper_tasks {
                    tasks.push(task.get_future());
                }
                executor.spawn_ok_distribute(tasks);
                <BoxedActorGuardian as Supervisor>::add_actor(
                    self.guardians.user.as_ref(),
                    item.address.clone(),
                );
            }
        }

        // [todo] should we wait here before returning the address??
        if let Err(e) = item
            .address
            .tell_sys(SystemMessage::Notification(SystemNotification::ActorInit))
        {
            trace!("ActorCore::stop::error {:?}", e);
        }

        // insert in cluster receptionist
        self.add_to_addr_book(item.address.get_address());

        item.address
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub(crate) fn resolve_actor_addr<R: ActorReceiver>(
        &self,
        id: &ActorId,
    ) -> Option<ActorAddr<R>> {
        // We don't do id.check_system_id() as currently we only support one system per process.
        if id.check_node_id(self.id.node_id) {
            // local actor
            match id.guardian() {
                ActorGuardianType::User => return self.guardians.user.get_actor(id.unique_key()),
                ActorGuardianType::System => return self.guardians.sys.get_actor(id.unique_key()),
                _ => {}
            }
        } else {
            // remote actor
            let remote_addr = self
                .guardians
                .remote
                .get_actor::<R>(id.unique_key())
                .or_else(|| {
                    let new_addr =
                        ActorBuilder::create_remote_addr::<R>(id.clone(), self.weak_self.clone())
                            .ok();

                    new_addr.map(|addr| {
                        self.guardians.remote.add_actor(addr.clone());
                        addr
                    })
                });

            return remote_addr;
        }

        None
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub async fn get_remote_addr<R>(&self, node_id: NodeId, key: &str) -> Option<ActorAddr<R>>
    where
        R: ActorReceiver,
    {
        if let Some(node) = cluster::PROCESS_CLUSTER_NODE.get() {
            if let Some(client) = node.get_node_client(node_id).await {
                return client.get_remote_addr::<R>(key).await;
            }
        }

        None
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub fn add_to_addr_book(&self, addr: Addr) {
        if let Some(node) = cluster::PROCESS_CLUSTER_NODE.get() {
            node.add_to_addr_book(addr);
        }
    }

    #[cfg(not(feature = "ipc-cluster"))]
    pub fn add_to_addr_book(&self, _addr: Addr) {}

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub(crate) async fn get_node_client(
        &self,
        node_id: cluster::NodeId,
    ) -> Option<cluster::RemoteNodeClient> {
        if let Some(node) = cluster::PROCESS_CLUSTER_NODE.get() {
            return node.get_node_client(node_id).await;
        }

        None
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub async fn spawn_worker_node(&self, config: NodeCreationConfig) -> Result<NodeId, ()> {
        if let Some(node) = cluster::PROCESS_CLUSTER_NODE.get() {
            return node.spawn_worker_node(config).await;
        }

        Err(())
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub async fn shutdown_worker_node(&self, node_id: &NodeId) -> i8 {
        if let Some(node) = cluster::PROCESS_CLUSTER_NODE.get() {
            return node.shutdown_worker_node(node_id).await;
        }

        1
    }
}

#[cfg(all(unix, feature = "ipc-cluster"))]
pub(crate) fn resolve_actor_addr_in_current_node<R>(id: ActorId) -> ActorAddr<R>
where
    R: ActorReceiver,
{
    cluster::PROCESS_CLUSTER_NODE
        .get()
        .and_then(|node| node.system().resolve_actor_addr(&id))
        .unwrap_or_else(move || ActorBuilder::create_unresolved_addr(id))
}

#[cfg(not(feature = "ipc-cluster"))]
pub(crate) fn resolve_actor_addr_in_current_node<R>(id: ActorId) -> ActorAddr<R>
where
    R: ActorReceiver,
{
    warn!("resolve_supported_only_for_feature=ipc-cluster. placeholder called.");
    ActorBuilder::create_unresolved_addr(id)
}
