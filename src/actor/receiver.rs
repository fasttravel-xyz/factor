use futures::future::Future;
use std::{
    marker::PhantomData,
    sync::{Arc, Weak},
};

use crate::actor::{core::ActorCore, ActorAddr, ActorWeakAddr, Addr};
use crate::message::{
    handler::{MessageHandler, MessageResponseType, ResponseResult},
    Message, MessageSendError,
};
use crate::system::{SystemEvent, SystemMessage, SystemRef};

/// All actor types must implement this trait.
pub trait ActorReceiver
where
    Self: Send + 'static + Sized,
{
    type Context: ActorReceiverContext<Self>;

    /// Receive and handle system events.
    fn recv_sys(&mut self, _msg: &SystemEvent, _ctx: &mut Self::Context) {}

    /// Handle and finalize actor creation.
    fn finalize_init(&mut self, _ctx: &mut Self::Context) {}

    /// Handle and finalize actor termination.
    fn finalize_stop(&mut self, _ctx: &mut Self::Context) {}
}

/// Provides services to the message handlers.
pub trait ActorReceiverContext<R: ActorReceiver> {
    /// Spawn a future in the system executor.
    fn spawn_ok<Fut: 'static + Future<Output = ()> + Send>(&self, task: Fut);

    /// Get the reference to the system.
    fn system(&self) -> SystemRef;

    /// Get the address of the actor.
    fn address(&self) -> Option<ActorAddr<R>>;

    // pending tasks:
    // fn schedule();
    // fn schedule_once();
    // fn schedule_interval();
    // fn cancel_schedule();
    // fn spawn_wait();
    // fn notify(); // bypass mailbox. for in-process actors.
    // fn state() -> ActorState;
}

// mod-private trait for context private functions
pub(in crate::actor) trait ActorReceiverContextPrivate<R: ActorReceiver> {
    fn new(system: SystemRef, address: ActorWeakAddr<R>, weak_core_ref: Weak<ActorCore<R>>)
        -> Self;
}

pub(crate) trait SystemHandlerContext<R: ActorReceiver> {
    fn core(&self) -> Option<Arc<ActorCore<R>>>;
}

/// Basic Context Object for the Message Handlers
pub struct BasicContext<R: ActorReceiver> {
    system: SystemRef,
    address: ActorWeakAddr<R>,
    core: Weak<ActorCore<R>>,
    // [todo]: store the core-executor
}

impl<R: ActorReceiver> ActorReceiverContextPrivate<R> for BasicContext<R> {
    fn new(system: SystemRef, address: ActorWeakAddr<R>, core: Weak<ActorCore<R>>) -> Self {
        BasicContext {
            system,
            address,
            core,
        }
    }
}

impl<R: ActorReceiver> ActorReceiverContext<R> for BasicContext<R> {
    fn spawn_ok<Fut>(&self, task: Fut)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        // [todo] spawn on the correct executor for actor-pool
        self.system.spawn_ok(task);
    }

    fn system(&self) -> SystemRef {
        self.system.clone()
    }

    fn address(&self) -> Option<ActorAddr<R>> {
        self.address.upgrade()
    }
}

impl<R: ActorReceiver> SystemHandlerContext<R> for BasicContext<R> {
    fn core(&self) -> Option<Arc<ActorCore<R>>> {
        self.core.upgrade()
    }
}

/// ActorReceiver for a functional message handler
pub struct FunctionHandler<F, M>
where
    M: Message,
    F: (Fn(M, &mut FnHandlerContext) -> M::Result) + Send + Sync + 'static,
{
    func: Box<F>,
    phantom: PhantomData<M>,
}

impl<F, M> FunctionHandler<F, M>
where
    M: Message,
    F: (Fn(M, &mut FnHandlerContext) -> M::Result) + Send + Sync + 'static,
{
    pub fn new(f: F) -> FunctionHandler<F, M> {
        FunctionHandler {
            func: Box::new(f),
            phantom: PhantomData,
        }
    }
}

impl<F, M> ActorReceiver for FunctionHandler<F, M>
where
    M: Message + Send + 'static,
    F: (Fn(M, &mut FnHandlerContext) -> M::Result) + Send + Sync + 'static,
{
    type Context = BasicContext<Self>;
}

impl<M, F> MessageHandler<M> for FunctionHandler<F, M>
where
    M: Message + Send + 'static,
    F: (Fn(M, &mut FnHandlerContext) -> M::Result) + Send + Sync + 'static,
{
    type Result = MessageResponseType<M::Result>;

    fn handle(&mut self, msg: M, ctx: &mut Self::Context) -> Self::Result {
        let mut address = None;
        if let Some(a) = ctx.address() {
            address = Some(a.get_address());
        }
        let mut fn_ctx = FnHandlerContext {
            system: ctx.system(),
            address,
        };
        let rslt = (self.func)(msg, &mut fn_ctx);
        MessageResponseType::Result(ResponseResult(rslt))
    }
}

/// Context for the functional handlers
#[allow(dead_code)]
pub struct FnHandlerContext {
    system: SystemRef,
    address: Option<Addr>,
}

#[allow(dead_code)]
impl FnHandlerContext {
    /// Spawn a future in the system executor.
    fn spawn_ok<Fut>(&self, task: Fut)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        self.system.spawn_ok(task);
    }

    /// Get reference to the system
    fn system(&self) -> SystemRef {
        self.system.clone()
    }

    /// Send System Message
    fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        if let Some(addr) = &self.address {
            return addr.0.tell_sys_msg(msg);
        }

        Err(MessageSendError)
    }
}
