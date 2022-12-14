use flume;
use log::error;
use std::sync::{Arc, Mutex, Weak};

use crate::message::{
    envelope::{Envelope, Payload, SystemEnvelope, SystemPayload},
    MessageSendError,
};
use crate::system::{
    executor::{ActorExecutor, ThreadPoolExecutor},
    guardian::{ActorGuardianType, GuardianWeakAddr},
    SystemEvent, SystemMessage, SystemRef,
};
use crate::{
    actor::{
        mailbox::Mailbox,
        receiver::{ActorReceiver, BasicContext},
        ActorWeakAddr, Addr,
    },
    system::SystemWeakRef,
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

pub(crate) struct ActorCore<R>
where
    R: ActorReceiver,
{
    executor: ActorExecutor,
    guardian: Option<GuardianWeakAddr>,
    address: Option<ActorWeakAddr<R>>,
    // processors: Vec<Arc<Processor<R>>>, // index aligned with loopers
    loopers: Vec<Looper<R>>, // index aligned with processors
}

pub(in crate::actor) struct Processor<R: ActorReceiver> {
    pub(in crate::actor) system: SystemRef,
    pub(in crate::actor) receiver: Arc<Mutex<Option<R>>>,
    pub(in crate::actor) core: Weak<ActorCore<R>>,
}

struct Looper<R: ActorReceiver> {
    tx_cmd: flume::Sender<CoreCommand>,
    tx_msg: MsgSender<R>,
    tx_msg_sys: SysMsgSender<R>,
    // mb: Arc<Mailbox<R>>,
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
                        // 3. Multiple actors might need restart if sharing panicking thread or use catch_unwind().
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
    fn create_loopers(num_threads: usize) -> Vec<(Looper<R>, LopperTask<R>)> {
        // number of loopers is same as num_threads. we could expose this to config if necessary.
        let mut loopers: Vec<(Looper<R>, LopperTask<R>)> = Vec::new();

        // all loopers share the message queue.
        let (tx_msg, rx_msg) = flume::unbounded();

        // all loopers get their own cmd and sys_msg queues.
        for _ in 0..num_threads {
            let (tx_cmd, rx_cmd) = flume::unbounded();
            let (tx_msg_sys, rx_msg_sys) = flume::unbounded();
            let mb = Arc::new(Mailbox::new(rx_msg.clone(), rx_msg_sys));
            let mb_ref = mb.clone();

            loopers.push((
                Looper {
                    tx_cmd,
                    tx_msg: tx_msg.clone(),
                    tx_msg_sys,
                    // mb,
                },
                LopperTask {
                    rx_cmd: Some(rx_cmd),
                    mb_ref: Some(mb_ref),
                    processor: None,
                },
            ))
        }

        loopers
    }

    pub(crate) fn create_core<Fac>(
        factory: Fac,
        system: &SystemRef,
        g_type: &ActorGuardianType,
        address: ActorWeakAddr<R>,
        pool_size: Option<usize>,
    ) -> Result<(Arc<Self>, Vec<LopperTask<R>>, ActorExecutor), ActorCoreCreationError>
    where
        R: ActorReceiver,
        Fac: Fn(ActorWeakAddr<R>) -> R + Send + Sync + 'static,
    {
        // executor type
        let mut executor = ActorExecutor::System;
        let mut act_pool_size = 1;

        if let Some(pool_size) = pool_size {
            executor = ActorExecutor::Pool(Arc::new(Box::new(ThreadPoolExecutor::new(
                Some(pool_size),
                Some("_actor_"),
            ))));
            act_pool_size = pool_size;
        }

        // initialize the looopers
        let mut loopers_and_tasks = ActorCore::create_loopers(act_pool_size);

        // "guardian" will be None for other guardians as for a guardian the g_type is Root(dummy)
        let mut guardian = None;
        if let Some(g) = system.get_guardian_ref(g_type) {
            guardian = Some(g.downgrade());
        }

        let mut processors: Vec<Arc<Processor<R>>> = Vec::new();
        let mut loopers: Vec<Looper<R>> = Vec::new();
        let mut looper_tasks: Vec<LopperTask<R>> = Vec::new();
        let core = Arc::new_cyclic(|weak| {
            // create the corresponding processors for the loopers
            for i in 0..act_pool_size {
                let processor = Arc::new(Processor {
                    receiver: Arc::new(Mutex::new(Some(factory(address.clone())))),
                    system: system.clone(),
                    core: weak.clone(),
                });

                if let Some((looper, task)) = loopers_and_tasks.pop() {
                    processors.push(processor.clone());
                    loopers.push(looper);
                    looper_tasks.push(task);
                    looper_tasks[i].processor = Some(processor.clone());
                } else {
                    error!("ActorCore_create_core_error: loopers size mismatch");
                }
            }

            ActorCore {
                executor: executor.clone(),
                guardian,
                address: Some(address),
                // processors,
                loopers,
            }
        });

        Ok((core, looper_tasks, executor))
    }

    pub(crate) fn send(&self, env: Envelope<R>) -> Result<(), MessageSendError> {
        if self.loopers.len() < 1 {
            return Err(MessageSendError::ErrActorCoreCorrupted);
        }
        let mut result = Ok(());

        // message queued only once, as all loopers share queue
        self.loopers[0].tx_msg.send(Box::new(env)).map_err(|e| {
            error!("ActorCore::send::error {}", e);
            MessageSendError::ErrActorCoreCorrupted
        })?;

        // schedule the pool mailboxes to run. ??Handle multiple runs queued??
        for looper in &self.loopers {
            if let Err(e) = looper.tx_cmd.send(CoreCommand::Run) {
                result = Err(MessageSendError::ErrActorCoreCorrupted);
                error!("ActorCore::send::error {}", e);
                break;
            }
        }

        result
    }

    pub(crate) fn send_sys(&self, env: SystemEnvelope<R>) -> Result<(), MessageSendError> {
        if self.loopers.len() < 1 {
            return Err(MessageSendError::ErrActorCoreCorrupted);
        }
        let mut result = Ok(());

        // message queued only once, as all loopers share queue
        self.loopers[0]
            .tx_msg_sys
            .send(Box::new(env))
            .map_err(|e| {
                error!("ActorCore::send_sys::error {}", e);
                MessageSendError::ErrActorCoreCorrupted
            })?;

        for looper in &self.loopers {
            if let Err(e) = looper.tx_cmd.send(CoreCommand::Run) {
                result = Err(MessageSendError::ErrActorCoreCorrupted);
                error!("ActorCore::send_sys::error {}", e);
                break;
            }
        }

        result
    }

    pub(crate) fn schedule_mailbox(&self) {
        for looper in &self.loopers {
            if let Err(e) = looper.tx_cmd.send(CoreCommand::Run) {
                error!("schedule_mailbox_error {}", e);
                break;
            }
        }
    }

    pub(crate) fn set_pause(&self, b: bool) {
        for looper in &self.loopers {
            if let Err(e) = looper.tx_cmd.send(CoreCommand::Pause(b)) {
                error!("set_pause_error {}", e);
                break;
            }
        }
    }

    pub(crate) fn address(&self) -> Option<ActorWeakAddr<R>> {
        self.address.clone()
    }

    pub(crate) fn executor(&self) -> ActorExecutor {
        self.executor.clone()
    }

    pub(crate) fn stop(&self) {
        // sys.publish(Topic::DeadLetter)
        // sys.publish(SystemEvent::ActorTerminated)

        if let Some(g_ref) = &self.guardian {
            if let Some(g) = g_ref.upgrade() {
                if let Some(weak_ref) = &self.address {
                    if let Some(addr) = weak_ref.upgrade() {
                        for looper in &self.loopers {
                            if let Err(e) = looper.tx_cmd.send(CoreCommand::Terminate) {
                                error!("ActorCore::stop::error {}", e);
                                break;
                            }
                        }

                        let msg = SystemMessage::Event(SystemEvent::ActorTerminated(Addr(
                            Box::new(addr),
                        )));
                        if let Err(e) = g.tell_sys(msg) {
                            error!("ActorCore::stop::error {:?}", e);
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

// fields accessed when feature ipc-custer is active.
#[cfg_attr(not(feature = "ipc-cluster"), allow(dead_code))]
pub(crate) struct RemoteActorCore<R>
where
    R: ActorReceiver,
{
    pub(crate) address: ActorWeakAddr<R>,
    pub(crate) system: SystemWeakRef,
}

impl<R> RemoteActorCore<R>
where
    R: ActorReceiver,
{
    #[cfg(all(unix, feature = "ipc-cluster"))]
    pub(crate) fn new(address: ActorWeakAddr<R>, system: SystemWeakRef) -> Self {
        Self { address, system }
    }
}
