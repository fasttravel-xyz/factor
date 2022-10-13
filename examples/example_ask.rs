//! Example of the ask-pattern. Users could send messages and wait for
//! the response before proceeding.

// cargo run --example example_ask
use factor::{builder::ActorBuilderConfig, prelude::*};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
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
    let spawn_item = ActorBuilder::create(
        |_| OperationActor::default(),
        &sys,
        ActorBuilderConfig::default(),
    );
    let addr = sys.run_actor(spawn_item.unwrap());

    // perform additions synchronously by awaiting.
    let addr_moved = addr.clone();
    let add_ops = async move {
        for i in 1..1000 {
            addr_moved
                .ask(Operation::Add(i))
                .await
                .map_err(|e| println!("factor_ask_error: {:?} ", e))
                .err();
        }
    };

    factor::local_spawn(add_ops)
        .map_err(|e| println!("factor_local_spawn_error: {:?} ", e))
        .err();

    // perform subtractions asynchronously.
    for i in 501..1000 {
        let addr_moved = addr.clone();
        let sub_op = async move {
            addr_moved
                .ask(Operation::Sub(i))
                .await
                .map_err(|e| println!("factor_ask_error: {:?} ", e))
                .err();
        };
        factor::local_spawn(sub_op)
            .map_err(|e| println!("factor_local_spawn_error: {:?} ", e))
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
        .map_err(|e| println!("factor_local_spawn_error: {:?} ", e))
        .err();

    factor::local_run();
}
