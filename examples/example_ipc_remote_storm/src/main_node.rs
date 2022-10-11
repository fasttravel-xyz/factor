use clap::Parser;
use std::sync::atomic::Ordering;

use example_ipc_remote_storm::*;
use factor::{self, prelude::*};

#[cfg(debug_assertions)]
const WORKER_PATH: &str = "./target/debug/worker_node";

#[cfg(not(debug_assertions))]
const WORKER_PATH: &str = "./target/release/worker_node";

/// Main node of the factor cluster.
#[derive(Parser, Debug)]
struct Args {
    /// number of messages from each node.
    #[arg(long, default_value_t = 0)]
    num_msgs: u32,

    /// Number of nodes in the cluster.
    #[arg(long, default_value_t = 0)]
    num_nodes: NodeId,
}

async fn run_main() {
    let args = Args::parse();
    println!("num_msgs: {}", args.num_msgs);
    println!("num_nodes: {}", args.num_nodes);

    let provider = build_type_provider();
    let system = factor::init_cluster(0, Some("main_node_system".to_owned()), provider).await;

    let now = std::time::Instant::now();
    let mut node_ids = Vec::with_capacity(args.num_nodes as usize);
    for _ in 1..=args.num_nodes {
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
    let spawn_item = builder::ActorBuilder::create(|| ServerActor(), &system, config);
    let _server_addr = system.run_actor(spawn_item.unwrap());

    let now = std::time::Instant::now();
    for i in 1..=args.num_nodes {
        let client_addr = system
            .get_remote_addr::<ClientActor>(i, ROOT_ACTOR_ADDR)
            .await
            .expect("client_remote_addr_get_failed");

        let start_storm = StartStorm {
            num_msgs: args.num_msgs,
            max_rps: 10000, // [todo] expose as cmd arg.
        };

        client_addr
            .tell(start_storm)
            .expect("client_tell_start_storm_failed");
    }
    println!("time_taken_to_schedule_storm: {:#?}", now.elapsed());

    loop {
        let end_test_counter = END_TEST_COUNTER.load(Ordering::SeqCst);
        // println!("value_of_END_TEST_COUNTER: {}", end_test_counter);
        if end_test_counter == args.num_nodes {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1000));
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

/// cargo run --bin main_node --release -- --num-msgs 20 --num-nodes 2
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    run_main().await;
}
