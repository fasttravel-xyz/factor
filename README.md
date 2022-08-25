<div align="center">
  <h1>factor</h2>
</div>

**factor** is a Rust library to build concurrent services using the Actor Model.

>*The current version of this repository is 0.0.1-dev0 and is undergoing development for the first release client 0.1.0-rc0, which means that both the public interfaces and internal module structures may change significantly.*

**factor** is designed to support remote actors if required in future, so all interactions with the actor are only through messages. Even Lifecycle commands should also be sent as `SystemMessage::SystemCommand`, there are no direct methods like `stop()`. So all communications are async by default, in future we might add in-process sync-actors.


**factor provides:**

* `Typed` messages.
* `Ask` and `Tell` patterns.
* Uses `futures` for asynchronous message handling.
* Concurrency using `futures::executor::ThreadPool`.
* Quick `LocalPool` access to wait for multiple quick-tasks completion.
* Unbounded channels for messages (this might change.)
* Simple functional message handlers.
* Async response.
* Granular locks when possible.
* Runs on stable Rust 1.62+


**factor intends to provide (in future):**
* Actor supervision to recover from failure (restart).
* PubSub service for system topics.
* Message and task scheduling.
* Status dashboard for system and actors.
* Extend logging and tracing support.
* Remote actors.
* Thread level executor control.
* Ergonomic macros.
* Any messages.

**Usage:**

**factor** is not published to [crates.io](https://crates.io), to use add below dependency to your `Cargo.toml` file.
```toml
[dependencies]
factor = { git = "https://github.com/fasttravel-xyz/factor", branch = "0.0.1-dev0" }
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
    let spawn_item = ActorBuilder::create(OperationActor::default(), &sys);
    let addr = sys.run_actor(spawn_item.unwrap());

    // additions
    for i in 1..1000 {
        if let Ok(fut) = addr.ask(Operation::Add(i)) {
            let t = {
                async {
                    let _ = fut.await;
                }
            };
            if let Err(e) = factor::local_spawn(t) {
                print!("factor_local_spawn_error: {:?} ", e)
            }
        }
    }
    // subtractions
    for i in 501..1000 {
        if let Ok(fut) = addr.ask(Operation::Sub(i)) {
            let t = {
                async {
                    let _ = fut.await;
                }
            };
            if let Err(e) = factor::local_spawn(t) {
                print!("factor_local_spawn_error: {:?} ", e)
            }
        }
    }

    // run all tasks to completion
    factor::local_run();

    if let Ok(fut) = addr.ask(Operation::Sum) {
        let t = {
            async {
                match fut.await {
                    Ok(sum) => {
                        println!("Received Sum: {}. Expected Sum: {}", sum, 125250);
                        assert_eq!(sum, 125250);
                    }
                    Err(e) => {
                        println!("error_in_ask_operation_sum: {:?}", e);
                    }
                }
            }
        };
        let _ = factor::local_spawn(t);
    }

    factor::local_run();
}
```

For examples refer to the [tests] directory:

* [tell-ask test]
* [handshake test]
* [init-stop test]

More examples will be added to the [examples] directory after v0.1.0 release.

[tests]: https://github.com/
[examples]: https://github.com/
[tell-ask test]: https://github.com/
[handshake test]: https://github.com/
[init-stop test]: https://github.com/

```sh
# run all the tests:
cargo test

# run an example:
cargo run --example example_ask
```

## Contributing
Currently, this repository is not open for contributions.

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
