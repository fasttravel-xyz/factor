use factor::{builder::ActorBuilderConfig, init_system, prelude::*};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

static SUM: AtomicU8 = AtomicU8::new(0);
static BREAK_LOOP: AtomicBool = AtomicBool::new(false);

#[cfg(all(unix, feature = "ipc-cluster"))]
use serde::{Deserialize, Serialize};

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageAdd(u8);
impl Message for MessageAdd {
    type Result = ();
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageLoop;
impl Message for MessageLoop {
    type Result = ();
}

struct OpsReceiver {}
impl ActorReceiver for OpsReceiver {
    type Context = BasicContext<Self>;
}

impl MessageHandler<MessageLoop> for OpsReceiver {
    type Result = MessageResponseType<<MessageLoop as Message>::Result>;

    fn handle(&mut self, _msg: MessageLoop, _ctx: &mut Self::Context) -> Self::Result {
        loop {
            // println!("one_thread_is_doing_long_computaion");
            std::thread::sleep(Duration::from_millis(200));
            if BREAK_LOOP.load(Ordering::SeqCst) {
                break;
            }
        }

        MessageResponseType::Result(().into())
    }
}

impl MessageHandler<MessageAdd> for OpsReceiver {
    type Result = MessageResponseType<<MessageAdd as Message>::Result>;

    fn handle(&mut self, msg: MessageAdd, _ctx: &mut Self::Context) -> Self::Result {
        SUM.fetch_add(msg.0, Ordering::SeqCst);

        MessageResponseType::Result(().into())
    }
}

#[tokio::test]
async fn test_actor_pool() {
    let system = init_system(Some("test_system".to_owned()));

    let mut config = ActorBuilderConfig::default();
    config.pool_size = Some(2);

    let spawn_item = builder::ActorBuilder::create_pool(|_| OpsReceiver {}, &system, config);
    let addr = system.run_actor(spawn_item.unwrap());
    let msg_addr = addr.message_addr::<MessageLoop>();
    let ops_addr = addr.message_addr::<MessageAdd>();

    // lock one thread of the actorpool with long computation.
    msg_addr.tell(MessageLoop).expect("addr_tell_failed");

    std::thread::sleep(Duration::from_millis(1000));

    // send more operations for the other thread.
    ops_addr.ask(MessageAdd(3)).await.expect("addr_ask_failed");
    ops_addr.ask(MessageAdd(3)).await.expect("addr_ask_failed");
    ops_addr.ask(MessageAdd(3)).await.expect("addr_ask_failed");
    ops_addr.ask(MessageAdd(3)).await.expect("addr_ask_failed");
    ops_addr.ask(MessageAdd(3)).await.expect("addr_ask_failed");

    let sum = SUM.load(Ordering::SeqCst);

    assert_eq!(
        sum, 15,
        "actor_pool_failed_to_perform_operations_while_one_thread_blocked_on_long_computation"
    );
    BREAK_LOOP.store(true, Ordering::SeqCst);

    std::thread::sleep(Duration::from_millis(300));
}
