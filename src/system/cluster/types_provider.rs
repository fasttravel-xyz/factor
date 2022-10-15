use async_trait::async_trait;
use log::error;
use serde::{Deserialize, Serialize};
use serde_json;
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;

use crate::actor::ActorId;
use crate::system::resolve_actor_addr_in_current_node;
use crate::{ActorAddr, ActorReceiver, MessageCluster, MessageClusterHandler};

pub(super) enum DispatchMethod {
    Tell,
    Ask,
    #[allow(dead_code)]
    TellSys,
}

#[derive(Serialize, Deserialize)]
pub(super) struct RemoteMsg<M>
where
    M: MessageCluster + Send + 'static,
{
    pub(super) act_id: ActorId,

    #[serde(bound(deserialize = "M: MessageCluster"))]
    pub(super) msg: M,
}

struct RemoteMessageDecoder {
    r_msg_json: String,
}

impl RemoteMessageDecoder {
    fn new(r_msg_json: String) -> Self {
        Self { r_msg_json }
    }

    async fn decode_and_dispatch<R, M>(self, method: DispatchMethod) -> Option<M::Result>
    where
        R: ActorReceiver + MessageClusterHandler<M> + 'static,
        M: MessageCluster + Send + 'static,
        M::Result: 'static,
    {
        match serde_json::from_str::<RemoteMsg<M>>(&self.r_msg_json) {
            Ok(remote_msg) => {
                let addr = resolve_actor_addr_in_current_node::<R>(remote_msg.act_id);

                match method {
                    DispatchMethod::Tell => {
                        let _ = addr.tell_addr(remote_msg.msg);
                    }
                    DispatchMethod::Ask => {
                        return addr.ask_addr(remote_msg.msg).await.ok();
                    }
                    _ => {}
                }
            }
            Err(e) => error!("serde_addr_failed: {:?}", e),
        }

        None
    }
}

#[derive(Eq, Hash, PartialEq, Clone, Serialize, Deserialize)]
struct TypeKey {
    aid: u16,
    mid: u16,
}

trait RemoteMessageDispatcherClone {
    fn boxed(&self) -> Box<dyn RemoteMessageDispatcher + Send + Sync>;
}

impl Clone for Box<dyn RemoteMessageDispatcher + Send + Sync> {
    fn clone(&self) -> Box<dyn RemoteMessageDispatcher + Send + Sync> {
        self.boxed()
    }
}

impl<T> RemoteMessageDispatcherClone for T
where
    T: 'static + RemoteMessageDispatcher + Clone + Send + Sync,
{
    fn boxed(&self) -> Box<dyn RemoteMessageDispatcher + Send + Sync> {
        Box::new(self.clone())
    }
}

#[async_trait]
trait RemoteMessageDispatcher: RemoteMessageDispatcherClone {
    async fn decode_and_dispatch(
        &self,
        decoder: RemoteMessageDecoder,
        method: DispatchMethod,
    ) -> Option<String>;
}

#[derive(Clone)]
struct RemoteMessageTypeDecoder<R, M> {
    phantomdata_r: std::marker::PhantomData<R>,
    phantomdata_m: std::marker::PhantomData<M>,
}

impl<R, M> RemoteMessageTypeDecoder<R, M>
where
    R: ActorReceiver + MessageClusterHandler<M> + 'static + Clone + Send + Sync,
    M: MessageCluster + Send + 'static + Clone + Sync,
    M::Result: 'static,
{
    fn new() -> Self {
        Self {
            phantomdata_m: PhantomData::default(),
            phantomdata_r: PhantomData::default(),
        }
    }
}

#[async_trait]
impl<R, M> RemoteMessageDispatcher for RemoteMessageTypeDecoder<R, M>
where
    R: ActorReceiver + MessageClusterHandler<M> + 'static + Clone + Send + Sync,
    M: MessageCluster + Send + 'static + Clone + Sync,
    M::Result: 'static,
{
    async fn decode_and_dispatch(
        &self,
        decoder: RemoteMessageDecoder,
        method: DispatchMethod,
    ) -> Option<String> {
        decoder
            .decode_and_dispatch::<R, M>(method)
            .await
            .and_then(|result| {
                serde_json::to_string::<M::Result>(&result)
                    .map_err(|e| error!("remote_message_decoder_M_Result_failed: {}", e))
                    .ok()
            })
    }
}

#[derive(Clone)]
pub struct RemoteMessageTypeProvider {
    aid_counter: u16,
    mid_counter: u16,
    aid_map: HashMap<TypeId, u16>,
    mid_map: HashMap<TypeId, u16>,
    dispatchers: HashMap<TypeKey, Box<dyn RemoteMessageDispatcher + Send + Sync>>,
}

impl RemoteMessageTypeProvider {
    pub fn new() -> Self {
        Self {
            aid_counter: 0,
            mid_counter: 0,
            aid_map: HashMap::new(),
            mid_map: HashMap::new(),
            dispatchers: HashMap::new(),
        }
    }

    pub fn register<R, M>(&mut self)
    where
        R: ActorReceiver + MessageClusterHandler<M> + 'static + Clone + Sync + Send,
        M: MessageCluster + Send + 'static + Clone + Sync + Send,
        M::Result: 'static,
    {
        let a_t = TypeId::of::<R>();
        let m_t = TypeId::of::<M>();
        let mut aid = self.aid_map.get(&a_t);
        let mut mid = self.mid_map.get(&m_t);

        if aid.is_none() {
            self.aid_counter += 1;
            self.aid_map.insert(a_t, self.aid_counter);
            aid = Some(&self.aid_counter);
        }

        if mid.is_none() {
            self.mid_counter += 1;
            self.mid_map.insert(m_t, self.mid_counter);
            mid = Some(&self.mid_counter);
        }

        let k = TypeKey {
            aid: aid.unwrap().clone(),
            mid: mid.unwrap().clone(),
        };

        let t_decoder = RemoteMessageTypeDecoder::<R, M>::new();

        self.dispatchers.insert(k, Box::new(t_decoder));
    }

    pub(super) fn encode<R, M>(&self, addr: ActorAddr<R>, msg: M) -> Option<(String, String)>
    where
        R: ActorReceiver + MessageClusterHandler<M> + 'static,
        M: MessageCluster + Send + 'static,
    {
        let a_t = TypeId::of::<R>();
        let m_t = TypeId::of::<M>();

        let aid = self.aid_map.get(&a_t);
        let mid = self.mid_map.get(&m_t);

        if aid.is_some() && mid.is_some() {
            let k = TypeKey {
                aid: aid.unwrap().clone(),
                mid: mid.unwrap().clone(),
            };

            let act_id = addr.get_id().clone();
            let r_msg = RemoteMsg::<M> { act_id, msg };

            let tinfo_json = serde_json::to_string(&k);
            let r_msg_json = serde_json::to_string(&r_msg);

            if tinfo_json.is_ok() && r_msg_json.is_ok() {
                return Some((tinfo_json.unwrap(), r_msg_json.unwrap()));
            }
        }

        None
    }

    pub(super) async fn decode_and_dispatch(
        &self,
        tinfo_json: String,
        r_msg_json: String,
        method: DispatchMethod,
    ) -> Option<String> {
        let decoder = RemoteMessageDecoder::new(r_msg_json);

        let t_info_key = serde_json::from_str::<TypeKey>(&tinfo_json).unwrap();

        if let Some(dispatcher) = self.dispatchers.get(&t_info_key) {
            return dispatcher.decode_and_dispatch(decoder, method).await;
        }

        None
    }
}
