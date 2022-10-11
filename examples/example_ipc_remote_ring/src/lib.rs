use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

use factor::prelude::*;

pub const ROOT_ACTOR_ADDR: &'static str = "root_actor_addr";
pub static END_TEST: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
pub struct WorkerActor();

#[derive(Clone)]
pub struct MainActor();

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RingMessage {
    pub string_payload: String,
    pub remaining_rounds: u32,
    pub max_nodes: NodeId,
    pub next_node: NodeId,
    pub sum: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RingMessageAsk(pub RingMessage);

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RingMessageTell(pub RingMessage);

impl Message for RingMessageAsk {
    type Result = ();
}

impl Message for RingMessageTell {
    type Result = ();
}

impl ActorReceiver for WorkerActor {
    type Context = BasicContext<Self>;
}

impl MessageHandler<RingMessageAsk> for WorkerActor {
    type Result = MessageResponseType<<RingMessageAsk as Message>::Result>;

    fn handle(&mut self, msg: RingMessageAsk, ctx: &mut Self::Context) -> Self::Result {
        // println!("worker_ring_message_received: {:#?} ", msg);

        let next_msg = RingMessageAsk(RingMessage {
            next_node: msg.0.next_node + 1,
            sum: msg.0.sum + 1,
            ..msg.0
        });

        let system = ctx.system().clone();

        let task = async move {
            if msg.0.next_node == msg.0.max_nodes {
                let main_addr = system
                    .get_remote_addr::<MainActor>(0, ROOT_ACTOR_ADDR)
                    .await
                    .expect("get_root_addr_failed_for_main");
                main_addr.ask(next_msg).await.expect("main_addr_ask_failed");
            } else {
                let next_addr = system
                    .get_remote_addr::<WorkerActor>(msg.0.next_node, ROOT_ACTOR_ADDR)
                    .await
                    .expect("get_worker_root_addr_failed");
                next_addr.ask(next_msg).await.expect("next_addr_ask_failed");
            }
        };

        // uncomment to test rpc-timeout, even with small rounds(4 rounds) and nodes(2 nodes)
        // std::thread::sleep(std::time::Duration::from_millis(3000));

        return MessageResponseType::Future(Box::pin(task));
    }
}

impl MessageHandler<RingMessageTell> for WorkerActor {
    type Result = MessageResponseType<<RingMessageTell as Message>::Result>;

    fn handle(&mut self, msg: RingMessageTell, ctx: &mut Self::Context) -> Self::Result {
        // println!("worker_ring_message_received: {:#?} ", msg);

        let next_msg = RingMessageTell(RingMessage {
            next_node: msg.0.next_node + 1,
            sum: msg.0.sum + 1,
            ..msg.0
        });

        let system = ctx.system().clone();

        let task = async move {
            if msg.0.next_node == msg.0.max_nodes {
                let main_addr = system
                    .get_remote_addr::<MainActor>(0, ROOT_ACTOR_ADDR)
                    .await
                    .expect("get_root_addr_failed_for_main");
                main_addr.tell(next_msg).expect("main_addr_tell_failed");
            } else {
                let next_addr = system
                    .get_remote_addr::<WorkerActor>(msg.0.next_node, ROOT_ACTOR_ADDR)
                    .await
                    .expect("get_worker_root_addr_failed");
                next_addr.tell(next_msg).expect("next_addr_tell_failed");
            }
        };

        ctx.spawn_ok(task);

        return MessageResponseType::Result(().into());
    }
}

impl ActorReceiver for MainActor {
    type Context = BasicContext<Self>;
}

impl MessageHandler<RingMessageAsk> for MainActor {
    type Result = MessageResponseType<<RingMessageAsk as Message>::Result>;

    fn handle(&mut self, msg: RingMessageAsk, ctx: &mut Self::Context) -> Self::Result {
        // println!("main_ring_message_received: {:#?} ", msg);

        let system = ctx.system().clone();

        let task = async move {
            if msg.0.remaining_rounds > 0 {
                let next_msg = RingMessageAsk(RingMessage {
                    remaining_rounds: msg.0.remaining_rounds - 1,
                    next_node: 2,
                    sum: msg.0.sum + 1,
                    ..msg.0
                });

                let next_addr = system
                    .get_remote_addr::<WorkerActor>(1, ROOT_ACTOR_ADDR)
                    .await
                    .expect("get_worker_root_addr_failed");
                next_addr.ask(next_msg).await.expect("next_addr_ask_failed");
            } else {
                println!("ring_complete_final_sum: {}", msg.0.sum);
            }
        };

        return MessageResponseType::Future(Box::pin(task));
    }
}

impl MessageHandler<RingMessageTell> for MainActor {
    type Result = MessageResponseType<<RingMessageTell as Message>::Result>;

    fn handle(&mut self, msg: RingMessageTell, ctx: &mut Self::Context) -> Self::Result {
        // println!("main_ring_message_received: {:#?} ", msg);

        let system = ctx.system().clone();

        let task = async move {
            if msg.0.remaining_rounds > 0 {
                let next_msg = RingMessageTell(RingMessage {
                    remaining_rounds: msg.0.remaining_rounds - 1,
                    next_node: 2,
                    sum: msg.0.sum + 1,
                    ..msg.0
                });

                let next_addr = system
                    .get_remote_addr::<WorkerActor>(1, ROOT_ACTOR_ADDR)
                    .await
                    .expect("get_worker_root_addr_failed");
                next_addr.tell(next_msg).expect("next_addr_tell_failed");
            } else {
                println!("ring_complete_final_sum: {}", msg.0.sum);
                END_TEST.store(true, Ordering::SeqCst);
            }
        };

        ctx.spawn_ok(task);

        return MessageResponseType::Result(().into());
    }
}

pub fn build_type_provider() -> RemoteMessageTypeProvider {
    let mut provider = RemoteMessageTypeProvider::new();

    provider.register::<MainActor, RingMessageAsk>();
    provider.register::<MainActor, RingMessageTell>();
    provider.register::<WorkerActor, RingMessageAsk>();
    provider.register::<WorkerActor, RingMessageTell>();
    provider
}
