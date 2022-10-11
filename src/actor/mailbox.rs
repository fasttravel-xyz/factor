use flume;
use log::trace;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use crate::actor::core::Processor;
use crate::actor::receiver::{
    ActorReceiver, ActorReceiverContext, ActorReceiverContextPrivate, SystemHandlerContext,
};
use crate::message::envelope::{Payload, SystemPayload};
use crate::system::{SystemCommand, SystemMessage, SystemNotification};

type MsgReceiver<R> = flume::Receiver<Box<dyn Payload<R> + Send>>;
type SysMsgReceiver<R> = flume::Receiver<Box<dyn SystemPayload<R> + Send>>;
type SysMsgList<R> = Vec<Box<dyn SystemPayload<R> + Send>>;

/// The Actor Mailbox
pub(crate) struct Mailbox<R: ActorReceiver> {
    data: Arc<Inner<R>>,
}

struct Inner<R: ActorReceiver> {
    msgs: MsgReceiver<R>,
    msgs_sys: SysMsgReceiver<R>,
    paused: Arc<AtomicBool>,
    limit_break: Arc<AtomicBool>,
}

impl<R: ActorReceiver> Mailbox<R> {
    pub(crate) fn new(msgs: MsgReceiver<R>, msgs_sys: SysMsgReceiver<R>) -> Self {
        let inner = Inner {
            msgs,
            msgs_sys,
            paused: Arc::new(AtomicBool::new(true)),
            limit_break: Arc::new(AtomicBool::new(false)),
        };
        Mailbox {
            data: Arc::new(inner),
        }
    }

    pub(crate) fn set_paused(&self, b: bool) {
        self.data.paused.store(b, Ordering::Relaxed);
    }

    fn is_paused(&self) -> bool {
        self.data.paused.load(Ordering::Relaxed)
    }

    pub(crate) fn set_limit_break(&self, b: bool) {
        self.data.limit_break.store(b, Ordering::Relaxed);
    }

    fn is_limit_break(&self) -> bool {
        self.data.limit_break.load(Ordering::Relaxed)
    }

    fn process_msgs(&self, receiver: &mut R, ctx: &mut R::Context) -> Result<(), ()> {
        let msgs = self.data.msgs.clone();

        if !msgs.is_disconnected() && !msgs.is_empty() {
            let mut count = 0;
            let now = Instant::now();
            loop {
                let elapsed_time = now.elapsed();

                trace!(
                    "== {} == {} NANOSECS == ",
                    " mailbox_process_msgs elapsed: ",
                    elapsed_time.as_nanos()
                );

                let per_actor_run_msg_limit = ctx.system().get_per_actor_run_msg_limit();
                let per_actor_time_slice = ctx.system().get_per_actor_time_slice();

                if (count < per_actor_run_msg_limit) && (elapsed_time < per_actor_time_slice) {
                    if let Ok(mut envelope) = msgs.try_recv() {
                        envelope.handle(receiver, ctx);
                        count = count + 1;
                    } else {
                        break;
                    }
                } else {
                    self.set_limit_break(true);
                    break;
                }
            }
        }

        Ok(())
    }

    fn process_sys_msgs(&self, receiver: &mut R, ctx: &mut R::Context) -> Result<(), ()>
    where
        R::Context: SystemHandlerContext<R>,
    {
        let msgs_sys = self.data.msgs_sys.clone();

        if !msgs_sys.is_disconnected() && !msgs_sys.is_empty() {
            let msg_list: SysMsgList<R> = msgs_sys.drain().collect();
            for mut envelope in msg_list {
                envelope.handle(receiver, ctx);
            }
        }

        Ok(())
    }

    pub(in crate::actor) fn run(&self, processor: &Processor<R>) -> Result<(), ()>
    where
        R: ActorReceiver,
        R::Context: ActorReceiverContextPrivate<R> + SystemHandlerContext<R>,
    {
        // [todo] create RAII-MailboxPanicGuard to handle thread::panicking() in drop().

        let mut guard = processor.receiver.lock().unwrap();
        let receiver = guard.as_mut().unwrap();
        let weak_core_ref = processor.core.clone();
        if let Some(core) = weak_core_ref.upgrade() {
            if let Some(weak_ref) = core.address() {
                let mut ctx = R::Context::new(
                    processor.system.clone(),
                    weak_ref,
                    weak_core_ref.clone(),
                    core.executor(),
                );

                // process system messages on priority before user messages
                let _ = self.process_sys_msgs(receiver, &mut ctx);

                // process user messages
                // our msgs_queue is unbounded, this could lead to high number of
                // pending messages. TODO pause receiving messages after a limit is reached.
                if !self.is_paused() {
                    let _ = self.process_msgs(receiver, &mut ctx);
                }

                // reschedule mailbox if processing is stopped because per.run.limit reached
                if self.is_limit_break() {
                    self.set_limit_break(false);
                    core.schedule_mailbox();
                }
                // process system messages received during processing user messages
                let _ = self.process_sys_msgs(receiver, &mut ctx);
            }
        }

        Ok(())
    }
}

pub(crate) fn handle_system_msg<R: ActorReceiver>(
    msg: &SystemMessage,
    actor: &mut R,
    ctx: &mut R::Context,
) where
    R::Context: SystemHandlerContext<R>,
{
    match msg {
        SystemMessage::Event(event) => {
            actor.recv_sys(event, ctx);
        }
        SystemMessage::Notification(notif) => {
            handle_system_notification(&notif, actor, ctx);
        }
        SystemMessage::Command(cmd) => {
            handle_system_cmd::<R>(&cmd, actor, ctx);
        }
    }
}

pub(crate) fn handle_system_notification<R: ActorReceiver>(
    notif: &SystemNotification,
    actor: &mut R,
    ctx: &mut R::Context,
) where
    R::Context: SystemHandlerContext<R>,
{
    if let Some(core) = ctx.core() {
        match notif {
            SystemNotification::ActorInit => {
                actor.finalize_init(ctx);
                // sys.publish();
                core.set_pause(false);
                core.schedule_mailbox();
            }
            SystemNotification::ActorFailed => {} // todo
        }
    }
}

pub(crate) fn handle_system_cmd<R: ActorReceiver>(
    cmd: &SystemCommand,
    actor: &mut R,
    ctx: &mut R::Context,
) where
    R::Context: SystemHandlerContext<R>,
{
    if let Some(core) = ctx.core() {
        match cmd {
            SystemCommand::ActorPause(b) => core.set_pause(*b),
            SystemCommand::ActorRestart => core.restart(),
            SystemCommand::ActorStop => {
                // do actor.finalize_stop before the core.stop, in case
                // user wants to cancel [todo] it
                actor.finalize_stop(ctx);
                core.stop();
            }
        }
    }
}

#[cfg(feature = "debug-log")]
impl<R: ActorReceiver> Drop for Mailbox<R> {
    fn drop(&mut self) {
        println!(
            "==== DROPPING ================================== {}",
            "Mailbox"
        );
    }
}
