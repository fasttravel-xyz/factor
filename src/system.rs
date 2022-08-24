pub(crate) mod guardian;

use futures::{
    executor::{ThreadPool, ThreadPoolBuilder},
    Future,
};
use log::trace;
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    sync::{Arc, Weak},
    time::Duration,
};

use crate::actor::{
    builder::ActorSpawnItem,
    receiver::{ActorReceiver, BasicContext},
    ActorRef, Address,
};
use crate::common;
use crate::system::guardian::{
    ActorGuardianType, BoxedActorGuardian, GuardianRef, Guardians, Supervisor,
};

/// System Reference. Cloning only clones the light reference.
pub type SystemRef = Arc<ActorSystem>;
/// System Weak Reference. Upgradable to SystemRef.
pub type SystemWeakRef = Weak<ActorSystem>;

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
    ActorCreated(Address),
    ActorRestarted(Address),
    ActorTerminated(Address),
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

/// System Id to uniquely identify a system
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemId {
    pub(crate) uuid: common::EntityId,
    pub(crate) host_id: u16,
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
/// It is recommended to have a single system per process.
pub struct ActorSystem {
    id: SystemId,
    guardians: Guardians,
    executor: ThreadPool,
    per_actor_run_msg_limit: u16,
    per_actor_time_slice: Duration,
    // cluster: ClusterHandle, // [todo] cluster implmentation is low priority.
}

impl ActorSystem {
    /// Internal helper function to create the system and the guardians
    fn new_cyclic(weak_me: SystemWeakRef, config: &ActorSystemCofig, name: Option<String>) -> Self {
        let host_id = 0;
        let uuid = common::generate_entity_id().unwrap();
        let id = SystemId {
            uuid,
            host_id,
            name,
        };

        let guardians = Guardians::new(weak_me).unwrap();

        let mut builder = ThreadPoolBuilder::new();
        builder.name_prefix("system_mt_executor_");
        if let Some(size) = config.threadpool_size {
            builder.pool_size(size);
        }

        let executor = builder.create().unwrap(); // let it panic if error
        ActorSystem {
            id,
            guardians,
            executor,
            per_actor_run_msg_limit: config.per_actor_run_msg_limit,
            per_actor_time_slice: config.per_actor_time_slice,
        }
    }

    /// Function to create the system.
    pub(crate) fn create_system(name: Option<String>) -> SystemRef {
        let config = ActorSystemCofig::default();
        let sys = Arc::new_cyclic(|w| ActorSystem::new_cyclic(w.clone(), &config, name));
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

    /// get the guardian reference (ActorRef of the delegate guardian actor).
    pub(crate) fn get_guardian_ref(&self, g_type: &ActorGuardianType) -> Option<GuardianRef> {
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

    /// Run the actor spawn item in the system executor that was created using  
    /// "ActorBuilder::create" or "ActorBuilder::create_fn".
    /// This method consumes "item", which is what we want.
    pub fn run_actor<R>(&self, item: ActorSpawnItem<R>) -> ActorRef<R>
    where
        R: ActorReceiver<Context = BasicContext<R>>,
    {
        self.spawn_ok(item.looper_task.get_future());
        <BoxedActorGuardian as Supervisor>::add_actor(
            self.guardians.user.as_ref(),
            item.address.clone(),
        );

        // [todo] should we wait here before returning the address??
        if let Err(e) = item
            .address
            .tell_sys(SystemMessage::Notification(SystemNotification::ActorInit))
        {
            trace!("ActorCore::stop::error {:?}", e);
        }

        item.address
    }
}
