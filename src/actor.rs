pub mod builder;
pub(crate) mod core;
pub(crate) mod mailbox;
pub mod receiver;

use async_trait::async_trait;
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::sync::{Arc, Weak};

use crate::actor::receiver::ActorReceiver;
use crate::common;
use crate::message::{
    handler::MessageHandler, Message, MessageSend, MessageSendError, MessageSender,
    SystemMessageSend,
};
use crate::system::{self, guardian::ActorGuardianType, NodeId, SystemId, SystemMessage};

// =============================================================================
// ACTOR ID, PATH, AND ADDRESSES.
// [todo]: Weak Addresses
// [todo]: Rename addresses to be consistent, i.e Addr, MessageAddr, ActorAddr.
// =============================================================================

/// Identification details of an Actor. The information in the ActorId
/// is sufficient to resolve into an Actor Address/Reference.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ActorId {
    uuid: common::EntityId,
    path: ActorPath,
    tag: Option<String>,
    discovery: DiscoveryKey,
}

/// Path of an actor
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ActorPath {
    /// the system id
    sys_id: SystemId,
    /// guardian type (can be system or user, can never be remote as remote only holds remote-handles not local-actors)
    guardian: ActorGuardianType,
}

/// Address/Reference of an actor that hides the actor and message type and has no generic dependence.
/// Provides the basic services related to ActorId and SystemMessage.
pub struct Addr(pub Box<dyn Address + Send + Sync>);

/// Address/Reference of an actor that hides the actor type and is only dependent on message type.
/// Provides the basic services related to a message of a specific type.
pub struct MessageAddr<M>(pub Box<dyn ActorMessageReceiver<M> + Send + Sync>)
where
    M: Message + Send + 'static,
    M::Result: Send;
// [todo][low-priority]: Make MessageAddr Serializable, by storing the typeid of R.

/// Address/Reference to an Actor that is dependent on the actor type (struct generic) and message type (method generic).
/// Provides all the services related to an actor.
pub struct ActorAddr<R: ActorReceiver>(Arc<ActorAddrInner<R>>);

/// Weak Address/Reference of an actor. Upgradable to ActorAddr.
/// Use to avoid circular dependencies.
pub struct ActorWeakAddr<R: ActorReceiver>(Weak<ActorAddrInner<R>>);
// =============================================================================

/// Discovery Tag for discovery by other nodes.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum DiscoveryKey {
    None,
    Uuid,
    Tag,
}

/// Unique key to identify the actor in the cluster. With current implementation
/// uuid is enough to identify an actor, we don't need the entire path. But if we need
/// we could just change here.
pub type ActorUniqueKey = u128;

impl ActorId {
    /// Generate unique Id.
    pub fn generate(
        sys_id: SystemId,
        guardian: ActorGuardianType,
        tag: Option<String>,
        discovery: DiscoveryKey,
    ) -> Result<ActorId, ()> {
        let mut result = Err(());

        let path = ActorPath { sys_id, guardian };
        if let Ok(uuid) = common::generate_entity_id() {
            result = Ok(ActorId {
                uuid,
                path,
                tag,
                discovery,
            })
        }

        result
    }

    /// Get unique key in cluster
    pub fn unique_key(&self) -> &ActorUniqueKey {
        &self.uuid
    }

    /// Get the guardian of the actor in it's host node.
    pub fn guardian(&self) -> &ActorGuardianType {
        &self.path.guardian
    }

    /// Checks if target and self NodeIds are same or not.
    pub fn check_node_id(&self, node_id: NodeId) -> bool {
        if self.path.sys_id.node_id == node_id {
            return true;
        }

        false
    }

    /// Get the Node Id
    pub fn node_id(&self) -> NodeId {
        self.path.sys_id.node_id
    }

    /// Get the discovery key option
    #[allow(dead_code)] // used when feature ipc-custer is active.
    pub(crate) fn discovery(&self) -> &DiscoveryKey {
        &self.discovery
    }

    /// Get the discovery key option
    #[allow(dead_code)] // used when feature ipc-custer is active.
    pub(crate) fn tag(&self) -> Option<String> {
        self.tag.clone()
    }

    /// Get unique key in cluster
    pub fn uuid_str(&self) -> String {
        self.uuid.to_string()
    }

    /// Checks if target and self SystemIds are same or not.
    pub fn check_system_id(&self, sys_id: &SystemId) -> bool {
        if self.path.sys_id.node_id == sys_id.node_id && self.path.sys_id.uuid == sys_id.uuid {
            return true;
        }

        false
    }

    /// Get the path string from the Id details.
    pub fn get_path_str(&self) -> String {
        let guardian_str = match self.path.guardian {
            ActorGuardianType::System => "sys",
            ActorGuardianType::User => "user",
            _ => "error",
        };

        let path_str = format!(
            "{}/{}/{}/{}",
            self.path.sys_id.node_id, self.path.sys_id.uuid, guardian_str, self.uuid
        );
        path_str
    }
}

/// Runtime Error while resolving an Address to an ActorAddr<R>
#[derive(Debug)]
pub struct DowncastAddressError;

/// Basic trait for exposing interface to get the ActorId and send SystemMessage.
pub trait Address: common::AsAny {
    fn boxed(&self) -> Box<dyn Address + Sync + Send>;
    fn get_actor_id(&self) -> &ActorId;
    fn tell_sys_msg(&self, msg: SystemMessage) -> Result<(), MessageSendError>;
    // fn try_tell_any_msg(&self, msg: AnyMessage); // [todo][low-priority]
}

/// Basic trait for exposing interface to send a message of specific type.
#[async_trait]
pub trait ActorMessageReceiver<M: Message + Send + 'static> {
    fn boxed(&self) -> Box<dyn ActorMessageReceiver<M> + Send + Sync>;
    fn tell_msg(&self, msg: M) -> Result<(), MessageSendError>;
    async fn ask_msg(&self, msg: M) -> Result<M::Result, MessageSendError>;
}

impl<R: ActorReceiver> Address for ActorAddr<R> {
    fn boxed(&self) -> Box<dyn Address + Sync + Send> {
        Box::new(self.clone())
    }

    fn get_actor_id(&self) -> &ActorId {
        self.get_id()
    }

    fn tell_sys_msg(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        self.tell_sys(msg)
    }
}

impl Addr {
    /// Try to get the ActorAddr from the Address
    pub fn try_get_actoraddr<R: ActorReceiver>(
        &self,
    ) -> Result<ActorAddr<R>, DowncastAddressError> {
        let mut result = Err(DowncastAddressError);
        if let Some(actor_ref) = self.0.as_any().downcast_ref::<ActorAddr<R>>() {
            result = Ok(actor_ref.clone());
        }

        result
    }

    pub fn get_id(&self) -> &ActorId {
        self.0.get_actor_id()
    }

    pub fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        self.0.tell_sys_msg(msg)
    }
}

impl std::fmt::Debug for Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", "Addr")
    }
}

impl Clone for Addr {
    fn clone(&self) -> Self {
        Self(self.0.boxed())
    }
}

#[async_trait]
impl<M: Message + Send, R: ActorReceiver> ActorMessageReceiver<M> for ActorAddr<R>
where
    R: ActorReceiver + MessageHandler<M> + 'static,
    M: Message + Send + 'static,
    M::Result: 'static,
{
    fn boxed(&self) -> Box<dyn ActorMessageReceiver<M> + Send + Sync> {
        Box::new(self.clone())
    }

    fn tell_msg(&self, msg: M) -> Result<(), MessageSendError> {
        self.tell(msg)
    }

    async fn ask_msg(&self, msg: M) -> Result<M::Result, MessageSendError> {
        self.ask(msg).await
    }
}

impl<M> MessageAddr<M>
where
    M: Message + Send + 'static,
    M::Result: Send,
{
    pub fn tell(&self, msg: M) -> Result<(), MessageSendError> {
        self.0.tell_msg(msg)
    }

    pub async fn ask(&self, msg: M) -> Result<M::Result, MessageSendError> {
        self.0.ask_msg(msg).await
    }
}

impl<M> std::fmt::Debug for MessageAddr<M>
where
    M: Message + Send + 'static,
    M::Result: Send,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", "MessageAddr")
    }
}

impl<M> Clone for MessageAddr<M>
where
    M: Message + Send + 'static,
    M::Result: Send,
{
    fn clone(&self) -> Self {
        Self(self.0.boxed())
    }
}

impl<R, M> From<&ActorAddr<R>> for MessageAddr<M>
where
    R: ActorReceiver + MessageHandler<M> + 'static,
    M: Message + Send + 'static,
    M::Result: 'static,
{
    fn from(addr: &ActorAddr<R>) -> Self {
        MessageAddr(<ActorAddr<R> as ActorMessageReceiver<M>>::boxed(&addr))
    }
}

/// [SERIALIZE]: When serializing we serialize only the identifier and not the runtime
/// resolved comm channels and resources. As ActorAddr will be passed around
/// between nodes of the cluster inside messages, resolved resources have
/// to be re-resolved for each node during deserialization.
impl<R: ActorReceiver> Serialize for ActorAddr<R> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let id = self.get_id();

        println!("In ActorAddr Serialize: {:#?}", id);

        id.serialize(serializer)
    }
}

impl<R: ActorReceiver> Serialize for ActorWeakAddr<R> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        println!("In ActorWeakAddr Serialize");

        if let Some(addr) = self.upgrade() {
            let data = Some(addr);
            data.serialize(serializer)
        } else {
            Option::<ActorAddr<R>>::None.serialize(serializer)
        }
    }
}

/// [DESERIALIZE]: When deserializing the ActorAddr we only deserialize the Id.
/// The Id resolution is a separate step so that resolution could be
/// handled in a context dependent manner.
impl<'de, R: ActorReceiver> Deserialize<'de> for ActorAddr<R> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let id_actor = ActorId::deserialize(deserializer)?;

        println!("In ActorAddr Deserialize: {:#?}", id_actor);

        Ok(system::resolve_actor_addr_in_current_node(id_actor))
    }
}

impl<'de, R: ActorReceiver> Deserialize<'de> for ActorWeakAddr<R> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        println!("In ActorWeakAddr Deserialize");

        if let Some(data) = Option::<ActorAddr<R>>::deserialize(deserializer)? {
            Ok(data.downgrade())
        } else {
            // we don't return error, we return empty weak addr that will always return None.
            Ok(ActorWeakAddr::new(Weak::new()))
        }
    }
}

impl<R: ActorReceiver> ActorAddr<R> {
    /// Create an ActorAddr with the provided inner.
    pub(in crate::actor) fn new(inner: Arc<ActorAddrInner<R>>) -> Self {
        ActorAddr(inner)
    }

    /// Get the ActorId of the actor.
    pub fn get_id(&self) -> &ActorId {
        &self.0.id.as_ref().unwrap()
    }

    /// Get the Address of the actor.
    pub fn get_address(&self) -> Addr {
        Addr(Box::new(self.clone()))
    }

    /// get the MessageAddr of the actor for a message type
    pub fn message_addr<M>(&self) -> MessageAddr<M>
    where
        R: ActorReceiver + MessageHandler<M> + 'static,
        M: Message + Send + 'static,
        M::Result: 'static,
    {
        self.into()
    }

    /// Get the weak actor refernce of the actor.
    pub fn downgrade(&self) -> ActorWeakAddr<R> {
        ActorWeakAddr(Arc::downgrade(&self.0))
    }

    /// Send a message to the actor.
    pub fn tell<M>(&self, msg: M) -> Result<(), MessageSendError>
    where
        R: ActorReceiver + MessageHandler<M> + 'static,
        M: Message + Send + 'static,
        M::Result: 'static,
    {
        if let Some(sender) = &self.0.sender {
            sender.tell(msg)
        } else {
            Err(MessageSendError::ErrSenderNotResolved)
        }
    }

    /// Send a message and receive a response from the actor.
    pub async fn ask<M>(&self, msg: M) -> Result<M::Result, MessageSendError>
    where
        R: ActorReceiver + MessageHandler<M> + 'static,
        M: Message + Send + 'static,
        M::Result: 'static,
    {
        if let Some(sender) = &self.0.sender {
            sender.ask(msg).await
        } else {
            Err(MessageSendError::ErrSenderNotResolved)
        }
    }

    /// Send a system message to the actor.
    pub fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        if let Some(sender) = &self.0.sender {
            sender.tell_sys(msg)
        } else {
            Err(MessageSendError::ErrSenderNotResolved)
        }
    }
}

impl<R: ActorReceiver> Clone for ActorAddr<R> {
    fn clone(&self) -> Self {
        ActorAddr(self.0.clone())
    }
}

impl<R: ActorReceiver> ActorWeakAddr<R> {
    pub(in crate::actor) fn new(w: Weak<ActorAddrInner<R>>) -> Self {
        Self(w)
    }

    pub fn upgrade(&self) -> Option<ActorAddr<R>> {
        match self.0.upgrade() {
            Some(d) => Some(ActorAddr(d.clone())),
            None => None,
        }
    }
}

impl<R: ActorReceiver> Clone for ActorWeakAddr<R> {
    fn clone(&self) -> Self {
        ActorWeakAddr(self.0.clone())
    }
}

/// Inner data of an ActorAddr.
/// Cloning of ActorAddr is cheap as cloning the ActorAddr doesn't clone the
/// Inner data, the Arc to the Inner data gets cloned.
pub(in crate::actor) struct ActorAddrInner<R: ActorReceiver> {
    pub(in crate::actor) id: Option<ActorId>,
    pub(in crate::actor) sender: Option<MessageSender<R>>,
}

#[cfg(feature = "debug-log")]
impl<R: ActorReceiver> Drop for Inner<R> {
    fn drop(&mut self) {
        println!("==== DROPPING ========= {}", "ActorAddrInner");
    }
}
