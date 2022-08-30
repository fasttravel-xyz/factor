//! Example of the ask-pattern. Users could send messages and wait for
//! the response before proceeding.

// cargo run --example example_ask
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
    let spawn_item = builder::ActorBuilder::create(|| OperationActor::default(), &sys);
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
