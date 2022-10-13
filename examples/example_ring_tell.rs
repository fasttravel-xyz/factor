// run using: cargo run --example example_ring_tell --release -- --num-rounds 2000 --num-nodes 5000

use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;
use factor::{builder::ActorBuilderConfig, prelude::*};

#[cfg(all(unix, feature = "ipc-cluster"))]
use serde::{Deserialize, Serialize};

static END_TEST: AtomicBool = AtomicBool::new(false);

/// Main node of the factor cluster.
#[derive(Parser, Debug)]
struct Args {
    /// maximun round trips around the ring.
    #[arg(long, default_value_t = 0)]
    num_rounds: u32,

    /// Number of nodes in the cluster.
    #[arg(long, default_value_t = 0)]
    num_nodes: u32,
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
pub struct UpdateNextAddr {
    pub next_addr: ActorAddr<RingActor>,
}

impl Message for UpdateNextAddr {
    type Result = ();
}

impl MessageHandler<UpdateNextAddr> for RingActor {
    type Result = MessageResponseType<<UpdateNextAddr as Message>::Result>;

    fn handle(&mut self, msg: UpdateNextAddr, _ctx: &mut Self::Context) -> Self::Result {
        self.next_addr = Some(msg.next_addr);
        return MessageResponseType::Result(().into());
    }
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
pub struct RingMessage {
    pub string_payload: String,
    pub remaining_sends: u32,
    pub sum: u32,
}

impl Message for RingMessage {
    type Result = ();
}

pub struct RingActor {
    pub next_addr: Option<ActorAddr<RingActor>>,
}

impl ActorReceiver for RingActor {
    type Context = BasicContext<Self>;
}

impl MessageHandler<RingMessage> for RingActor {
    type Result = MessageResponseType<<RingMessage as Message>::Result>;

    fn handle(&mut self, msg: RingMessage, _ctx: &mut Self::Context) -> Self::Result {
        let next_addr = self
            .next_addr
            .as_ref()
            .expect("next_addr_is_none_error")
            .clone();

        if msg.remaining_sends > 0 {
            let next_msg = RingMessage {
                remaining_sends: msg.remaining_sends - 1,
                sum: msg.sum + 1,
                ..msg
            };

            next_addr.tell(next_msg).expect("next_addr_tell_failed");
        } else {
            println!("final_sum_after_ring_messaging: {}", msg.sum);
            END_TEST.store(true, Ordering::SeqCst);
        }

        return MessageResponseType::Result(().into());
    }
}

// run using: cargo run --example example_ring_tell --release -- --num-rounds 2000 --num-nodes 5000
#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() {
    let args = Args::parse();
    println!("num_ring_rounds: {}", args.num_rounds);
    println!("num_nodes: {}", args.num_nodes);

    let sys = factor::init_system(Some("TestSystem".to_string()));

    let now = std::time::Instant::now();
    let spawn_item = ActorBuilder::create(
        |_| RingActor { next_addr: None },
        &sys,
        ActorBuilderConfig::default(),
    );
    let first_addr = sys.run_actor(spawn_item.unwrap());
    let mut next_addr = first_addr.clone();
    for _ in 1..args.num_nodes {
        let next_addr_moved = next_addr.clone();
        let spawn_item = ActorBuilder::create(
            move |_| RingActor {
                next_addr: Some(next_addr_moved.clone()),
            },
            &sys,
            ActorBuilderConfig::default(),
        );
        next_addr = sys.run_actor(spawn_item.unwrap());
    }
    first_addr
        .ask(UpdateNextAddr { next_addr })
        .await
        .expect("update_next_addr_failed");
    let elapsed = now.elapsed();
    println!("time_taken_for_node_creation: {:#?}", elapsed);

    let now = std::time::Instant::now();
    let num_sends: u32 = args.num_rounds * args.num_nodes;
    let msg = RingMessage {
        string_payload: "123456789_123456789_".to_owned(),
        remaining_sends: num_sends,
        sum: 0,
    };
    first_addr.tell(msg).expect("ring_msg_tell_failed");

    loop {
        if END_TEST.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let elapsed = now.elapsed();
    println!("time_taken_for_ring_messaging_tell: {:#?}", elapsed);
}
