use example_ipc_remote_ask::*;
use factor::{self, prelude::*};

#[cfg(debug_assertions)]
const WORKER_PATH: &str = "./target/debug/worker_node";

#[cfg(not(debug_assertions))]
const WORKER_PATH: &str = "./target/release/worker_node";

async fn run_main() {

    let provider = build_type_provider();
    let system = factor::init_cluster(0, Some("main_node_system".to_owned()), provider).await;

    let config = NodeCreationConfig {
        exec_path: WORKER_PATH.to_owned(),
    };
    let node_id = system
        .spawn_worker_node(config)
        .await
        .expect("worker_node_spawn_failed");

    if let Some(addr) = system
        .get_remote_addr::<MockActor>(node_id, "node_root_actor")
        .await
    {
        addr.tell_addr(MockMessage("... Hello From Main ...".to_owned()))
            .expect("remote_tell_failed");

        let answer = addr
            .ask_addr(MockMessage("... Query From Main ...".to_owned()))
            .await
            .expect("remote_ask_failed");

        println!("main_ask_answer: {:#?}", answer);
    } else {
        panic!("worker_node_root_addr_not_available");
    }

    println!("terminating_worker_node");
    assert_eq!(0, system.shutdown_worker_node(&node_id).await);
    std::thread::sleep(std::time::Duration::from_millis(2000));

    println!("main_node_end");
}

/// cargo run --bin main_node
#[tokio::main]
async fn main() {
    run_main().await;
}
