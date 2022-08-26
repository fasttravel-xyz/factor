use log::trace;
use std::sync::Arc;

use crate::actor::{
    core::{ActorCore, LopperTask},
    receiver::{ActorReceiver, FnHandlerContext, FunctionHandler},
    ActorAddr, ActorAddrInner, ActorId, ActorWeakAddr,
};
use crate::message::{Message, MessageSender};
use crate::system::{guardian::ActorGuardianType, SystemRef};

// ActorSpawnItem
pub struct ActorSpawnItem<R: ActorReceiver> {
    pub(crate) address: ActorAddr<R>,
    pub(crate) looper_task: LopperTask<R>,
}

/// ActorBuilder provides actor creation functionality.
/// Actor creation needs to follow specific steps, so the only way to create
/// actors is by using the actor builder.
pub struct ActorBuilder;

impl ActorBuilder {
    /// Create user actor with the provided receiver.
    pub fn create<R: ActorReceiver>(receiver: R, system: &SystemRef) -> Option<ActorSpawnItem<R>> {
        match Self::create_actor(receiver, &ActorGuardianType::User, system) {
            Ok(item) => Some(item),
            Err(e) => {
                trace!("ActorBuilder_create_error {:?}", e);
                None
            }
        }
    }

    /// Create user actor with the provided function-message handler for a single Message type.
    pub fn create_fn<F, M>(
        f: F,
        system: &SystemRef,
    ) -> Option<ActorSpawnItem<FunctionHandler<F, M>>>
    where
        F: (Fn(M, &mut FnHandlerContext) -> M::Result) + Send + Sync + 'static,
        M: Message + Send + 'static,
    {
        let receiver = FunctionHandler::new(f);
        match Self::create_actor(receiver, &ActorGuardianType::User, system) {
            Ok(item) => Some(item),
            Err(e) => {
                trace!("ActorBuilder_create_error {:?}", e);
                None
            }
        }
    }

    /// Internal helper function to create the actor.
    fn create_cyclic<R: ActorReceiver, F>(
        task: &mut LopperTask<R>,
        data_fn: F,
    ) -> Arc<ActorAddrInner<R>>
    where
        F: FnOnce(&ActorWeakAddr<R>, &mut LopperTask<R>) -> ActorAddrInner<R>,
    {
        let inner = Arc::new_cyclic(|weak_inner| {
            let weak_ref = ActorWeakAddr::new(weak_inner.clone());
            data_fn(&weak_ref, task)
        });

        inner
    }

    /// Create an actor with the provides receiver and supervised by the
    /// provided guardian type.
    pub(crate) fn create_actor<R: ActorReceiver>(
        receiver: R,
        g_type: &ActorGuardianType,
        system: &SystemRef,
    ) -> Result<ActorSpawnItem<R>, ActorCreationError> {
        let mut looper_task = LopperTask::default();

        // [todo] add a check that g_type is not Remote.
        let inner = Self::create_cyclic(&mut looper_task, |weak_ref, task| {
            let mut id = None;
            let mut sender = None;

            if let Ok((core, t)) =
                ActorCore::create_core(receiver, system, g_type, weak_ref.clone())
            {
                sender = Some(MessageSender::LocalSender { core });
                *task = t;
            }

            if let Ok(aid) = ActorId::generate(system.get_id().clone(), g_type.clone()) {
                id = Some(aid);
            }

            ActorAddrInner { id, sender }
        });

        if inner.id.is_none() {
            return Err(ActorCreationError::IdGenerationError); // this error has preference
        } else if inner.sender.is_none() {
            return Err(ActorCreationError::MessageSenderCreationError);
        }

        let address = ActorAddr::new(inner);
        Ok(ActorSpawnItem {
            address,
            looper_task,
        })
    }
}

#[derive(Debug)]
pub enum ActorCreationError {
    IdGenerationError,
    MessageSenderCreationError,
}
