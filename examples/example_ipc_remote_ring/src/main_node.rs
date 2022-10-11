use clap::Parser;
use std::sync::atomic::Ordering;

use example_ipc_remote_ring::*;
use factor::{self, prelude::*};

#[cfg(debug_assertions)]
const WORKER_PATH: &str = "./target/debug/worker_node";

#[cfg(not(debug_assertions))]
const WORKER_PATH: &str = "./target/release/worker_node";

/// Main node of the factor cluster.
#[derive(Parser, Debug)]
struct Args {
    /// Number of nodes in the cluster.
    #[arg(long, default_value_t = 0)]
    send_ask: u8,

    /// maximun round trips around the ring.
    #[arg(long, default_value_t = 0)]
    num_rounds: u32,

    /// Number of nodes in the cluster.
    #[arg(long, default_value_t = 0)]
    num_nodes: NodeId,
}

async fn run_main() {
    let args = Args::parse();
    println!("send_ask: {}", args.send_ask);
    println!("num_ring_rounds: {}", args.num_rounds);
    println!("num_nodes: {}", args.num_nodes);

    let provider = build_type_provider();
    let system = factor::init_cluster(0, Some("main_node_system".to_owned()), provider).await;

    println!("init_cluster_done");

    let now = std::time::Instant::now();
    let mut node_ids = Vec::with_capacity((args.num_nodes - 1) as usize);
    for _ in 1..args.num_nodes {
        let config = NodeCreationConfig {
            exec_path: WORKER_PATH.to_owned(),
        };

        let node_id = system
            .spawn_worker_node(config)
            .await
            .expect("worker_node_spawn_failed");

        node_ids.push(node_id);
    }
    let spawn_duration = now.elapsed();
    println!("time_taken_to_spawn_worker_nodes: {:#?}", spawn_duration);

    let mut config = ActorBuilderConfig::default();
    config.actor_tag = Some(ROOT_ACTOR_ADDR.to_string());
    config.discovery = DiscoveryKey::Tag;
    let spawn_item = builder::ActorBuilder::create(|| MainActor(), &system, config);
    let addr = system.run_actor(spawn_item.unwrap());

    let msg = RingMessage {
        string_payload: "123456789_123456789_".to_owned(),
        remaining_rounds: args.num_rounds,
        max_nodes: args.num_nodes,
        next_node: 1,
        sum: 0,
    };

    if args.send_ask > 0 {
        let now = std::time::Instant::now();
        let ask_msg = RingMessageAsk(msg);
        addr.ask(ask_msg).await.expect("main_addr_ask_failed");
        let msg_duration = now.elapsed();
        println!(
            "time_taken_for_ring_messaging_to_complete: {:#?}",
            msg_duration
        );
    } else {
        let now = std::time::Instant::now();
        let tell_msg = RingMessageTell(msg);
        addr.tell(tell_msg).expect("main_addr_tell_failed");

        loop {
            if END_TEST.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }

        let msg_duration = now.elapsed();
        println!(
            "time_taken_for_ring_messaging_to_complete: {:#?}",
            msg_duration
        );
    }

    println!("terminating_worker_nodes");
    for node_id in node_ids {
        // println!("terminating_worker_node: {}", node_id);
        // std::thread::sleep(std::time::Duration::from_millis(30));

        assert_eq!(0, system.shutdown_worker_node(&node_id).await);
    }
    std::thread::sleep(std::time::Duration::from_millis(2000));

    println!("main_node_end");
}

/// cargo run --bin main_node --release -- --send-ask 1 --num-rounds 2 --num-nodes 2
#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() {
    run_main().await;
}
