use clap::Parser;

use example_ipc_remote_ring::*;
use factor::{self, prelude::*};

/// Worker node of the factor cluster.
#[derive(Parser, Debug)]
struct Args {
    /// Node id of this node.
    #[arg(short, long, default_value_t = 0)]
    node_id: NodeId,
}

async fn run_worker() {
    let args = Args::parse();
    let provider = build_type_provider();
    let system = factor::init_cluster(
        args.node_id,
        Some("worker_node_system".to_owned()),
        provider,
    )
    .await;

    let mut config = ActorBuilderConfig::default();
    config.actor_tag = Some(ROOT_ACTOR_ADDR.to_string());
    config.discovery = DiscoveryKey::Tag;
    let spawn_item = builder::ActorBuilder::create(|_| WorkerActor(), &system, config);
    let _addr = system.run_actor(spawn_item.unwrap());
}

// run the main node, not this worker node. main node will spawn this.
// cargo run --bin main_node --release --  --send-ask 1 --num-rounds 2 --num-nodes 20
#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() {
    run_worker().await;

    loop {
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }
}
