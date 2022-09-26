use factor::prelude::*;
use std::sync::atomic::{AtomicI8, AtomicU8, Ordering};

static SUM: AtomicI8 = AtomicI8::new(0);
static SUB_CALL_COUNT: AtomicU8 = AtomicU8::new(0);
static INIT_MSG: &str = "init finalized";

struct MessageAdd(i8);
impl Message for MessageAdd {
    type Result = i8;
}

struct MessageSub(i8);
impl Message for MessageSub {
    type Result = i8;
}

enum MessageCount {
    Add,
    Sub,
}
impl Message for MessageCount {
    type Result = u8;
}

struct MessageInItStr();
impl Message for MessageInItStr {
    type Result = String;
}

struct OpsReceiver {
    init_msg: String,
    add_call_count: u8,
    sub_call_count: u8,
}
impl Default for OpsReceiver {
    fn default() -> Self {
        OpsReceiver {
            init_msg: String::default(),
            add_call_count: 0,
            sub_call_count: 0,
        }
    }
}
impl ActorReceiver for OpsReceiver {
    type Context = BasicContext<Self>;

    fn finalize_init(&mut self, _ctx: &mut Self::Context) {
        self.init_msg = INIT_MSG.to_string();
    }

    fn finalize_stop(&mut self, _ctx: &mut Self::Context) {
        // persist the call count on terminate
        SUB_CALL_COUNT.store(self.sub_call_count, Ordering::SeqCst);
    }
}

impl MessageHandler<MessageAdd> for OpsReceiver {
    type Result = MessageResponseType<<MessageAdd as Message>::Result>;

    fn handle(&mut self, msg: MessageAdd, _ctx: &mut Self::Context) -> Self::Result {
        SUM.fetch_add(msg.0, Ordering::SeqCst);
        self.add_call_count += 1;
        MessageResponseType::Result(SUM.load(Ordering::SeqCst).into())
    }
}

impl MessageHandler<MessageSub> for OpsReceiver {
    type Result = MessageResponseType<<MessageSub as Message>::Result>;

    fn handle(&mut self, msg: MessageSub, _ctx: &mut Self::Context) -> Self::Result {
        SUM.fetch_sub(msg.0, Ordering::SeqCst);
        self.sub_call_count += 1;
        MessageResponseType::Result(SUM.load(Ordering::SeqCst).into())
    }
}

impl MessageHandler<MessageCount> for OpsReceiver {
    type Result = MessageResponseType<<MessageCount as Message>::Result>;

    fn handle(&mut self, msg: MessageCount, _ctx: &mut Self::Context) -> Self::Result {
        let count;
        match msg {
            MessageCount::Add => count = self.add_call_count,
            MessageCount::Sub => count = self.sub_call_count,
        };

        MessageResponseType::Result(count.into())
    }
}

impl MessageHandler<MessageInItStr> for OpsReceiver {
    type Result = MessageResponseType<<MessageInItStr as Message>::Result>;

    fn handle(&mut self, _msg: MessageInItStr, _ctx: &mut Self::Context) -> Self::Result {
        MessageResponseType::Result(self.init_msg.clone().into())
    }
}

#[tokio::test]
async fn test_init_stop() {
    let sys = factor::init_system(Some("TestSystem".to_string()));

    let mut spawn_item = builder::ActorBuilder::create(|| OpsReceiver::default(), &sys);
    let addr_a = sys.run_actor(spawn_item.unwrap());
    addr_a.ask(MessageInItStr()).await
        .map(|msg| assert_eq!(msg, INIT_MSG) )
        .map_err(|e| panic!("ask_init_msg_error: {:?}", e))
        .err();


    spawn_item = builder::ActorBuilder::create(|| OpsReceiver::default(), &sys);
    let addr_b = sys.run_actor(spawn_item.unwrap());

    // loop 5 times
    for _ in 0..5 {
        addr_a.tell(MessageAdd(5)).expect("tell_add_error");
        addr_b.tell(MessageSub(3)).expect("tell_sub_error");
    }

    addr_a.ask(MessageCount::Add).await
        .map(|count| assert_eq!(count, 5) )
        .map_err(|e| panic!("ask_add_error: {:?}", e))
        .err();

    addr_a.ask(MessageCount::Sub).await
        .map(|count| assert_eq!(count, 0) )
        .map_err(|e| panic!("ask_sub_error: {:?}", e))
        .err();

    addr_b.tell_sys(SystemMessage::Command(SystemCommand::ActorStop))
        .expect("sys_cmd_actor_stop_error");

    // We could add a SystemCommand::MailboxFlush to flush all the user
    // messages and wait on that command. But flushing the mailbox is no
    // guarantee that all computations are completed, as message handlers
    // could spawn async computation. A complete solution to flush will
    // require: (1) blocking/buffering any new messages, (2) running all
    // pending tasks in an user dedicated executor to completion,
    // (3) keeping track of new spawns from the message handlers and
    // running them to completion as well, (4) terminating any task that
    // does not complete within a set time interval.
    std::thread::sleep(std::time::Duration::from_millis(3000));

    addr_b.ask(MessageCount::Add).await
        .expect_err("ask_after_stop_error");

    assert_eq!(SUM.load(Ordering::SeqCst), 10);
    assert_eq!(SUB_CALL_COUNT.load(Ordering::SeqCst), 5);
}
