pub(crate) mod envelope;
pub(crate) mod handler;

use futures::channel::oneshot;
use log::trace;
use std::sync::Arc;

use crate::actor::{core::ActorCore, receiver::ActorReceiver, ActorWeakAddr};
use crate::message::{
    envelope::{Envelope, SystemEnvelope},
    handler::MessageHandler,
};
use crate::system::SystemMessage;

/// All message types must implement this trait.
pub trait Message {
    type Result: 'static + Send;
}

/// Trait for the actor Typed Message Sender.
pub(crate) trait MessageSend<M: Message + Send + 'static>: Send {
    // CHANGELOG [15/AUG/2022]: The trait has the generic `M: Message` and the
    // implementing struct has the generic `R: ActorReceiver`. This way trait-objects
    // of MessageSend could act as handlers/recipients of a single message type M.
    // Keeping the generic M on the methods prevents creation of trait-object.

    /// Send message to the actor mailbox.
    fn tell(&self, msg: M) -> Result<(), MessageSendError>;

    /// Send message to the actor mailbox and receive a response back.
    fn ask(&self, msg: M) -> Result<oneshot::Receiver<M::Result>, MessageSendError>;
}

pub(crate) trait SystemMessageSend: Send {
    /// Send a system message to the actor mailbox.
    fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError>;
}

/// Message Sender holding the Local Sender or Remote Sender.
#[allow(dead_code)]
pub(crate) enum MessageSender<R: ActorReceiver> {
    LocalSender { core: Arc<ActorCore<R>> },
    RemoteSender { _remote_guardian: ActorWeakAddr<R> },
}

impl<R, M> MessageSend<M> for MessageSender<R>
where
    M: Message + Send + 'static,
    R: ActorReceiver + MessageHandler<M>,
{
    fn tell(&self, msg: M) -> Result<(), MessageSendError> {
        let mut result = Ok(());

        match self {
            MessageSender::LocalSender { core } => {
                let envelope = Envelope::new(msg, None);
                if let Err(e) = core.send(envelope) {
                    trace!("MessageSender::tell::error {:?}", e);
                    result = Err(e);
                }
            }
            MessageSender::RemoteSender { _remote_guardian } => {
                // =============================================================
                // [todo] cluster implementation is low priority.
                // [pseudocode] create the guardian::RemoteMessage and send to
                //              _remote_guardian for dispatch:
                // let remote_ref = core.address().unwrap().upgrade().unwrap();
                // let r_msg = RemoteMessage{msg, remote_ref};
                // let r_dispatcher = _remote_guardian.upgrade().unwrap();
                // r_dispatcher->tell(r_msg);
                // =============================================================
            }
        }

        result
    }

    fn ask(&self, msg: M) -> Result<oneshot::Receiver<M::Result>, MessageSendError> {
        let (tx, rx) = oneshot::channel::<M::Result>();
        let mut result = Ok(rx);

        match self {
            MessageSender::LocalSender { core } => {
                let envelope = Envelope::new(msg, Some(tx));
                if let Err(e) = core.send(envelope) {
                    trace!("MessageSender::ask::error {:?}", e);
                    result = Err(e);
                }
            }
            MessageSender::RemoteSender { _remote_guardian } => {} // todo
        }

        result
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
                    trace!("MessageSender::tell_sys::error {:?}", e);
                    result = Err(e);
                }
            }
            MessageSender::RemoteSender { _remote_guardian } => {} // todo
        }

        result
    }
}

/// Message Sender Error.
#[derive(Debug)]
pub struct MessageSendError;
