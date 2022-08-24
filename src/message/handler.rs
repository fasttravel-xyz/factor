use core::pin::Pin;
use futures::{channel::oneshot, future::Future};

use crate::actor::receiver::{ActorReceiver, ActorReceiverContext};
use crate::message::Message;

/// Trait to be implemented to handle specific messages.
pub trait MessageHandler<M>
where
    M: Message,
    Self: ActorReceiver,
{
    /// Message Response Type conforms to MessageResponse trait
    type Result: MessageResponse<M, Self>;

    /// Called for every message received by the actor
    fn handle(&mut self, msg: M, ctx: &mut Self::Context) -> Self::Result;
}

/// Trait representing a message response
pub trait MessageResponse<M: Message, R: ActorReceiver> {
    fn handle(self, ctx: &mut R::Context, tx: ReplyToRef<M>);
}

pub(crate) trait ReplyTo<M: Message> {
    fn reply(self, response: M::Result);
}

/// The reply to address to send the response for a request.
/// Used for the ask-pattern.
pub struct ReplyToRef<M: Message> {
    pub tx: Option<oneshot::Sender<M::Result>>,
}

impl<M: Message> ReplyTo<M> for ReplyToRef<M> {
    fn reply(self, response: M::Result) {
        if let Some(tx) = self.tx {
            if !tx.is_canceled() {
                let _ = tx.send(response);
            }
        }
    }
}

/// Response type for immediate and async responses.
pub enum MessageResponseType<I> {
    Result(ResponseResult<I>),
    Future(ResponseFuture<I>),
}
/// Immediate response.
pub struct ResponseResult<I>(pub I);
/// Async future response.
pub type ResponseFuture<I> = Pin<Box<dyn Future<Output = I> + Send>>;

impl<M: Message + 'static, R: ActorReceiver> MessageResponse<M, R>
    for MessageResponseType<M::Result>
{
    fn handle(self, ctx: &mut R::Context, reply_to: ReplyToRef<M>) {
        match self {
            MessageResponseType::Result(response) => {
                reply_to.reply(response.0);
            }
            MessageResponseType::Future(response) => {
                let task = async { reply_to.reply(response.await) };
                ctx.spawn_ok(task);
            }
        }
    }
}

impl<T> From<T> for ResponseResult<T> {
    fn from(t: T) -> Self {
        ResponseResult(t)
    }
}
