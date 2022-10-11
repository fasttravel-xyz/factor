use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU16, Ordering};

use factor::prelude::*;

pub const ROOT_ACTOR_ADDR: &'static str = "root_actor_addr";
pub static END_TEST_COUNTER: AtomicU16 = AtomicU16::new(0);

#[derive(Clone)]
pub struct ServerActor();

#[derive(Clone)]
pub struct ClientActor(pub NodeId);

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StormMessage {
    pub end_test: bool,
    pub sum: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StartStorm {
    // total messages to be sent.
    pub num_msgs: u32,
    // maximum requests per seconds.
    pub max_rps: u32,
}

impl Message for StormMessage {
    type Result = u32;
}

impl Message for StartStorm {
    type Result = ();
}

impl ActorReceiver for ClientActor {
    type Context = BasicContext<Self>;
}

impl MessageHandler<StartStorm> for ClientActor {
    type Result = MessageResponseType<<StartStorm as Message>::Result>;

    fn handle(&mut self, msg: StartStorm, ctx: &mut Self::Context) -> Self::Result {
        // println!("client_start_storm_received: {:#?} ", msg);

        let node_id = self.0;
        let system = ctx.system();
        let task = async move {
            let mut end_test = false;
            let mut sum = 0;
            let server_addr = system
                .get_remote_addr::<ServerActor>(0, ROOT_ACTOR_ADDR)
                .await
                .expect("get_root_addr_failed_for_main");

            let now = std::time::Instant::now();
            for i in 1..=msg.num_msgs {
                if i == msg.num_msgs {
                    end_test = true;
                }
                let storm_msg = StormMessage { end_test, sum };

                sum = server_addr
                    .ask(storm_msg)
                    .await
                    .expect("server_addr_ask_failed");
            }
            let elapsed = now.elapsed();
            println!(
                "time_taken_to_storm_from_node_id_{}_: {:#?}",
                node_id, elapsed
            );

            println!("final_sum_from_node_id_{}_: {}", node_id, sum);
        };

        ctx.spawn_ok(task);

        return MessageResponseType::Result(().into());
    }
}

impl ActorReceiver for ServerActor {
    type Context = BasicContext<Self>;
}

impl MessageHandler<StormMessage> for ServerActor {
    type Result = MessageResponseType<<StormMessage as Message>::Result>;

    fn handle(&mut self, msg: StormMessage, _ctx: &mut Self::Context) -> Self::Result {
        // println!("server_storm_message_received: {:#?} ", msg);
        let sum = msg.sum + 1;

        if msg.end_test {
            END_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        }

        return MessageResponseType::Result(sum.into());
    }
}

pub fn build_type_provider() -> RemoteMessageTypeProvider {
    let mut provider = RemoteMessageTypeProvider::new();

    provider.register::<ServerActor, StormMessage>();
    provider.register::<ClientActor, StartStorm>();
    provider
}
