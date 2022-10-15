use serde::{Deserialize, Serialize};

use factor::prelude::*;

#[derive(Clone)]
pub struct MockActor();

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MockMessage(pub String);

impl MessageCluster for MockMessage {
    type Result = Option<String>;
}

impl ActorReceiver for MockActor {
    type Context = BasicContext<Self>;
}

impl MessageClusterHandler<MockMessage> for MockActor {
    type Result = MessageResponseType<<MockMessage as MessageCluster>::Result>;

    fn handle(&mut self, _msg: MockMessage, _ctx: &mut Self::Context) -> Self::Result {
        println!("factor_message_from_main: {:#?} ", _msg);

        return MessageResponseType::Result(
            (Some("... factor_answer_from_client ...".to_owned())).into(),
        );
    }
}

pub fn build_type_provider() -> RemoteMessageTypeProvider {
    let mut provider = RemoteMessageTypeProvider::new();

    provider.register::<MockActor, MockMessage>();
    provider
}
