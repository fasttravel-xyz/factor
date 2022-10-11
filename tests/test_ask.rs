use factor::{builder::ActorBuilderConfig, prelude::*};

#[cfg(all(unix, feature = "ipc-cluster"))]
use serde::{Deserialize, Serialize};

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageAdd(i8);
impl Message for MessageAdd {
    type Result = i8;
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageSubtract(i8);
impl Message for MessageSubtract {
    type Result = i8;
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageAddCount;
impl Message for MessageAddCount {
    type Result = u8;
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageSubCount;
impl Message for MessageSubCount {
    type Result = u8;
}

#[derive(Default)]
struct OpsReceiver {
    sum: i8,
    add_call_count: u8,
    sub_call_count: u8,
}

impl ActorReceiver for OpsReceiver {
    type Context = BasicContext<Self>;
}

impl MessageHandler<MessageAdd> for OpsReceiver {
    type Result = MessageResponseType<<MessageAdd as Message>::Result>;

    fn handle(&mut self, msg: MessageAdd, _ctx: &mut Self::Context) -> Self::Result {
        self.sum += msg.0;
        self.add_call_count += 1;
        MessageResponseType::Result(self.sum.into())
    }
}

impl MessageHandler<MessageSubtract> for OpsReceiver {
    type Result = MessageResponseType<<MessageSubtract as Message>::Result>;

    fn handle(&mut self, msg: MessageSubtract, _ctx: &mut Self::Context) -> Self::Result {
        self.sub_call_count += 1;
        self.sum -= msg.0;
        let sum = self.sum;

        let fut = {
            // [todo] We need to implement access of self to the future to
            // be able to do the computation inside the async.
            Box::pin(async move { sum })
        };
        MessageResponseType::Future(fut)
    }
}

impl MessageHandler<MessageAddCount> for OpsReceiver {
    type Result = MessageResponseType<<MessageAddCount as Message>::Result>;

    fn handle(&mut self, _msg: MessageAddCount, _ctx: &mut Self::Context) -> Self::Result {
        MessageResponseType::Result(self.add_call_count.into())
    }
}

impl MessageHandler<MessageSubCount> for OpsReceiver {
    type Result = MessageResponseType<<MessageSubCount as Message>::Result>;

    fn handle(&mut self, _msg: MessageSubCount, _ctx: &mut Self::Context) -> Self::Result {
        MessageResponseType::Result(self.sub_call_count.into())
    }
}

#[tokio::test]
async fn test_ask() {
    let sys = factor::init_system(Some("TestSystem".to_string()));
    let spawn_item = builder::ActorBuilder::create(
        || OpsReceiver::default(),
        &sys,
        ActorBuilderConfig::default(),
    );
    let addr = sys.run_actor(spawn_item.unwrap());

    // 0 + 3
    addr.ask(MessageAdd(3))
        .await
        .map(|sum| assert_eq!(sum, 3))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();

    // 0 + 3 + 13
    addr.ask(MessageAdd(13))
        .await
        .map(|sum| assert_eq!(sum, 16))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();

    // 0 + 3 + 13 - 5
    addr.ask(MessageSubtract(5))
        .await
        .map(|sum| assert_eq!(sum, 11))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();

    // 0 + 3 + 13 - 5 - 6
    addr.ask(MessageSubtract(6))
        .await
        .map(|sum| assert_eq!(sum, 5))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();

    // 0 + 3 + 13 - 5 - 6 + 3 = 8
    addr.ask(MessageAdd(3))
        .await
        .map(|sum| assert_eq!(sum, 8))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();

    addr.ask(MessageAddCount)
        .await
        .map(|count| assert_eq!(count, 3))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();

    addr.ask(MessageSubCount)
        .await
        .map(|count| assert_eq!(count, 2))
        .map_err(|e| panic!("factor_ask_error: {:?} ", e))
        .err();
}
