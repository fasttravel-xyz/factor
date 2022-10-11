use dashmap::DashMap;

use crate::actor::{ActorId, Addr};

/// Minimal implementation of the Receptionist. As most requests will be one time
/// we won't need buffering requests, or else we could wrap this in an actor.
pub(super) struct Receptionist {
    // [todo] store the WeakAddr or ActorId
    addr_book: DashMap<String, Addr>,
}

impl Receptionist {
    pub(super) fn new() -> Self {
        Self {
            addr_book: DashMap::new(),
        }
    }

    pub(super) fn get_actor_id(&self, key: &str) -> Option<ActorId> {
        self.addr_book
            .get(key)
            .and_then(|pair| Some(pair.value().get_id().clone()))
    }

    pub(super) fn set_address(&self, key: &str, addr: Addr) {
        self.addr_book.insert(key.to_string(), addr);
    }
}
