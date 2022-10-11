use dashmap::DashMap;
use futures::channel::oneshot;
use log::{error, trace, warn};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU16, Ordering::SeqCst};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tarpc::context::Context;
use tarpc::serde_transport as transport;
use tarpc::tokio_serde::formats::Json;
use tarpc::tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio::net::UnixStream;
use tokio::runtime::Handle as TokioRtHandle;

use crate::actor::{ActorId, DiscoveryKey};
use crate::{ActorAddr, ActorReceiver, Addr, Message, MessageHandler, SystemRef};

use super::get_node_bind_addr;
use super::receptionist::*;
use super::types_provider::*;
use crate::system::resolve_actor_addr_in_current_node;

pub type NodeId = u16;

// [todo] this is clonnable, move all members to Arc<Inner>, to avoid unnecessary copy.
#[derive(Clone)]
pub(crate) struct RemoteNodeClient {
    // Keeping the tokio_rt in case the ipc_node APIs need tokio reactor in future.
    // We have to spawn the ipc_node.api().await inside the tokio_rt (see ping below).
    _tokio_rt: TokioRtHandle,
    ipc_node: Arc<IpcNodeClient>,
    r_msg_type_provider: RemoteMessageTypeProvider,
}

impl RemoteNodeClient {
    fn new(
        _tokio_rt: TokioRtHandle,
        ipc_node: Arc<IpcNodeClient>,
        r_msg_type_provider: RemoteMessageTypeProvider,
    ) -> Self {
        Self {
            _tokio_rt,
            ipc_node,
            r_msg_type_provider,
        }
    }

    pub(super) async fn ping(&self, node_id: NodeId) -> i8 {
        /*
        ================================================================
        EXAMPLE SAMPLE IN CASE WE NEED TO SPAWN THESE CALLS IN TOKIO_RT.
        WE MIGHT NEED TO DO SO, IF TARPC STARTS USING tokio::spawn() or
        SOME RESOURCE THAT NEEDS THE TOKIO IO/TIMER DRIVER.
        ================================================================
        let ipc_node = self.ipc_node.clone();
        let task = async move {ipc_node
            .ping(tarpc::context::current(), node_id)
            .await
            .map_err(|e| error!("ping_failed: {}", e))
            .unwrap_or(1)
        };
        self._tokio_rt.spawn(task).await.expect("tokio_spawn_failed")
        ===============================================================
        */

        let ret = self
            .ipc_node
            .ping(tarpc::context::current(), node_id)
            .await
            .map_err(|e| error!("ping_failed: {}", e))
            .unwrap_or(1);

        ret
    }

    pub(super) async fn shutdown(&self) -> i8 {
        let ret = self
            .ipc_node
            .shutdown(tarpc::context::current())
            .await
            .map_err(|e| error!("shutdown_notification_failed: {}", e))
            .unwrap_or(1);

        ret
    }

    pub(crate) async fn get_remote_addr<R>(&self, key: &str) -> Option<ActorAddr<R>>
    where
        R: ActorReceiver,
    {
        if let Ok(ret) = self
            .ipc_node
            .get_remote_addr(tarpc::context::current(), key.to_string())
            .await
        {
            if let Some(aid) = ret {
                return Some(resolve_actor_addr_in_current_node::<R>(aid));
            }
        }

        None
    }

    pub(crate) async fn tell<R, M>(&self, addr: ActorAddr<R>, msg: M)
    where
        R: ActorReceiver + MessageHandler<M> + 'static,
        M: Message + Send + 'static,
    {
        let msg_json = self.r_msg_type_provider.encode(addr, msg).unwrap();
        let _ret = self
            .ipc_node
            .tell(tarpc::context::current(), msg_json.0, msg_json.1)
            .await;
    }

    pub(crate) async fn ask<R, M>(&self, addr: ActorAddr<R>, msg: M) -> Result<M::Result, ()>
    where
        R: ActorReceiver + MessageHandler<M> + 'static,
        M: Message + Send + 'static,
        M::Result: 'static,
    {
        let msg_json = self.r_msg_type_provider.encode(addr, msg).unwrap();
        let ret = self
            .ipc_node
            .ask(tarpc::context::current(), msg_json.0, msg_json.1)
            .await
            .map_err(|e| error!("remote_node_client_rpc_error: {}", e))?
            .ok_or(())?;

        serde_json::from_str::<M::Result>(&ret)
            .map_err(|e| error!("ask_m_result_deserialization_failed: {}", e))
    }
}

#[tarpc::service]
pub(super) trait IpcNode {
    // CORE
    async fn ping(node_id: NodeId) -> i8;
    async fn get_remote_addr(key: String) -> Option<ActorId>;
    async fn shutdown() -> i8;

    // ACTOR
    async fn tell(tinfo_json: String, r_msg_json: String) -> Option<String>;
    async fn ask(tinfo_json: String, r_msg_json: String) -> Option<String>;
}

struct NodeCreationPromise {
    tx: oneshot::Sender<NodeId>,
}

impl NodeCreationPromise {
    fn complete(self, node_id: NodeId) {
        self.tx
            .send(node_id)
            .map_err(|e| error!("node_creation_promise_complete_failed_for_node: {}", e))
            .err();
    }

    fn cancel(self) {
        // consume self, as sender drop before send will result in cancellation.
    }
}

pub struct NodeCreationConfig {
    pub exec_path: String,
}

#[derive(Clone)]
pub(crate) struct ClusterNode(Arc<ClusterNodeInner>);

impl ClusterNode {
    pub(crate) fn new(
        node_id: NodeId,
        system: SystemRef,
        r_msg_type_provider: RemoteMessageTypeProvider,
        tokio_rt: TokioRtHandle,
    ) -> Self {
        let inner = Arc::new_cyclic(|weak_self| {
            ClusterNodeInner::new(weak_self, node_id, system, r_msg_type_provider, tokio_rt)
        });

        Self(inner)
    }

    pub(crate) fn tokio_rt(&self) -> &TokioRtHandle {
        &self.0.tokio_rt
    }

    pub(crate) fn system(&self) -> SystemRef {
        self.0.system.clone()
    }

    pub(crate) fn add_to_addr_book(&self, addr: Addr) {
        self.0.add_to_addr_book(addr)
    }

    pub(crate) async fn get_node_client(&self, node_id: NodeId) -> Option<RemoteNodeClient> {
        if let Some(client) = self.0.nodes_clients.get(&node_id) {
            return Some(client.clone());
        }

        let inner = self.0.clone();
        let task = async move { inner.get_node_client(&node_id).await };
        self.tokio_rt().spawn(task).await.unwrap_or(None)
    }

    pub(crate) async fn spawn_worker_node(&self, config: NodeCreationConfig) -> Result<NodeId, ()> {
        self.0.spawn_worker_node(config).await
    }

    pub(crate) async fn shutdown_worker_node(&self, node_id: &NodeId) -> i8 {
        self.0.shutdown_worker_node(node_id).await
    }
}

// NOTE: We spawn all ipc-related calls in the tokio_rt as they need the tokio IO driver.
// We want tokio dependency only when ipc-cluster feature is active. Routing the
// calls to tokio_rt also avoids runtime error if the polling initiates from the
// ThreadPool executor and not tokio_rt thread. This adds some performance cost,
// but for now we want to keep the option open about which executor to use in our
// main-executor (system/actorpool executors currently use futures::executor::ThreadPool)
struct ClusterNodeInner {
    weak_self: Weak<Self>,
    tokio_rt: TokioRtHandle,
    #[allow(dead_code)]
    node_id: NodeId,
    system: SystemRef,
    nodes_clients: DashMap<NodeId, RemoteNodeClient>,
    nodes_heartbeat: DashMap<NodeId, Instant>,
    creation_promises: DashMap<NodeId, NodeCreationPromise>,
    r_msg_type_provider: RemoteMessageTypeProvider,
    receptionist: Receptionist,
    nodes_id_counter: AtomicU16,
    nodes_processes: DashMap<NodeId, Child>,
}

impl ClusterNodeInner {
    fn new(
        weak_self: &Weak<Self>,
        node_id: NodeId,
        system: SystemRef,
        r_msg_type_provider: RemoteMessageTypeProvider,
        tokio_rt: TokioRtHandle,
    ) -> Self {
        Self {
            weak_self: weak_self.clone(),
            tokio_rt,
            node_id,
            system,
            nodes_clients: DashMap::new(),
            nodes_heartbeat: DashMap::new(),
            creation_promises: DashMap::new(),
            r_msg_type_provider,
            receptionist: Receptionist::new(),
            nodes_id_counter: AtomicU16::new(1),
            nodes_processes: DashMap::new(),
        }
    }

    fn add_to_addr_book(&self, addr: Addr) {
        match addr.get_id().discovery() {
            DiscoveryKey::Tag => {
                if let Some(key) = addr.get_id().tag() {
                    self.receptionist.set_address(key.as_str(), addr);
                }
            }
            DiscoveryKey::Uuid => {
                let key = addr.get_id().uuid_str();
                self.receptionist.set_address(key.as_str(), addr);
            }
            _ => {}
        }
    }

    async fn get_node_client(&self, node_id: &NodeId) -> Option<RemoteNodeClient> {
        let bind_addr = get_node_bind_addr(node_id);

        trace!("client_bind_addr: {}", bind_addr);

        let conn = UnixStream::connect(bind_addr).await.unwrap();
        let codec_builder = LengthDelimitedCodec::builder();
        let transport = transport::new(codec_builder.new_framed(conn), Json::default());

        let new_client = IpcNodeClient::new(Default::default(), transport);
        let client = new_client.spawn();
        let r_node = RemoteNodeClient::new(
            self.tokio_rt.clone(),
            Arc::new(client),
            self.r_msg_type_provider.clone(),
        );

        self.nodes_clients.insert(node_id.clone(), r_node.clone());

        Some(r_node)
    }

    async fn spawn_worker_node(&self, config: NodeCreationConfig) -> Result<NodeId, ()> {
        // Currently, worker node creation only allowed from the main node.
        if self.node_id != 0 {
            return Err(());
        }

        let next_id = self.nodes_id_counter.load(SeqCst);
        let (tx, rx) = oneshot::channel::<NodeId>();
        let promise = NodeCreationPromise { tx };

        self.creation_promises.insert(next_id, promise);
        // schedule a task to cancel the promise with timeout.
        let weak_self_moved = self.weak_self.clone();
        let cancel_task = async move {
            weak_self_moved.upgrade().map(|node| {
                node.creation_promises
                    .remove(&next_id)
                    .map(|pair| pair.1.cancel());
            });
        };
        self.system
            .schedule_once(Box::pin(cancel_task), Duration::from_secs(5))
            .map_err(|e| error!("node_creation_promise_cancel_task_schedule_failed: {:?}", e))
            .err();

        // spawn the worker process
        let ret = Command::new(config.exec_path.as_str())
            .arg("--node-id")
            .arg(next_id.to_string())
            .spawn()
            .map_err(|e| {
                error!(
                    "worker_node_spawn_failed_for_node: {}, with_error: {}",
                    next_id, e
                );
                self.creation_promises.remove(&next_id);
            });

        // increment the next_id even if spawn fails.
        self.nodes_id_counter.store(next_id + 1, SeqCst);

        if let Ok(child) = ret {
            self.nodes_processes.insert(next_id, child);

            return rx.await.map_err(|e| {
                error!(
                    "worker_node_init_ping_failed_for_node: {}, with_error: {}",
                    next_id, e
                );
                ()
            });
        }

        Err(())
    }

    async fn shutdown_worker_node(&self, node_id: &NodeId) -> i8 {
        // Currently, worker node termination only allowed from the main node.
        if self.node_id != 0 {
            warn!("worker_node_termination_onlly_allowed_from_main_node");
            return 1;
        }

        trace!("Shutdown!! node: {}", node_id);

        // inform the node about the shutdown request.
        if let Some(client) = self.get_node_client(&node_id).await {
            client.shutdown().await;
        }

        // schedule to force terminate the worker process after certain duration.
        // [todo] expose the force-kill-wait duration to user-config.
        let force_kill_wait_duration = Duration::from_secs(1);
        if let Some(mut pair) = self.nodes_processes.remove(&node_id) {
            let task = async move {
                pair.1
                    .kill()
                    .map(|_| trace!("worker_node_terminated_ok"))
                    .map_err(|e| error!("worker_node_termination_failed: {}", e))
                    .err();
            };

            self.system
                .schedule_once(Box::pin(task), force_kill_wait_duration)
                .map_err(|e| error!("shutdown_schedule_failed: {:?}", e))
                .err();

            return 0;
        }

        1
    }
}

#[tarpc::server]
impl IpcNode for ClusterNode {
    async fn ping(self, _: Context, node_id: NodeId) -> i8 {
        trace!("Ping! from node: {}", node_id);

        // If this is the main node, resolve the node creation promise to notify the requester on first ping.
        if self.0.node_id == 0 && !self.0.nodes_heartbeat.contains_key(&node_id) {
            trace!("first_ping_from_node: {}", node_id);

            self.0
                .creation_promises
                .remove(&node_id)
                .map(|pair| pair.1.complete(pair.0));
        }

        self.0.nodes_heartbeat.insert(node_id, Instant::now());

        0
    }

    async fn shutdown(self, _: Context) -> i8 {
        trace!("Shutdown!!");
        // [todo] expose a callback for this event, so that users could register callbacks in init_cluster().

        0
    }

    async fn get_remote_addr(self, _: Context, key: String) -> Option<ActorId> {
        self.0.receptionist.get_actor_id(key.as_str())
    }

    async fn tell(self, _: Context, tinfo_json: String, r_msg_json: String) -> Option<String> {
        self.0
            .r_msg_type_provider
            .decode_and_dispatch(tinfo_json, r_msg_json, DispatchMethod::Tell)
            .await;
        None
    }

    async fn ask(self, _: Context, tinfo_json: String, r_msg_json: String) -> Option<String> {
        self.0
            .r_msg_type_provider
            .decode_and_dispatch(tinfo_json, r_msg_json, DispatchMethod::Ask)
            .await
    }
}
