use factor::prelude::*;

struct MessageAdd(i8);
impl Message for MessageAdd {
    type Result = i8;
}

struct MessageSubtract(i8);
impl Message for MessageSubtract {
    type Result = i8;
}

struct MessageAddCount;
impl Message for MessageAddCount {
    type Result = u8;
}

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
    let spawn_item = builder::ActorBuilder::create(|| OpsReceiver::default(), &sys);
    let addr = sys.run_actor(spawn_item.unwrap());

    // 0 + 3
    let mut result = addr.ask(MessageAdd(3));
    assert!(result.is_ok());
    if let Ok(r) = result.unwrap().await {
        assert_eq!(r, 3);
    }

    // 0 + 3 + 13
    result = addr.ask(MessageAdd(13));
    assert!(result.is_ok());
    if let Ok(r) = result.unwrap().await {
        assert_eq!(r, 16);
    }

    // 0 + 3 + 13 - 5
    result = addr.ask(MessageSubtract(5));
    assert!(result.is_ok());
    if let Ok(r) = result.unwrap().await {
        assert_eq!(r, 11);
    }

    // 0 + 3 + 13 - 5 - 6
    result = addr.ask(MessageSubtract(6));
    assert!(result.is_ok());
    if let Ok(r) = result.unwrap().await {
        assert_eq!(r, 5);
    }

    // 0 + 3 + 13 - 5 - 6 + 3 = 8
    result = addr.ask(MessageAdd(3));
    assert!(result.is_ok());
    if let Ok(r) = result.unwrap().await {
        assert_eq!(r, 8);
    }

    let mut rcount = addr.ask(MessageAddCount);
    assert!(rcount.is_ok());
    if let Ok(count) = rcount.unwrap().await {
        assert_eq!(count, 3);
    }

    rcount = addr.ask(MessageSubCount);
    assert!(rcount.is_ok());
    if let Ok(count) = rcount.unwrap().await {
        assert_eq!(count, 2);
    }
}
