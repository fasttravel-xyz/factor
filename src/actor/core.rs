use flume;
use log::trace;
use std::sync::{Arc, Mutex, Weak};

use crate::actor::{
    mailbox::Mailbox,
    receiver::{ActorReceiver, BasicContext},
    ActorWeakAddr, Addr,
};
use crate::message::{
    envelope::{Envelope, Payload, SystemEnvelope, SystemPayload},
    MessageSendError,
};
use crate::system::{
    guardian::{ActorGuardianType, GuardianWeakRef},
    SystemEvent, SystemMessage, SystemRef,
};

/// Messages related to actor looper and mailbox scheduling.
#[allow(dead_code)]
pub(in crate::actor) enum CoreCommand {
    /// schedules the actors mailbox to run
    Run,
    /// pause message processing
    Pause(bool),
    /// restart the actor
    Restart,
    /// terminate the actor
    Terminate,
}

type MsgSender<R> = flume::Sender<Box<dyn Payload<R> + Send>>;
type SysMsgSender<R> = flume::Sender<Box<dyn SystemPayload<R> + Send>>;

#[allow(dead_code)]
pub(crate) struct ActorCore<R: ActorReceiver> {
    processor: Arc<Processor<R>>, // Option<> for RESTART
    looper: Looper<R>,            // Option<> for RESTART
    guardian: Option<GuardianWeakRef>,
    address: Option<ActorWeakAddr<R>>,
}

pub(in crate::actor) struct Processor<R: ActorReceiver> {
    pub(in crate::actor) system: SystemRef,
    pub(in crate::actor) receiver: Arc<Mutex<Option<R>>>,
    pub(in crate::actor) core: Weak<ActorCore<R>>,
}

#[allow(dead_code)]
struct Looper<R: ActorReceiver> {
    tx_cmd: flume::Sender<CoreCommand>,
    tx_msg: MsgSender<R>,
    tx_msg_sys: SysMsgSender<R>,
    mb: Arc<Mailbox<R>>,
}

// In future maybe we can use type_alias_impl_trait and won't need this struct
pub(crate) struct LopperTask<R: ActorReceiver> {
    rx_cmd: Option<flume::Receiver<CoreCommand>>,
    mb_ref: Option<Arc<Mailbox<R>>>,
    processor: Option<Arc<Processor<R>>>,
}

impl<R: ActorReceiver> Default for LopperTask<R> {
    fn default() -> Self {
        LopperTask {
            rx_cmd: None,
            mb_ref: None,
            processor: None,
        }
    }
}

impl<R: ActorReceiver> LopperTask<R> {
    pub(crate) fn get_future(self) -> impl futures::Future<Output = ()>
    where
        R: ActorReceiver<Context = BasicContext<R>>,
    {
        let rx_cmd = self.rx_cmd.unwrap();
        let mb_ref = self.mb_ref.unwrap();
        let processor = self.processor.unwrap();

        let fut = async move {
            while let Ok(msg) = rx_cmd.recv_async().await {
                match msg {
                    CoreCommand::Run => {
                        let _ = mb_ref.run(&processor);
                    }
                    CoreCommand::Pause(b) => {
                        mb_ref.set_paused(b);
                    }
                    CoreCommand::Restart => {
                        // [todo]: actor restart
                        // 1. To restart we have to recreate Looper and the Processor.
                        // 2. We have to make sure whether the objects are unwinding
                        //    safe and if not how to handle them. Some cases like
                        //    process_abort will not be recoverable.
                        // 3. Multiple actors might need restart if sharing panicking thread.
                        // 4. MailboxPanicGuard => thread::panicking() needs to be implemented.
                        // 5. We have to make sure that user provided ActorReceiver
                        //    could be recovered or might need recreation. For this
                        //    a factory/config interface for creation of receiver is required.
                        break;
                    }
                    CoreCommand::Terminate => {
                        break;
                    }
                }
            }
        };

        return fut;
    }
}

impl<R: ActorReceiver> ActorCore<R> {
    fn create_looper() -> (Looper<R>, LopperTask<R>) {
        let (tx_cmd, rx_cmd) = flume::unbounded();
        let (tx_msg, rx_msg) = flume::unbounded();
        let (tx_msg_sys, rx_msg_sys) = flume::unbounded();
        let mb = Arc::new(Mailbox::new(rx_msg, rx_msg_sys));
        let mb_ref = mb.clone();

        (
            Looper {
                tx_cmd,
                tx_msg,
                tx_msg_sys,
                mb,
            },
            LopperTask {
                rx_cmd: Some(rx_cmd),
                mb_ref: Some(mb_ref),
                processor: None,
            },
        )
    }

    pub(crate) fn create_core(
        receiver: R,
        system: &SystemRef,
        g_type: &ActorGuardianType,
        address: ActorWeakAddr<R>,
    ) -> Result<(Arc<Self>, LopperTask<R>), ActorCoreCreationError> {
        let (looper, mut looper_task) = ActorCore::create_looper();

        // "guardian" will be None for other guardians as for a guardian the g_type is Root(dummy)
        let mut guardian = None;
        if let Some(g) = system.get_guardian_ref(g_type) {
            guardian = Some(g.downgrade());
        }

        let core = Arc::new_cyclic(|weak| {
            let processor = Arc::new(Processor {
                receiver: Arc::new(Mutex::new(Some(receiver))),
                system: system.clone(),
                core: weak.clone(),
            });

            looper_task.processor = Some(processor.clone());

            ActorCore {
                processor: processor.clone(),
                looper,
                guardian,
                address: Some(address),
            }
        });

        Ok((core, looper_task))
    }

    pub(crate) fn send(&self, env: Envelope<R>) -> Result<(), MessageSendError> {
        let mut result = Ok(());

        match self.looper.tx_msg.send(Box::new(env)) {
            Ok(()) => {}
            Err(e) => {
                result = Err(MessageSendError);
                trace!("ActorCore::send::error {}", e);
            }
        };

        // schedule the mailbox to run. ??Handle multiple runs queued??
        if let Err(e) = self.looper.tx_cmd.send(CoreCommand::Run) {
            result = Err(MessageSendError);
            trace!("ActorCore::send::error {}", e);
        }

        result
    }

    pub(crate) fn send_sys(&self, env: SystemEnvelope<R>) -> Result<(), MessageSendError> {
        let mut result = Ok(());

        match self.looper.tx_msg_sys.send(Box::new(env)) {
            Ok(()) => {}
            Err(e) => {
                result = Err(MessageSendError);
                trace!("ActorCore::send_sys::error {}", e);
            }
        };

        if let Err(e) = self.looper.tx_cmd.send(CoreCommand::Run) {
            result = Err(MessageSendError);
            trace!("ActorCore::send_sys::error {}", e);
        }

        result
    }

    pub(crate) fn schedule_mailbox(&self) {
        let _ = self.looper.tx_cmd.send(CoreCommand::Run);
    }

    pub(crate) fn set_pause(&self, b: bool) {
        let _ = self.looper.tx_cmd.send(CoreCommand::Pause(b));
    }

    pub(crate) fn address(&self) -> Option<ActorWeakAddr<R>> {
        self.address.clone()
    }

    pub(crate) fn stop(&self) {
        // sys.publish(Topic::DeadLetter)
        // sys.publish(SystemEvent::ActorTerminated)

        if let Some(g_ref) = &self.guardian {
            if let Some(g) = g_ref.upgrade() {
                if let Some(weak_ref) = &self.address {
                    if let Some(addr) = weak_ref.upgrade() {
                        if let Err(e) = self.looper.tx_cmd.send(CoreCommand::Terminate) {
                            trace!("ActorCore::stop::error {}", e);
                        }

                        let msg = SystemMessage::Event(SystemEvent::ActorTerminated(Addr(
                            Box::new(addr),
                        )));
                        if let Err(e) = g.tell_sys(msg) {
                            trace!("ActorCore::stop::error {:?}", e);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn restart(&self) {
        // tentative restart_protocol:
        // 1. terminate current mailbox
        // 2. recreate processor.receiver
        // 3. recreate looper
        // 4. respawn looper
        // 5. receiver.restarted()
        // 6. sys.publish(Topic::ActorRestarted)
    }
}

#[derive(Debug)]
pub(crate) struct ActorCoreCreationError;
