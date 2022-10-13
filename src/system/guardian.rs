use dashmap::DashMap;
use log::trace;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Weak};

use crate::actor::{
    builder::{ActorBuilder, ActorBuilderConfig},
    receiver::{ActorReceiver, BasicContext},
    ActorAddr, ActorUniqueKey, ActorWeakAddr, Addr,
};
use crate::system::{SystemEvent, SystemRef, SystemWeakRef};

pub(crate) type BoxedGuardianHandle = Arc<BoxedActorGuardian>;
pub(crate) type BoxedGuardianWeakHandle = Weak<BoxedActorGuardian>;
pub(crate) type GuardianAddr = ActorAddr<GuardianReceiver>;
pub(crate) type GuardianWeakAddr = ActorWeakAddr<GuardianReceiver>;

/// The guardians of all the Actors in the System.
pub(crate) struct Guardians {
    pub(crate) sys: BoxedGuardianHandle,
    pub(crate) user: BoxedGuardianHandle,
    pub(crate) remote: BoxedGuardianHandle,
}

impl Guardians {
    pub(crate) fn new(system: SystemWeakRef) -> Result<Self, GuardianCreationError> {
        let sys = Arc::new(BoxedActorGuardian(Box::new(ActorGuardian::new(
            &ActorGuardianType::System,
            system.clone(),
        )?)));

        let user = Arc::new(BoxedActorGuardian(Box::new(ActorGuardian::new(
            &ActorGuardianType::User,
            system.clone(),
        )?)));

        let remote = Arc::new(BoxedActorGuardian(Box::new(ActorGuardian::new(
            &ActorGuardianType::Remote,
            system.clone(),
        )?)));

        Ok(Guardians { sys, user, remote })
    }
}

// current tasks, maintain list on lifecycle events. resolve actor-ref when required.
// subscribe to topics and co-ordinate and share workload of children.
// Currently only one system-actor: Pub-Sub-Topic-Broadcast actor. Future sys-actor,
// insight-analytics-log actor.
enum ActorGuardian {
    System(Inner),
    User(Inner),
    Remote(Inner),
}

// this is required not to expose private inner to outside
pub(crate) struct BoxedActorGuardian(Box<ActorGuardian>);
impl BoxedActorGuardian {
    pub(crate) fn run(&self, handle: BoxedGuardianWeakHandle) {
        self.0.run(handle)
    }

    pub(crate) fn get_delegate(&self) -> Option<ActorAddr<GuardianReceiver>> {
        self.0.get_delegate()
    }

    #[allow(dead_code)]
    fn system(&self) -> Option<SystemRef> {
        self.0.system()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ActorGuardianType {
    /// Guardian(dummy) for System, User, and Remote guardians
    Root,
    /// Guardian for system services
    System,
    /// Guardian for user actors
    User,
    /// Guardian for handles of remote actors
    Remote,
}

impl ActorGuardian {
    pub(crate) fn new(
        g_type: &ActorGuardianType,
        system: SystemWeakRef,
    ) -> Result<Self, GuardianCreationError> {
        let mut result = Err(GuardianCreationError);

        // guardian creation only during system::new
        if let None = system.upgrade() {
            let delegate = RwLock::new(None);
            let addresses = DashMap::new();
            let inner = Inner {
                system,
                delegate,
                addresses,
            };

            match g_type {
                ActorGuardianType::System => result = Ok(ActorGuardian::System(inner)),
                ActorGuardianType::User => result = Ok(ActorGuardian::User(inner)),
                ActorGuardianType::Remote => result = Ok(ActorGuardian::Remote(inner)),
                _ => {}
            }
        }

        result
    }

    pub(crate) fn run(&self, handle: BoxedGuardianWeakHandle) {
        if let Some(system) = self.inner().system.upgrade() {
            let mut _g_type = ActorGuardianType::Root;

            match self {
                ActorGuardian::System(_inner) => _g_type = ActorGuardianType::System,
                ActorGuardian::User(_inner) => _g_type = ActorGuardianType::User,
                ActorGuardian::Remote(_inner) => _g_type = ActorGuardianType::Remote,
            }

            let factory = move |_| GuardianReceiver(handle.clone());
            match ActorBuilder::create_actor(
                factory,
                &_g_type,
                &system,
                ActorBuilderConfig::default(),
            ) {
                Ok(spawn_item) => {
                    let delegate = system.run_actor(spawn_item);
                    let mut data = self.inner().delegate.write();
                    *data = Some(delegate);
                }
                Err(e) => {
                    trace!("ActorGuardian_run_error {:?}", e);
                }
            }
        }
    }

    pub(crate) fn get_delegate(&self) -> Option<ActorAddr<GuardianReceiver>> {
        if let Some(delegate) = self.inner().delegate.read().as_ref() {
            return Some(delegate.clone());
        }

        None
    }

    fn system(&self) -> Option<SystemRef> {
        self.inner().system.upgrade()
    }

    // method to get the inner as currently all variants use the same Inner
    fn inner(&self) -> &Inner {
        match self {
            ActorGuardian::System(inner)
            | ActorGuardian::User(inner)
            | ActorGuardian::Remote(inner) => inner,
        }
    }
}

// In future if we decide that all actors could have children, we won't need this
// crate internal trait. But this is low priority and should be done along with
// the specialized-executor with thread-level control. Both deal with resource
// allocation/management at a granular level.
pub(crate) trait Supervisor {
    fn get_actor<R: ActorReceiver>(&self, key: &ActorUniqueKey) -> Option<ActorAddr<R>>;
    fn add_actor<R: ActorReceiver>(&self, child: ActorAddr<R>);
    fn remove_actor<R: ActorReceiver>(&self, child: ActorAddr<R>);
    fn get_address(&self, key: &ActorUniqueKey) -> Option<Addr>;
    fn add_address(&self, child: Addr);
    fn remove_address(&self, child: &Addr);
}

struct Inner {
    system: SystemWeakRef,
    delegate: RwLock<Option<ActorAddr<GuardianReceiver>>>,
    addresses: DashMap<ActorUniqueKey, Addr>,
}

pub(crate) struct GuardianReceiver(BoxedGuardianWeakHandle);

impl ActorReceiver for GuardianReceiver {
    type Context = BasicContext<Self>;

    fn recv_sys(&mut self, msg: &SystemEvent, _ctx: &mut Self::Context) {
        match msg {
            SystemEvent::ActorTerminated(address) => {
                if let Some(g) = self.0.upgrade() {
                    g.remove_address(address);
                }
            }
            _ => {}
        }
    }
}

impl Supervisor for ActorGuardian {
    fn get_actor<R: ActorReceiver>(&self, key: &ActorUniqueKey) -> Option<ActorAddr<R>> {
        self.get_address(key)
            .and_then(|addr| addr.try_get_actoraddr().ok())
    }

    fn add_actor<R: ActorReceiver>(&self, actor: ActorAddr<R>) {
        self.add_address(actor.get_address());
    }

    fn remove_actor<R: ActorReceiver>(&self, actor: ActorAddr<R>) {
        let id = actor.get_id();
        self.inner().addresses.remove(&id.unique_key());
    }

    fn get_address(&self, key: &ActorUniqueKey) -> Option<Addr> {
        self.inner()
            .addresses
            .get(key)
            .map(|pair| pair.value().clone())
    }

    fn add_address(&self, address: Addr) {
        let id = address.get_id();
        self.inner()
            .addresses
            .insert(id.unique_key().clone(), address);
    }

    fn remove_address(&self, address: &Addr) {
        let id = address.get_id();
        self.inner().addresses.remove(&id.unique_key());
    }
}

impl Supervisor for BoxedActorGuardian {
    fn get_actor<R: ActorReceiver>(&self, key: &ActorUniqueKey) -> Option<ActorAddr<R>> {
        self.0.get_actor(key)
    }

    fn add_actor<R: ActorReceiver>(&self, actor: ActorAddr<R>) {
        self.0.as_ref().add_actor(actor);
    }

    fn remove_actor<R: ActorReceiver>(&self, actor: ActorAddr<R>) {
        self.0.as_ref().remove_actor(actor);
    }

    fn get_address(&self, key: &ActorUniqueKey) -> Option<Addr> {
        self.0.get_address(key)
    }

    fn add_address(&self, address: Addr) {
        self.0.as_ref().add_address(address);
    }

    fn remove_address(&self, address: &Addr) {
        self.0.as_ref().remove_address(address);
    }
}

#[derive(Debug)]
pub(crate) struct GuardianCreationError;
