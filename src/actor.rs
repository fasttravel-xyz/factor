pub mod builder;
pub(crate) mod core;
pub(crate) mod mailbox;
pub mod receiver;

use futures::channel::oneshot;
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::sync::{Arc, Weak};

use crate::actor::receiver::ActorReceiver;
use crate::common;
use crate::message::{
    handler::MessageHandler, Message, MessageSend, MessageSendError, MessageSender,
    SystemMessageSend,
};
use crate::system::{guardian::ActorGuardianType, SystemId, SystemMessage};

/// Identification details of an Actor. The information in the ActorId
/// is sufficient to resolve into an ActorRef/Address.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ActorId {
    uuid: common::EntityId,
    path: ActorPath,
    name: Option<String>,
}

impl ActorId {
    /// Generate unique Id.
    pub fn generate(sys_id: SystemId, guardian: ActorGuardianType) -> Result<ActorId, ()> {
        let mut result = Err(());

        let path = ActorPath { sys_id, guardian };
        if let Ok(uuid) = common::generate_entity_id() {
            result = Ok(ActorId {
                uuid,
                path,
                name: None,
            })
        }

        result
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
            self.path.sys_id.host_id, self.path.sys_id.uuid, guardian_str, self.uuid
        );
        path_str
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ActorPath {
    /// the system id
    sys_id: SystemId,
    /// guardian type (can be system or user, can never be remote as remote only holds remote-handles not local-actors)
    guardian: ActorGuardianType,
}

/// Runtime Error while resolving an Address to an ActorRef<R>
#[derive(Debug)]
pub struct DowncastAddressError;

/// Basic trait for exposing interface to get the ActorId and send SystemMessage.
pub trait Addr: common::AsAny {
    fn get_actor_id(&self) -> &ActorId;
    fn tell_sys_msg(&self, msg: SystemMessage) -> Result<(), MessageSendError>;
    // fn try_tell_any_msg(&self, msg: AnyMessage); // todo
}

/// Address of an actor that hides the type.
pub struct Address(pub Box<dyn Addr + Send + Sync>);

impl Address {
    /// Trye to get the ActorRef from the Address
    pub fn try_get_ref<R: ActorReceiver>(&self) -> Result<ActorRef<R>, DowncastAddressError> {
        let mut result = Err(DowncastAddressError);
        if let Some(actor_ref) = self.0.as_any().downcast_ref::<ActorRef<R>>() {
            result = Ok(actor_ref.clone());
        }

        result
    }
}

/// [SERIALIZE]: When serializing we serialize only the identifier and not the runtime
/// resolved comm channels and resources. As ActorRef will be passed around
/// between nodes of the cluster inside messages, resolved resources have
/// to be re-resolved for each node during deserialization.
impl<R: ActorReceiver> Serialize for ActorRef<R> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let id = self.get_id();
        id.serialize(serializer)
    }
}

/// [DESERIALIZE]: When deserializing the ActorRef we only deserialize the Id.
/// The Id resolution is a separate step so that resolution could be
/// handled in a context dependent manner.
impl<'de, R: ActorReceiver> Deserialize<'de> for ActorRef<R> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let id_actor = ActorId::deserialize(deserializer)?;
        Ok(ActorRef {
            inner: Arc::new(ActorRefInner {
                id: Some(id_actor),
                sender: None,
            }),
        })
    }
}

/// The reference/address to an Actor. This is the main handle to communicate
///  with the actor.
pub struct ActorRef<R: ActorReceiver> {
    inner: Arc<ActorRefInner<R>>,
}

impl<R: ActorReceiver> ActorRef<R> {
    /// Create an ActorRef with the provided inner.
    pub(in crate::actor) fn new(inner: Arc<ActorRefInner<R>>) -> Self {
        ActorRef { inner }
    }

    /// Get the ActorId of the actor.
    pub fn get_id(&self) -> &ActorId {
        &self.inner.id.as_ref().unwrap()
    }

    /// Get the Address of the actor.
    pub fn get_address(&self) -> Address {
        Address(Box::new(self.clone()))
    }

    /// Get the weak actor refernce of the actor.
    pub fn downgrade(&self) -> ActorWeakRef<R> {
        ActorWeakRef {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Send a message to the actor.
    pub fn tell<M>(&self, msg: M) -> Result<(), MessageSendError>
    where
        M: Message + Send + 'static,
        M::Result: Send,
        R: MessageHandler<M>,
    {
        if let Some(sender) = &self.inner.sender {
            sender.tell(msg)
        } else {
            Err(MessageSendError)
        }
    }

    /// Send a message and receive a response from the actor.
    pub fn ask<M>(&self, msg: M) -> Result<oneshot::Receiver<M::Result>, MessageSendError>
    where
        M: Message + Send + 'static,
        M::Result: Send,
        R: MessageHandler<M>,
    {
        if let Some(sender) = &self.inner.sender {
            sender.ask(msg)
        } else {
            Err(MessageSendError)
        }
    }

    /// Send a system message to the actor.
    pub fn tell_sys(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        if let Some(sender) = &self.inner.sender {
            sender.tell_sys(msg)
        } else {
            Err(MessageSendError)
        }
    }
}

impl<R: ActorReceiver> Addr for ActorRef<R> {
    fn get_actor_id(&self) -> &ActorId {
        self.get_id()
    }

    fn tell_sys_msg(&self, msg: SystemMessage) -> Result<(), MessageSendError> {
        self.tell_sys(msg)
    }
}

impl<R: ActorReceiver> Clone for ActorRef<R> {
    fn clone(&self) -> Self {
        ActorRef {
            inner: self.inner.clone(),
        }
    }
}

/// Weak reference/address of an actor. Upgradable to ActorRef.
/// Use to avoid circular dependencies.
pub struct ActorWeakRef<R: ActorReceiver> {
    pub(in crate::actor) inner: Weak<ActorRefInner<R>>, // add new()
}

impl<R: ActorReceiver> ActorWeakRef<R> {
    pub fn upgrade(&self) -> Option<ActorRef<R>> {
        match self.inner.upgrade() {
            Some(d) => Some(ActorRef { inner: d.clone() }),
            None => None,
        }
    }
}

impl<R: ActorReceiver> Clone for ActorWeakRef<R> {
    fn clone(&self) -> Self {
        ActorWeakRef {
            inner: self.inner.clone(),
        }
    }
}

/// Inner data of an ActorRef.
/// Cloning of ActorRef is cheap as cloning the ActorRef doesn't clone the
/// Inner data, the Arc to the Inner data gets cloned.
pub(in crate::actor) struct ActorRefInner<R: ActorReceiver> {
    pub(in crate::actor) id: Option<ActorId>,
    pub(in crate::actor) sender: Option<MessageSender<R>>,
}

#[cfg(feature = "debug-log")]
impl<R: ActorReceiver> Drop for Inner<R> {
    fn drop(&mut self) {
        println!("==== DROPPING ========= {}", "ActorRefInner");
    }
}
