//! factor is a Rust library to build concurrent services using the Actor Model.
//!
//! factor is designed to support remote actors if required in future, so all
//! interactions with the actors is only through messages. Even Lifecycle commands
//! should also be sent as SystemMessage::SystemCommand, there are no direct
//! methods like stop() etc. So all communications are async by default, in future
//! we might add in-process sync-actors.
//!
//! Actors are worker units that communicate exclusively using messages.
//! factor uses futures::executor::ThreadPool for concurrency, so actors share
//! threads and tasks of same actor may be executing in different threads.
//!
//! ## factor provides:
//! - Typed messages.
//! - Ask and Tell patterns.
//! - Uses futures for asynchronous message handling.
//! - Concurrency using futures::executor::ThreadPool.
//! - Quick LocalPool access to wait for multiple quick-tasks completion.
//! - Unbounded channels for messages (this might change.)
//! - Simple functional message handlers.
//! - Async response.
//! - Granular locks when possible.
//! - Runs on stable Rust 1.62+
//!

// ============================================
// REFERENCES
// ============================================
// Akka Actor Interaction Patterns: https://doc.akka.io/docs/akka/current/typed/interaction-patterns.html
// Actix Actor Framework for Rust: https://github.com/actix/actix
// Riker Actor Framework for Rust: https://github.com/riker-rs/riker
// Axiom Actor Model for Rust: https://github.com/rsimmonsjr/axiom
// ============================================

// modules of the crate
mod actor;
mod common;
mod message;
mod system;

// public interface of the crate. Use the prelude for more interfaces.
pub use actor::builder;
pub use actor::receiver::{ActorReceiver, BasicContext};
pub use actor::ActorRef;
pub use message::{
    handler::{MessageHandler, MessageResponseType},
    Message,
};
pub use system::SystemRef;

pub mod prelude {
    //! The 'factor' prelude.
    //!
    //!
    //! ```
    //! # #![allow(unused_imports)]
    //! use factor::prelude::*;
    //! ```

    #[doc(hidden)]
    pub use crate::{
        actor::{
            builder,
            receiver::{ActorReceiver, ActorReceiverContext, BasicContext},
            ActorRef, ActorWeakRef,
        },
        message::{
            handler::{MessageHandler, MessageResponseType, ResponseResult},
            Message,
        },
        system::{ActorSystem, SystemCommand, SystemMessage, SystemRef},
    };
}

////////////////////////////////////////////////////////////////////////////////

thread_local! {
    static FACTOR: std::cell::RefCell<FactorLocalResource> = std::cell::RefCell::new(FactorLocalResource::new());
}

/// Thread local resources
struct FactorLocalResource {
    pool: Box<futures::executor::LocalPool>,
}
impl FactorLocalResource {
    fn new() -> Self {
        Self {
            pool: Box::new(futures::executor::LocalPool::new()),
        }
    }
}

/// Initialize the system per process
pub fn init_system(name: Option<String>) -> SystemRef {
    // add an atomic bool check
    system::ActorSystem::create_system(name)
}

/// Spawn tasks into the local-pool
pub fn local_spawn<Fut>(future: Fut) -> Result<(), futures::task::SpawnError>
where
    Fut: 'static + futures::Future<Output = ()>,
{
    FACTOR.with(|f| futures::task::LocalSpawnExt::spawn_local(&f.borrow().pool.spawner(), future))
}

/// Run all tasks in the local-pool to completion.
pub fn local_run() {
    FACTOR.with(|f| f.borrow_mut().pool.as_mut().run())
}
