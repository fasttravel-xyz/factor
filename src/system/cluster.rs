#![cfg(all(unix, feature = "ipc-cluster"))]

//!
//! `factor` provides only ipc-remote actors where cluster-nodes are process-nodes,
//! this is the only cluster model that factor intends to provide. `factor` doesn't
//! intend to provide any other cluster model e.g. multi-machine (physical/virtual)
//! cluster for static or dynamic horizontal-scaling, etc. as there are better and
//! established solutions already available that `factor` could utilize.
//!
//! This execution model is targeted towards applications that need to keep adding
//! dedicated resources in separate processes. This model is different from:
//!     * a cluster running with a pre-defined sets of node and all nodes sharing
//!       the application load (static)
//!     * a cluster adding new nodes when a load-balancing threshold is reached
//!       and it needs more resources (dynamic).
//!
//! `factor` cluster provides an execution model in which an actor-group (a group of
//! related actors) is launched in a worker process of the main process (or main-node).
//! By default worker nodes don't store or receive information about other worker
//! nodes, as number of worker-nodes will keep on growing and usually different
//! worker nodes will have unrelated actors. A worker node retrieves the information
//! about another worker node only when required (e.g. to resolve the address
//! of an actor that is running in a different worker node.)
//!

mod ipc_node;
mod receptionist;
mod types_provider;

use log::trace;
use tarpc::serde_transport as transport;
use tarpc::server::BaseChannel;
use tarpc::server::Channel;
use tarpc::tokio_serde::formats::Json;
use tarpc::tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio::net::UnixListener;

use once_cell::sync::OnceCell;

use crate::SystemRef;
use crate::*;
use ipc_node::*;

pub(crate) use ipc_node::RemoteNodeClient;
pub use ipc_node::{NodeCreationConfig, NodeId};
pub use types_provider::RemoteMessageTypeProvider;

pub(super) static PROCESS_CLUSTER_NODE: once_cell::sync::OnceCell<ClusterNode> = OnceCell::new();

const TEMP_DIR_PATH: &'static str = "/tmp";
const SOCKET_DIR_PATH: &'static str = "/tmp/factor";

fn get_node_bind_addr(node_id: &NodeId) -> String {
    format!(
        "{}/factor_ipc_remote_node_{}.sock",
        SOCKET_DIR_PATH, node_id
    )
}

fn check_socket_path(bind_addr: &str) {
    if !std::path::Path::new(TEMP_DIR_PATH).exists() {
        panic!("FATAL: factor needs /tmp dir and permissions to it for proper functioning.");
    }
    if !std::path::Path::new(SOCKET_DIR_PATH).exists() {
        std::fs::create_dir(SOCKET_DIR_PATH)
            .expect(format!("FATAL: {} creation failed.", SOCKET_DIR_PATH).as_str());
    }
    let _ = std::fs::remove_file(&bind_addr);
}

/// Initialize the cluster-node per process.
pub(crate) async fn init_cluster<C>(
    node_id: NodeId,
    name: Option<String>,
    r_msg_type_provider: RemoteMessageTypeProvider,
    closure: C,
) -> system::SystemRef
where
    C: FnOnce(system::SystemRef) + Send + 'static,
{
    let cluster = PROCESS_CLUSTER_NODE.get_or_init(move || {
        let tokio_rt = tokio::runtime::Handle::current();

        let sys = init_system_cluster(name, Some(node_id));
        let cluster = ClusterNode::new(node_id, sys, r_msg_type_provider, tokio_rt);

        trace!("factor_process_node_initialized");
        cluster
    });

    let system_moved = cluster.system();
    let cluster_moved = cluster.clone();

    let task = async move {
        run_node_server(&node_id, system_moved, cluster_moved, closure).await;
    };
    cluster
        .tokio_rt()
        .spawn(task)
        .await
        .expect("run_server_failed");

    cluster.system()
}

async fn run_node_server<C>(node_id: &NodeId, system: SystemRef, node: ClusterNode, closure: C)
where
    C: FnOnce(system::SystemRef),
{
    let bind_addr = get_node_bind_addr(&node_id);

    trace!("run_node_server_bind_addr: {}", bind_addr);
    check_socket_path(&bind_addr);

    let listener = UnixListener::bind(&bind_addr).unwrap();
    let codec_builder = LengthDelimitedCodec::builder();
    let node_server = node.clone();

    node.tokio_rt().spawn(async move {
        loop {
            let (conn, _addr) = listener.accept().await.unwrap();
            let framed = codec_builder.new_framed(conn);
            let transport = transport::new(framed, Json::default());

            let server = node_server.clone();
            let fut = BaseChannel::with_defaults(transport).execute(server.serve());

            trace!("server_spawned");
            node_server.tokio_rt().spawn(fut);
        }
    });

    // run the initialization closure before the first ping.
    (closure)(system.clone());

    // if not the main node, notify the main node. [todo] schedule a heartbeat using the system.
    let main_node = 0;
    if node_id > &main_node {
        if let Some(client) = node.get_node_client(main_node).await {
            client.ping(*node_id).await;
        }
    }
}
