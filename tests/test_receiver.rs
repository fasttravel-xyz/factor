use factor::{builder::ActorBuilderConfig, prelude::*};
use std::sync::atomic::{AtomicU8, Ordering};

static SUM: AtomicU8 = AtomicU8::new(0);
static ADD_CALL_COUNT: AtomicU8 = AtomicU8::new(0);
static SUB_CALL_COUNT: AtomicU8 = AtomicU8::new(0);

#[cfg(all(unix, feature = "ipc-cluster"))]
use serde::{Deserialize, Serialize};

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageAdd(u8);
impl Message for MessageAdd {
    type Result = Option<u8>;
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageSubtract(u8);
impl Message for MessageSubtract {
    type Result = Option<u8>;
}

struct OpsReceiver {}
impl ActorReceiver for OpsReceiver {
    type Context = BasicContext<Self>;
}

impl MessageHandler<MessageAdd> for OpsReceiver {
    type Result = MessageResponseType<<MessageAdd as Message>::Result>;

    fn handle(&mut self, msg: MessageAdd, _ctx: &mut Self::Context) -> Self::Result {
        SUM.fetch_add(msg.0, Ordering::SeqCst);
        ADD_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        MessageResponseType::Result(None.into())
    }
}

impl MessageHandler<MessageSubtract> for OpsReceiver {
    type Result = MessageResponseType<<MessageSubtract as Message>::Result>;

    fn handle(&mut self, msg: MessageSubtract, _ctx: &mut Self::Context) -> Self::Result {
        SUM.fetch_sub(msg.0, Ordering::SeqCst);
        SUB_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        MessageResponseType::Result(None.into())
    }
}

#[tokio::test]
async fn test_receiver() {
    let sys = factor::init_system(Some("TestSystem".to_string()));
    let spawn_item =
        builder::ActorBuilder::create(|_| OpsReceiver {}, &sys, ActorBuilderConfig::default());
    let addr = sys.run_actor(spawn_item.unwrap());

    // 0 + 3
    addr.tell(MessageAdd(3)).expect("addr_tell_failed");

    // 0 + 3 + 13
    addr.tell(MessageAdd(13)).expect("addr_tell_failed");

    // 0 + 3 + 13 - 5
    addr.tell(MessageSubtract(5)).expect("addr_tell_failed");

    // 0 + 3 + 13 - 5 - 6
    addr.tell(MessageSubtract(6)).expect("addr_tell_failed");

    // 0 + 3 + 13 - 5 - 6 + 3 = 8
    addr.tell(MessageAdd(3)).expect("addr_tell_failed");

    let now = std::time::Instant::now();
    loop {
        let elapsed_time = now.elapsed();
        if elapsed_time > std::time::Duration::from_millis(5000) {
            break; // maximum wait time is 5 seconds
        }

        if ADD_CALL_COUNT.load(Ordering::SeqCst) >= 3 && SUB_CALL_COUNT.load(Ordering::SeqCst) >= 2
        {
            break;
        } else {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    assert_eq!(ADD_CALL_COUNT.load(Ordering::SeqCst), 3);
    assert_eq!(SUB_CALL_COUNT.load(Ordering::SeqCst), 2);
    assert_eq!(SUM.load(Ordering::SeqCst), 8);
}
