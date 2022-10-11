use log::{error, trace, warn};
use std::sync::Arc;

use crate::actor::{
    core::{ActorCore, LopperTask},
    receiver::{ActorReceiver, FnHandlerContext, FunctionHandler},
    ActorAddr, ActorAddrInner, ActorId, ActorWeakAddr, DiscoveryKey,
};
use crate::message::{Message, MessageSender};
use crate::system::{executor::ActorExecutor, guardian::ActorGuardianType, SystemRef};

#[cfg(all(unix, feature = "ipc-cluster"))]
use crate::{actor::core::RemoteActorCore, system::SystemWeakRef};

/// ActorSpawnItem
pub struct ActorSpawnItem<R: ActorReceiver> {
    pub(crate) executor: ActorExecutor,
    pub(crate) address: ActorAddr<R>,
    pub(crate) looper_tasks: Vec<LopperTask<R>>,
}

/// ActorBuilderConfig
pub struct ActorBuilderConfig {
    pub actor_tag: Option<String>,
    pub pool_size: Option<usize>,
    pub discovery: DiscoveryKey,
}
impl Default for ActorBuilderConfig {
    fn default() -> Self {
        Self {
            actor_tag: None,
            pool_size: None,
            discovery: DiscoveryKey::None,
        }
    }
}

/// ActorBuilder provides actor creation functionality.
/// Actor creation needs to follow specific steps, so the only way to create
/// actors is by using the actor builder.
pub struct ActorBuilder;

impl ActorBuilder {
    /// Create user actor with the provided receiver.
    pub fn create<R, Fac>(
        factory: Fac,
        system: &SystemRef,
        config: ActorBuilderConfig,
    ) -> Option<ActorSpawnItem<R>>
    where
        R: ActorReceiver,
        Fac: Fn() -> R + Send + Sync + 'static,
    {
        if let Some(_) = config.pool_size {
            warn!("creation of pool not allowed through this method. use create_pool()");
            None
        } else {
            match Self::create_actor(factory, &ActorGuardianType::User, system, config) {
                Ok(item) => Some(item),
                Err(e) => {
                    error!("ActorBuilder_create_error {:?}", e);
                    None
                }
            }
        }
    }

    /// Create user actor-pool for CPU bound computation services with the provided factory.
    pub fn create_pool<R, Fac>(
        factory: Fac,
        system: &SystemRef,
        config: ActorBuilderConfig,
    ) -> Option<ActorSpawnItem<R>>
    where
        R: ActorReceiver,
        Fac: Fn() -> R + Send + Sync + 'static,
    {
        if let Some(_) = config.pool_size {
            match Self::create_actor(factory, &ActorGuardianType::User, system, config) {
                Ok(item) => Some(item),
                Err(e) => {
                    trace!("create_pool_error {:?}", e);
                    None
                }
            }
        } else {
            trace!("create_pool_error: pool_size not provided");
            None
        }
    }

    /// Create user actor with the provided function-message handler for a single Message type.
    pub fn create_fn<F, M>(
        f: F,
        system: &SystemRef,
    ) -> Option<ActorSpawnItem<FunctionHandler<F, M>>>
    where
        F: (Fn(M, &mut FnHandlerContext) -> M::Result) + Send + Sync + 'static + Clone,
        M: Message + Send + 'static,
    {
        let factory = move || FunctionHandler::new(f.clone());
        match Self::create_actor(
            factory,
            &ActorGuardianType::User,
            system,
            ActorBuilderConfig::default(),
        ) {
            Ok(item) => Some(item),
            Err(e) => {
                trace!("ActorBuilder_create_error {:?}", e);
                None
            }
        }
    }

    /// Internal helper function to create the actor.
    fn create_cyclic<R: ActorReceiver, F>(
        tasks: &mut Vec<LopperTask<R>>,
        executor: &mut ActorExecutor,
        build_inner_fn: F,
    ) -> Arc<ActorAddrInner<R>>
    where
        F: FnOnce(
            &ActorWeakAddr<R>,
            &mut Vec<LopperTask<R>>,
            &mut ActorExecutor,
        ) -> ActorAddrInner<R>,
    {
        let inner = Arc::new_cyclic(|weak_inner| {
            let weak_addr = ActorWeakAddr::new(weak_inner.clone());
            build_inner_fn(&weak_addr, tasks, executor)
        });

        inner
    }

    /// Create an actor with the provides receiver and supervised by the
    /// provided guardian type.
    pub(crate) fn create_actor<R, Fac>(
        factory: Fac,
        g_type: &ActorGuardianType,
        system: &SystemRef,
        config: ActorBuilderConfig,
    ) -> Result<ActorSpawnItem<R>, ActorCreationError>
    where
        R: ActorReceiver,
        Fac: Fn() -> R + Send + Sync + 'static,
    {
        let mut looper_tasks: Vec<LopperTask<R>> = Vec::new();
        let mut executor_type: ActorExecutor = ActorExecutor::System;

        // [todo] add a check that g_type is not Remote.
        let inner = Self::create_cyclic(
            &mut looper_tasks,
            &mut executor_type,
            |weak_addr, tasks, executor| {
                let mut id = None;
                let mut sender = None;

                if let Ok((core, ts, exec)) = ActorCore::create_core(
                    factory,
                    system,
                    g_type,
                    weak_addr.clone(),
                    config.pool_size,
                ) {
                    sender = Some(MessageSender::LocalSender { core });
                    *tasks = ts;
                    *executor = exec;
                }

                if let Ok(aid) = ActorId::generate(
                    system.get_id().clone(),
                    g_type.clone(),
                    config.actor_tag,
                    config.discovery,
                ) {
                    id = Some(aid);
                }

                ActorAddrInner { id, sender }
            },
        );

        if inner.id.is_none() {
            return Err(ActorCreationError::IdGenerationError); // this error has preference
        } else if inner.sender.is_none() {
            return Err(ActorCreationError::MessageSenderCreationError);
        }

        let address = ActorAddr::new(inner);
        Ok(ActorSpawnItem {
            executor: executor_type,
            address,
            looper_tasks,
        })
    }

    /// Internal helper function to create the remote actor addrs.
    #[cfg(all(unix, feature = "ipc-cluster"))]
    fn create_cyclic_remote<R: ActorReceiver, F>(
        id: ActorId,
        system: SystemWeakRef,
        build_inner_fn: F,
    ) -> Arc<ActorAddrInner<R>>
    where
        F: FnOnce(ActorId, SystemWeakRef, ActorWeakAddr<R>) -> ActorAddrInner<R>,
    {
        let inner = Arc::new_cyclic(|weak_inner| {
            let weak_addr = ActorWeakAddr::new(weak_inner.clone());
            build_inner_fn(id, system, weak_addr)
        });

        inner
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub(crate) fn create_remote_addr<R>(
        id: ActorId,
        system: SystemWeakRef,
    ) -> Result<ActorAddr<R>, ActorCreationError>
    where
        R: ActorReceiver,
    {
        let inner = Self::create_cyclic_remote(id, system, |aid, system, weak_addr| {
            let remote_core = RemoteActorCore::<R>::new(weak_addr, system);
            let sender = Some(MessageSender::RemoteSender {
                core: Arc::new(remote_core),
            });
            ActorAddrInner {
                id: Some(aid),
                sender,
            }
        });

        Ok(ActorAddr::new(inner))
    }

    pub(crate) fn create_unresolved_addr<R>(id: ActorId) -> ActorAddr<R>
    where
        R: ActorReceiver,
    {
        ActorAddr::new(Arc::new(ActorAddrInner {
            id: Some(id),
            sender: None,
        }))
    }
}

#[derive(Debug)]
pub enum ActorCreationError {
    IdGenerationError,
    MessageSenderCreationError,
}
