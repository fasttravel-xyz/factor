use factor::prelude::*;
use std::sync::atomic::{AtomicU8, Ordering};

static SUM: AtomicU8 = AtomicU8::new(0);
static CALL_COUNT: AtomicU8 = AtomicU8::new(0);

struct FnMessageAdd(u8);
impl Message for FnMessageAdd {
    type Result = Option<u8>;
}

fn msg_handler(msg: FnMessageAdd) -> <FnMessageAdd as Message>::Result {
    SUM.fetch_add(msg.0, Ordering::SeqCst);
    CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    None
}

#[tokio::test]
async fn test_fn_handler() {
    let sys = factor::init_system(Some("TestSystem".to_string()));
    let spawn_item = builder::ActorBuilder::create_fn(msg_handler, &sys);
    let addr = sys.run_actor(spawn_item.unwrap());

    let mut result = addr.tell(FnMessageAdd(3));
    assert!(result.is_ok());

    result = addr.tell(FnMessageAdd(13));
    assert!(result.is_ok());

    result = addr.tell(FnMessageAdd(5));
    assert!(result.is_ok());

    let now = std::time::Instant::now();
    loop {
        let elapsed_time = now.elapsed();
        if elapsed_time > std::time::Duration::from_millis(5000) {
            break; // maximum wait time is 5 seconds
        }

        if CALL_COUNT.load(Ordering::SeqCst) >= 3 {
            break;
        } else {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 3);
    assert_eq!(SUM.load(Ordering::SeqCst), 21);
}
