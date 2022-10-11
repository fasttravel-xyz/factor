use futures::channel::oneshot;
use log::trace;

use crate::actor::{
    mailbox::handle_system_msg,
    receiver::{ActorReceiver, SystemHandlerContext},
};
use crate::message::{
    handler::{MessageHandler, MessageResponse, ReplyToRef},
    Message,
};
use crate::system::SystemMessage;

/// Trait representing a payload inside an Envelope.
pub(crate) trait Payload<R: ActorReceiver> {
    fn handle(&mut self, actor: &mut R, ctx: &mut <R as ActorReceiver>::Context);
}

/// The envelope for Message payload.
pub(crate) struct Envelope<R: ActorReceiver> {
    data: Box<dyn Payload<R> + Send>,
}

impl<R: ActorReceiver> Envelope<R> {
    pub(crate) fn new<M>(msg: M, tx: Option<oneshot::Sender<M::Result>>) -> Self
    where
        R: MessageHandler<M>,
        M: Message + Send + 'static,
        M::Result: Send,
    {
        Envelope {
            data: Box::new(MessageEnvelope {
                reply_to: Some(ReplyToRef { tx }),
                msg: Some(msg),
            }),
        }
    }
}

impl<R: ActorReceiver> Payload<R> for Envelope<R> {
    fn handle(&mut self, actor: &mut R, ctx: &mut <R as ActorReceiver>::Context) {
        self.data.handle(actor, ctx);
    }
}

/// Trait representing a payload inside an SystemEnvelope.
pub(crate) trait SystemPayload<R: ActorReceiver> {
    fn handle(&mut self, actor: &mut R, ctx: &mut <R as ActorReceiver>::Context)
    where
        R::Context: SystemHandlerContext<R>;
}

/// The Envelope for SystemMessage payload.
pub(crate) struct SystemEnvelope<R: ActorReceiver> {
    data: Box<dyn SystemPayload<R> + Send>,
}

impl<R: ActorReceiver> SystemEnvelope<R> {
    pub(crate) fn new(msg: SystemMessage) -> Self
    where
        R: ActorReceiver,
    {
        SystemEnvelope {
            data: Box::new(SystemMessageEnvelope(Some(msg))),
        }
    }
}

impl<R: ActorReceiver> SystemPayload<R> for SystemEnvelope<R> {
    fn handle(&mut self, actor: &mut R, ctx: &mut <R as ActorReceiver>::Context)
    where
        R::Context: SystemHandlerContext<R>,
    {
        self.data.handle(actor, ctx);
    }
}

struct MessageEnvelope<M>
where
    M: Message + Send,
    M::Result: Send,
{
    msg: Option<M>,
    reply_to: Option<ReplyToRef<M>>,
}

impl<R, M> Payload<R> for MessageEnvelope<M>
where
    R: ActorReceiver + MessageHandler<M>,
    M: Message + Send + 'static,
    M::Result: Send,
{
    fn handle(&mut self, actor: &mut R, ctx: &mut R::Context) {
        if let Some(msg) = self.msg.take() {
            // [todo]: if reply_to is none (i.e. tell and not ask), and response is Future not Result,
            // we have to poll the future, or the future will never get polled.
            let response = <R as MessageHandler<M>>::handle(actor, msg, ctx);
            if let Some(reply_to) = self.reply_to.take() {
                if reply_to.tx.is_some() {
                    response.handle(ctx, reply_to);
                }
            }
        } else {
            trace!("MessageEnvelope_handle_error msg is None")
        }
    }
}

struct SystemMessageEnvelope(Option<SystemMessage>);

impl<R: ActorReceiver> SystemPayload<R> for SystemMessageEnvelope {
    fn handle(&mut self, actor: &mut R, ctx: &mut R::Context)
    where
        R::Context: SystemHandlerContext<R>,
    {
        if let Some(msg) = self.0.take() {
            handle_system_msg(&msg, actor, ctx)
        } else {
            trace!("SystemMessageEnvelope_handle_error msg is None")
        }
    }
}
