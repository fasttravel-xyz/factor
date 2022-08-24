pub(crate) mod envelope;
pub mod handler;

use futures::channel::oneshot;
use log::trace;
use std::sync::Arc;

use crate::actor::{core::ActorCore, receiver::ActorReceiver, ActorWeakRef};
use crate::message::{
    envelope::{Envelope, SystemEnvelope},
    handler::MessageHandler,
};
use crate::system::SystemMessage;

/// All message types must implement this trait.
pub trait Message {
    type Result: 'static + Send;
}

/// Trait for the actor Message Sender.
pub(crate) trait MessageSend<R: ActorReceiver>: Send {
    /// Send message to the actor mailbox.
    fn tell<M>(&self, msg: M) -> Result<(), MessageSendError>
    where
        M: Message + Send + 'static,
        R: MessageHandler<M>;

    /// Send message to the actor mailbox and receive a response back.
    fn ask<M>(&self, msg: M) -> Result<oneshot::Receiver<M::Result>, MessageSendError>
    where
        M: Message + Send + 'static,
        R: MessageHandler<M>;

    /// Send a system message to the actor mailbox.
    fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError>;
}

/// Message Sender holding the Local Sender or Remote Sender.
#[allow(dead_code)]
pub(crate) enum MessageSender<R: ActorReceiver> {
    LocalSender { core: Arc<ActorCore<R>> },
    RemoteSender { _remote_guardian: ActorWeakRef<R> },
}

impl<R: ActorReceiver> MessageSend<R> for MessageSender<R> {
    fn tell<M>(&self, msg: M) -> Result<(), MessageSendError>
    where
        M: Message + Send + 'static,
        R: MessageHandler<M>,
    {
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
                // [todo] cluster implementation is low priority.
                // create the guardian::RemoteMessage and send to _remote_guardian for dispatch.
                // let remote_ref = core.address().unwrap().upgrade().unwrap();
                // let r_msg = RemoteMessage{msg, remote_ref};
                // let r_dispatcher = _remote_guardian.upgrade().unwrap();
                // r_dispatcher->tell(r_msg);
            }
        }

        result
    }

    fn ask<M>(&self, msg: M) -> Result<oneshot::Receiver<M::Result>, MessageSendError>
    where
        M: Message + Send + 'static,
        R: MessageHandler<M>,
    {
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
