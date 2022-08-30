use log::trace;
use std::sync::Arc;

use crate::actor::{
    core::{ActorCore, CoreExecutorType, LopperTask},
    receiver::{ActorReceiver, FnHandlerContext, FunctionHandler},
    ActorAddr, ActorAddrInner, ActorId, ActorWeakAddr,
};
use crate::message::{Message, MessageSender};
use crate::system::{guardian::ActorGuardianType, SystemRef};

// ActorSpawnItem
pub struct ActorSpawnItem<R: ActorReceiver> {
    pub(crate) executor: CoreExecutorType,
    pub(crate) address: ActorAddr<R>,
    pub(crate) looper_tasks: Vec<LopperTask<R>>,
}

/// ActorBuilder provides actor creation functionality.
/// Actor creation needs to follow specific steps, so the only way to create
/// actors is by using the actor builder.
pub struct ActorBuilder;

impl ActorBuilder {
    /// Create user actor with the provided receiver.
    pub fn create<R, Fac>(factory: Fac, system: &SystemRef) -> Option<ActorSpawnItem<R>>
    where
        R: ActorReceiver,
        Fac: Fn() -> R + Send + Sync + 'static,
    {
        match Self::create_actor(factory, &ActorGuardianType::User, system, None) {
            Ok(item) => Some(item),
            Err(e) => {
                trace!("ActorBuilder_create_error {:?}", e);
                None
            }
        }
    }

    /// Create user actor-pool for CPU bound computation services with the provided factory.
    pub fn create_pool<R, Fac>(
        factory: Fac,
        system: &SystemRef,
        pool_size: usize,
    ) -> Option<ActorSpawnItem<R>>
    where
        R: ActorReceiver,
        Fac: Fn() -> R + Send + Sync + 'static,
    {
        if pool_size > 0 {
            match Self::create_actor(factory, &ActorGuardianType::User, system, Some(pool_size)) {
                Ok(item) => Some(item),
                Err(e) => {
                    trace!("ActorBuilder_create_pool_error {:?}", e);
                    None
                }
            }
        } else {
            trace!("ActorBuilder_create_pool_error: pool_size less than one");
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
        match Self::create_actor(factory, &ActorGuardianType::User, system, None) {
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
        executor: &mut CoreExecutorType,
        data_fn: F,
    ) -> Arc<ActorAddrInner<R>>
    where
        F: FnOnce(
            &ActorWeakAddr<R>,
            &mut Vec<LopperTask<R>>,
            &mut CoreExecutorType,
        ) -> ActorAddrInner<R>,
    {
        let inner = Arc::new_cyclic(|weak_inner| {
            let weak_ref = ActorWeakAddr::new(weak_inner.clone());
            data_fn(&weak_ref, tasks, executor)
        });

        inner
    }

    /// Create an actor with the provides receiver and supervised by the
    /// provided guardian type.
    pub(crate) fn create_actor<R, Fac>(
        factory: Fac,
        g_type: &ActorGuardianType,
        system: &SystemRef,
        pool_size: Option<usize>,
    ) -> Result<ActorSpawnItem<R>, ActorCreationError>
    where
        R: ActorReceiver,
        Fac: Fn() -> R + Send + Sync + 'static,
    {
        let mut looper_tasks: Vec<LopperTask<R>> = Vec::new();
        let mut executor_type: CoreExecutorType = CoreExecutorType::Single;

        // [todo] add a check that g_type is not Remote.
        let inner = Self::create_cyclic(
            &mut looper_tasks,
            &mut executor_type,
            |weak_ref, tasks, executor| {
                let mut id = None;
                let mut sender = None;

                if let Ok((core, ts, exec)) =
                    ActorCore::create_core(factory, system, g_type, weak_ref.clone(), pool_size)
                {
                    sender = Some(MessageSender::LocalSender { core });
                    *tasks = ts;
                    *executor = exec;
                }

                if let Ok(aid) = ActorId::generate(system.get_id().clone(), g_type.clone()) {
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
}

#[derive(Debug)]
pub enum ActorCreationError {
    IdGenerationError,
    MessageSenderCreationError,
}
