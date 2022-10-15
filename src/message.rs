pub(crate) mod envelope;
pub(crate) mod handler;

use async_trait::async_trait;
use futures::channel::oneshot;
use log::error;
use std::fmt;
use std::sync::Arc;

#[cfg(all(unix, feature = "ipc-cluster"))]
use serde::{Deserialize, Serialize};

use crate::actor::{core::ActorCore, core::RemoteActorCore, receiver::ActorReceiver};
use crate::message::{
    envelope::{Envelope, SystemEnvelope},
    handler::MessageHandler,
};
use crate::system::SystemMessage;

#[cfg(all(unix, feature = "ipc-cluster"))]
use crate::message::handler::MessageClusterHandler;

/// All message types must implement this trait.
pub trait Message {
    type Result: 'static + Send;
}

/// All cluster-message(messages to be passed between ipc-nodes) types must implement this trait.
#[cfg(all(unix, feature = "ipc-cluster"))]
pub trait MessageCluster: for<'a> Deserialize<'a> + Serialize {
    type Result: 'static + Send + for<'a> Deserialize<'a> + Serialize;
}

/// Trait for the actor Typed Message Sender.
#[cfg(all(unix, feature = "ipc-cluster"))]
#[async_trait]
pub(crate) trait MessageClusterSend<M: MessageCluster + Send + 'static>: Send {
    // CHANGELOG [15/AUG/2022]: The trait has the generic `M: Message` and the
    // implementing struct has the generic `R: ActorReceiver`. This way trait-objects
    // of MessageSend could act as handlers/recipients of a single message type M.
    // Keeping the generic M on the methods prevents creation of trait-object.

    /// Send message to the actor mailbox.
    fn tell_addr(&self, msg: M) -> Result<(), MessageSendError>;

    /// Send message to the actor mailbox and receive a response back.
    async fn ask_addr(&self, msg: M) -> Result<M::Result, MessageSendError>;
}

/// Trait for the actor Typed Message Sender.
#[async_trait]
pub(crate) trait MessageSend<M: Message + Send + 'static>: Send {
    // CHANGELOG [15/AUG/2022]: The trait has the generic `M: Message` and the
    // implementing struct has the generic `R: ActorReceiver`. This way trait-objects
    // of MessageSend could act as handlers/recipients of a single message type M.
    // Keeping the generic M on the methods prevents creation of trait-object.

    /// Send message to the actor mailbox.
    fn tell(&self, msg: M) -> Result<(), MessageSendError>;

    /// Send message to the actor mailbox and receive a response back.
    async fn ask(&self, msg: M) -> Result<M::Result, MessageSendError>;
}

pub(crate) trait SystemMessageSend: Send {
    /// Send a system message to the actor mailbox.
    fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError>;
}

/// Message Sender holding the Local Sender or Remote Sender.
#[allow(dead_code)]
pub(crate) enum MessageSender<R: ActorReceiver> {
    LocalSender { core: Arc<ActorCore<R>> },
    RemoteSender { core: Arc<RemoteActorCore<R>> },
}

#[async_trait]
impl<R, M> MessageSend<M> for MessageSender<R>
where
    R: ActorReceiver + MessageHandler<M> + 'static,
    M: Message + Send + 'static,
    M::Result: 'static,
{
    fn tell(&self, msg: M) -> Result<(), MessageSendError> {
        match self {
            MessageSender::LocalSender { core } => {
                let envelope = Envelope::new(msg, None);
                return core.send(envelope).map_err(|e| {
                    error!("MessageSender::tell::error {:?}", e);
                    e
                });
            }
            MessageSender::RemoteSender { core } => return self.remote_tell(msg, core),
        }
    }

    async fn ask(&self, msg: M) -> Result<M::Result, MessageSendError> {
        let (tx, rx) = oneshot::channel::<M::Result>();

        match self {
            MessageSender::LocalSender { core } => {
                let envelope = Envelope::new(msg, Some(tx));
                if let Err(e) = core.send(envelope) {
                    error!("MessageSender::ask::error {:?}", e);
                    return Err(e);
                } else {
                    return rx.await.map_err(|e| e.into());
                }
            }
            MessageSender::RemoteSender { core } => return self.remote_ask(msg, core).await,
        }
    }
}

#[cfg(all(unix, feature = "ipc-cluster"))]
#[async_trait]
impl<R, M> MessageClusterSend<M> for MessageSender<R>
where
    R: ActorReceiver + MessageClusterHandler<M> + 'static,
    M: MessageCluster + Send + 'static,
    M::Result: 'static,
{
    fn tell_addr(&self, msg: M) -> Result<(), MessageSendError> {
        match self {
            MessageSender::LocalSender { core } => {
                let envelope = Envelope::new_cluster(msg, None);
                return core.send(envelope).map_err(|e| {
                    error!("MessageSender::tell::error {:?}", e);
                    e
                });
            }
            MessageSender::RemoteSender { core } => return self.remote_tell_addr(msg, core),
        }
    }

    async fn ask_addr(&self, msg: M) -> Result<M::Result, MessageSendError> {
        let (tx, rx) = oneshot::channel::<M::Result>();

        match self {
            MessageSender::LocalSender { core } => {
                let envelope = Envelope::new_cluster(msg, Some(tx));
                if let Err(e) = core.send(envelope) {
                    error!("MessageSender::ask::error {:?}", e);
                    return Err(e);
                } else {
                    return rx.await.map_err(|e| e.into());
                }
            }
            MessageSender::RemoteSender { core } => return self.remote_ask_addr(msg, core).await,
        }
    }
}

impl<R> MessageSender<R>
where
    R: ActorReceiver,
{
    #[cfg(all(unix, feature = "ipc-cluster"))]
    fn remote_tell_addr<M>(
        &self,
        msg: M,
        core: &Arc<RemoteActorCore<R>>,
    ) -> Result<(), MessageSendError>
    where
        R: MessageClusterHandler<M> + 'static,
        M: MessageCluster + Send + 'static,
        M::Result: 'static,
    {
        let addr = core
            .address
            .upgrade()
            .ok_or(MessageSendError::ErrAddrUpgradeFailed)?;
        let system = core
            .system
            .upgrade()
            .ok_or(MessageSendError::ErrSysUpgradeFailed)?;
        let system_moved = system.clone();
        let node_id = addr.get_id().node_id();
        let task = async move {
            if let Some(remote_client) = system_moved.get_node_client(node_id).await {
                remote_client.tell(addr, msg).await;
            }
        };
        system.spawn_ok(task);

        Ok(())
    }

    fn remote_tell<M>(
        &self,
        _msg: M,
        _core: &Arc<RemoteActorCore<R>>,
    ) -> Result<(), MessageSendError>
    where
        R: MessageHandler<M> + 'static,
        M: Message + Send + 'static,
        M::Result: 'static,
    {
        error!("this_is_a_remote_addr_use_tell_addr_instead_of_tell");
        Err(MessageSendError::ErrRemoteTellFailed)
    }

    #[cfg(all(unix, feature = "ipc-cluster"))]
    async fn remote_ask_addr<M>(
        &self,
        msg: M,
        core: &Arc<RemoteActorCore<R>>,
    ) -> Result<M::Result, MessageSendError>
    where
        R: MessageClusterHandler<M> + 'static,
        M: MessageCluster + Send + 'static,
        M::Result: 'static,
    {
        let addr = core
            .address
            .upgrade()
            .ok_or(MessageSendError::ErrAddrUpgradeFailed)?;
        let system = core
            .system
            .upgrade()
            .ok_or(MessageSendError::ErrSysUpgradeFailed)?;
        let node_id = addr.get_id().node_id();
        if let Some(remote_client) = system.get_node_client(node_id).await {
            return remote_client
                .ask(addr, msg)
                .await
                .map_err(|_| MessageSendError::ErrRemoteAskFailed);
        }

        Err(MessageSendError::ErrRemoteAskFailed)
    }

    async fn remote_ask<M>(
        &self,
        _msg: M,
        _core: &Arc<RemoteActorCore<R>>,
    ) -> Result<M::Result, MessageSendError>
    where
        R: MessageHandler<M> + 'static,
        M: Message + Send + 'static,
        M::Result: 'static,
    {
        error!("this_is_a_remote_addr_use_ask_addr_instead_of_ask");
        Err(MessageSendError::ErrRemoteAskFailed)
    }
}

impl<R> SystemMessageSend for MessageSender<R>
where
    R: ActorReceiver,
{
    fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        let mut result = Ok(());

        match self {
            MessageSender::LocalSender { core } => {
                let envelope = SystemEnvelope::new(msg);
                if let Err(e) = core.send_sys(envelope) {
                    error!("MessageSender::tell_sys::error {:?}", e);
                    result = Err(e);
                }
            }
            MessageSender::RemoteSender { core: _ } => {
                // Currently, system_messages not available to remote-addresses.
                // Adding support is trivial as we have already implemented tell/ask
                // for ipc-cluster, will do with pubsub implementation (low-priority).
            }
        }

        result
    }
}

/// Message Sender Error.
#[derive(Debug)]
pub enum MessageSendError {
    ErrDefault,
    ErrAddrUpgradeFailed,
    ErrSysUpgradeFailed,
    ErrSenderNotResolved,
    ErrAskFutureCanceled,
    ErrRemoteTellFailed,
    ErrRemoteAskFailed,
    ErrActorCoreCorrupted,
}

impl fmt::Display for MessageSendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg: &str;
        match self {
            MessageSendError::ErrAddrUpgradeFailed => msg = "weak_self_addr_upgrade_failed",
            MessageSendError::ErrSysUpgradeFailed => msg = "weak_system_upgrade_failed",
            MessageSendError::ErrSenderNotResolved => msg = "sender_not_resolved_for_addr",
            MessageSendError::ErrAskFutureCanceled => msg = "ask_future_canceled_by_mailbox",
            MessageSendError::ErrRemoteTellFailed => msg = "remote_tell_failed",
            MessageSendError::ErrRemoteAskFailed => msg = "remote_ask_failed",
            MessageSendError::ErrActorCoreCorrupted => msg = "actor_core_corrupted",
            _ => msg = "",
        }

        write!(f, "({})", msg)
    }
}

impl From<oneshot::Canceled> for MessageSendError {
    fn from(_error: oneshot::Canceled) -> Self {
        MessageSendError::ErrAskFutureCanceled
    }
}
