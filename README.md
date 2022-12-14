<div align="center"><h1>factor</h2></div>

**factor** is a Rust framework to build concurrent services using the Actor Model.

>*The current version of this repository is 0.0.1-dev0 and is undergoing development for the first release client 0.1.0-rc0, which means that both the public interfaces and internal module structures may change significantly.*

**factor provides:**

* `Typed` messages.
* `Ask` and `Tell` patterns.
* Uses `futures` for asynchronous message handling.
* Concurrency using `futures::executor::ThreadPool`.
* Unbounded channels for messages (this might change.)
* `ActorPool` for running a CPU bound computation service on multiple dedicated threads.
* Async response.
* Granular locks when possible.
* Runs on stable Rust 1.60+
* `Ipc-cluster` with remote ipc-nodes.
* Task and Message Scheduling with `schedule_once`, `schedule_once_at`, and `schedule_repeat`.
* Quick `LocalPool` access.
* Simple functional message handlers `[experimental.may.get.removed]`.

**factor intends to provide (in future):**
* Actor supervision to recover from failure (restart).
* PubSub service for system topics.
* Ergonomic macros.
* `[low-priority]`
    * Status dashboard for system and actors.
    * Thread level executor control.
    * Any messages.

**Usage:**

**factor** is not published to [crates.io](https://crates.io), to use add below dependency to your `Cargo.toml` file.
```toml
# In default configuration.
[dependencies]
factor = { git = "https://github.com/fasttravel-xyz/factor", branch = "0.0.1-dev0" }

# In ipc-cluster configuration (has dependency on tokio).
[dependencies]
factor = { git = "https://github.com/fasttravel-xyz/factor", branch = "0.0.1-dev0", features=["ipc-cluster"] }


```

**Basic Example:**
```rust
use factor::prelude::*;

enum Operation {
    Add(i32),
    Sub(i32),
    Sum,
}

impl Message for Operation {
    type Result = i32;
}

#[derive(Default)]
struct OperationActor {
    sum: i32,
    add_call_count: u32,
    sub_call_count: u32,
}

impl ActorReceiver for OperationActor {
    type Context = BasicContext<Self>;
}

impl MessageHandler<Operation> for OperationActor {
    type Result = MessageResponseType<<Operation as Message>::Result>;

    fn handle(&mut self, msg: Operation, _ctx: &mut Self::Context) -> Self::Result {
        match msg {
            Operation::Add(i) => {
                self.sum += i;
                self.add_call_count += 1;
                MessageResponseType::Result(self.sum.into())
            }
            Operation::Sub(i) => {
                self.sum -= i;
                self.sub_call_count += 1;
                MessageResponseType::Result(self.sum.into())
            }
            Operation::Sum => MessageResponseType::Result(self.sum.into()),
        }
    }
}

fn main() {
    let sys = factor::init_system(Some("LocalSystem".to_string()));
    let spawn_item = ActorBuilder::create(|_| OperationActor::default(), &sys);
    let addr = sys.run_actor(spawn_item.unwrap());

    // perform additions synchronously by awaiting.
    let addr_moved = addr.clone();
    let add_ops = async move {
        for i in 1..1000 {
            addr_moved
                .ask(Operation::Add(i))
                .await
                .map_err(|e| print!("factor_ask_error: {:?} ", e))
                .err();
        }
    };

    factor::local_spawn(add_ops)
        .map_err(|e| print!("factor_local_spawn_error: {:?} ", e))
        .err();

    // perform subtractions asynchronously.
    for i in 501..1000 {
        let addr_moved = addr.clone();
        let sub_op = async move {
            addr_moved
                .ask(Operation::Sub(i))
                .await
                .map_err(|e| print!("factor_ask_error: {:?} ", e))
                .err();
        };
        factor::local_spawn(sub_op)
            .map_err(|e| print!("factor_local_spawn_error: {:?} ", e))
            .err();
    }

    // run all tasks to completion
    factor::local_run();

    // check results
    let check_op = async move {
        addr.ask(Operation::Sum)
            .await
            .map(|sum| {
                println!("Received Sum: {}. Expected Sum: {}", sum, 125250);
                assert_eq!(sum, 125250);
            })
            .map_err(|e| println!("error_in_ask_operation_sum: {:?}", e))
            .err();
    };
    
    factor::local_spawn(check_op)
        .map_err(|e| print!("factor_local_spawn_error: {:?} ", e))
        .err();

    factor::local_run();
}
```

For examples refer to the [tests] and [examples] directories:

* [tell-ask test]
* [handshake test]
* [init-stop test]
* [actor-pool test]
* [schedule-task test]
* [ipc-remote-ask example]

Benchmarks:
* [performance-discussions]
* [ring-tell example]
* [ring-ask example]
* [ipc-remote-storm example]
* [ipc-remote-ring example]


More examples will be added to the [examples] directory after v0.1.0 release.

[tests]: tests
[examples]: examples
[tell-ask test]: tests/test_ask.rs
[handshake test]: tests/test_handshake.rs
[init-stop test]: tests/test_init_stop.rs
[actor-pool test]: tests/test_actor_pool.rs
[schedule-task test]: tests/test_schedule_task.rs
[ipc-remote-ask example]: examples/example_ipc_remote_ask
[ring-tell example]: examples/example_ring_tell.rs
[ring-ask example]: examples/example_ring_ask.rs
[ipc-remote-storm example]: examples/example_ipc_remote_storm
[ipc-remote-ring example]: examples/example_ipc_remote_ring
[performance-discussions]: examples/README.md

```sh
# run all the tests:
cargo test

# run an example:
cargo run --example example_ask
```

**Common Types and Traits:**

For **sending** messages **factor** provides three type of addresses/references (and their Weak counterparts) to an actor depending on the services they expose.
```rust
/// Address/Reference of an actor that hides the actor and message type and has no generic dependence.
/// Provides the basic services related to ActorId and SystemMessage.
pub struct Addr(pub Box<dyn Address + Send + Sync>);
/// Address/Reference of an actor that hides the actor type and is only dependent on message type.
/// Provides the basic services related to a message of a specific type.
pub struct MessageAddr<M>(pub Box<dyn ActorMessageReceiver<M> + Send + Sync>)
where
    M: Message + Send + 'static,
    M::Result: Send;
/// Address/Reference of an Actor that is dependent on the actor type (struct generic) and message type (method generic).
/// Provides all the services related to an actor.
pub struct ActorAddr<R: ActorReceiver>(Arc<ActorAddrInner<R>>);
```

For **receiving** messages **factor** provides few traits that actor/message types could implement to receive messages.
```rust
/// All message types must implement this trait.
pub trait Message {
    type Result: 'static + Send;
}
/// All actor types must implement this trait.
pub trait ActorReceiver
where
    Self: Send + 'static + Sized,
{
    type Context: ActorReceiverContext<Self>;
    /// Receive and handle system events.
    fn recv_sys(&mut self, _msg: &SystemEvent, _ctx: &mut Self::Context);
    /// Handle and finalize actor creation.
    fn finalize_init(&mut self, _ctx: &mut Self::Context);
    /// Handle and finalize actor termination.
    fn finalize_stop(&mut self, _ctx: &mut Self::Context);
    ...
}
/// Trait to be implemented to handle specific messages.
pub trait MessageHandler<M>
where
    M: Message,
    Self: ActorReceiver,
{
    /// Message Response Type conforms to MessageResponse trait
    type Result: MessageResponse<M, Self>;
    /// Called for every message received by the actor
    fn handle(&mut self, msg: M, ctx: &mut Self::Context) -> Self::Result;
    ...
}
```

**factor** provides the below execution models to attach resources to Actors:
* System Executor (Default)
    ```
    Actors and tasks get spawned into the ThreadPool executor of the System. All actors and tasks share the same ThreadPool.
    ```
* ActorPool Executor
    ```
    Spawn individual Actor in a dedicated ThreadPool, rest share the System ThreadPool. Intended for Actors with CPU bound computation needs. The dedicated ThreadPool could have a single thread or multiple threads.
    ```
* IPC-Remote Cluster Nodes
    ```
    Spawn a group of related Actors in a separate worker process (ipc-remote-node).
    Intended for Actors that need dedicated and isolated resources.

    
    `factor` provides only ipc-remote actors where cluster-nodes are process-nodes, this is the only cluster model that factor intends to provide. `factor` doesn't intend to provide any other cluster model e.g. multi-machine (physical/virtual) cluster for static or dynamic horizontal-scaling, etc., as there are better and established solutions already available that `factor` could utilize.

    This execution model is targeted towards applications that need to keep adding dedicated resources in separate processes. This model is different from:
        * a cluster running with a pre-defined sets of node and all nodes sharing the application load (static)
        * a cluster adding new nodes when a load-balancing threshold is reached and it needs more resources (dynamic).

    `factor` cluster provides an execution model in which a group of related actors is launched in a child process (worker-node) of the main process (main-node). By default, worker nodes don't store or receive information about other worker nodes, as number of worker-nodes will keep on growing and usually different worker nodes will have unrelated actors. A worker node retrieves the information about another worker node only when required (e.g. to resolve the address of an actor that is running in a different worker node.)
    

    ```


## Contributing
Currently, this repository is not open for contributions or feature requests.

## License

This project is licensed under either of

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.




#
**References and Alternatives:**

* [Akka Actor Interaction Patterns](https://doc.akka.io/docs/akka/current/typed/interaction-patterns.html)
* [Actix Actor Framework for Rust](https://github.com/actix/actix)
* [Riker Actor Framework for Rust](https://github.com/riker-rs/riker)
* [Axiom Actor Model for Rust](https://github.com/rsimmonsjr/axiom)
