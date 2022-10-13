use chrono;
use factor::{builder::ActorBuilderConfig, init_system, prelude::*};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(all(unix, feature = "ipc-cluster"))]
use serde::{Deserialize, Serialize};

static ONCE_SUM: AtomicU16 = AtomicU16::new(0);
static REPEAT_SUM: AtomicU16 = AtomicU16::new(0);
static REPEAT_COUNT: AtomicU16 = AtomicU16::new(0);

struct TaskHandleWrapper {
    handle: Mutex<Option<ScheduledTaskHandle>>,
}

impl TaskHandleWrapper {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            handle: Mutex::new(None),
        })
    }

    fn set_handle(&self, handle: ScheduledTaskHandle) {
        let mut data = self.handle.lock().expect("handle_mutex_error");
        *data = Some(handle);
    }

    fn cancel(&self) {
        self.handle
            .lock()
            .expect("handle_mutex_error")
            .as_ref()
            .expect("real_handle_missing")
            .cancel();
    }
}

#[cfg_attr(all(unix, feature = "ipc-cluster"), derive(Serialize, Deserialize))]
struct MessageAdd(u16);
impl Message for MessageAdd {
    type Result = Option<u16>;
}

struct OpsReceiver {}
impl ActorReceiver for OpsReceiver {
    type Context = BasicContext<Self>;
}

impl MessageHandler<MessageAdd> for OpsReceiver {
    type Result = MessageResponseType<<MessageAdd as Message>::Result>;

    fn handle(&mut self, msg: MessageAdd, _ctx: &mut Self::Context) -> Self::Result {
        REPEAT_SUM.fetch_add(msg.0, Ordering::SeqCst);

        MessageResponseType::Result(None.into())
    }
}

async fn schedule_once_tasks(system: &SystemRef) {
    let once_task_add_5 = Box::pin(async {
        ONCE_SUM.fetch_add(5, Ordering::SeqCst);
    });

    let once_task_add_10 = Box::pin(async {
        ONCE_SUM.fetch_add(10, Ordering::SeqCst);
    });

    let once_task_add_3 = Box::pin(async {
        ONCE_SUM.fetch_add(3, Ordering::SeqCst);
    });

    let once_task_add_20 = Box::pin(async {
        ONCE_SUM.fetch_add(20, Ordering::SeqCst);
    });

    // schedule once after 200 ms delay.
    let handle_a = system
        .schedule_once(once_task_add_5, Duration::from_millis(200))
        .expect("schedule_once_failed");

    // schedule once after 200 ms delay.
    let handle_b = system
        .schedule_once(once_task_add_10, Duration::from_millis(200))
        .expect("schedule_once_failed");

    // schedule once at Utc time that 300 ms from current Utc Time.
    let instant = chrono::Utc::now() + chrono::Duration::milliseconds(300);
    let handle_c = system
        .schedule_once_at(once_task_add_3, instant)
        .expect("schedule_once_at_failed");

    // schedule once after 10 seconds delay, so that we have enough time to cancel.
    let handle_d = system
        .schedule_once(once_task_add_20, Duration::from_millis(10000))
        .expect("schedule_once_failed");

    // sleep for 2 seconds
    std::thread::sleep(Duration::from_millis(2000));

    assert_eq!(handle_a.status(), 1);
    assert_eq!(handle_b.status(), 1);
    assert_eq!(handle_c.status(), 1);
    assert_eq!(handle_d.status(), 0);
    handle_d.cancel();
    assert_eq!(handle_d.status(), -1);

    let once_sum = ONCE_SUM.load(Ordering::SeqCst);

    assert_eq!(once_sum, 18, "scheduled_once_task_failed_to_update_sum");
}

use futures::Future;
use std::pin::Pin;

async fn schedule_repeat_tasks(system: &SystemRef) {
    let spawn_item =
        builder::ActorBuilder::create(|_| OpsReceiver {}, &system, ActorBuilderConfig::default());
    let addr = system.run_actor(spawn_item.unwrap());
    let addr_moved = addr.clone();

    let handle = TaskHandleWrapper::new();
    let handle_moved = handle.clone();

    let factory = move || {
        let addr_task = addr_moved.clone();
        let handle_task = handle_moved.clone();

        let task: Pin<Box<dyn Future<Output = ()> + Send + 'static>> = Box::pin(async move {
            REPEAT_COUNT.fetch_add(1, Ordering::SeqCst);
            addr_task.tell(MessageAdd(3)).expect("addr_tell_failed");

            if REPEAT_COUNT.load(Ordering::SeqCst) == 5 {
                handle_task.cancel();
            }
        });

        task
    };

    let initial_delay = Duration::from_millis(0);
    let repeat_interval = Duration::from_millis(200);
    let real_handle = system
        .schedule_repeat(Box::new(factory), initial_delay, repeat_interval)
        .expect("schedule_repeat_failed");
    handle.set_handle(real_handle.clone());

    // sleep for 2 seconds
    std::thread::sleep(Duration::from_millis(2000));

    let repeat_sum = REPEAT_SUM.load(Ordering::SeqCst);

    assert_eq!(real_handle.status(), -1);
    assert_eq!(repeat_sum, 15, "scheduled_once_task_failed_to_update_sum");
}


async fn spawn_with_handle(system: &SystemRef) {
    let remote_handle_1 = system.spawn_with_handle(async {
        std::thread::sleep(Duration::from_millis(100));
        6789
    }).expect("system_spawn_with_handle_failed");

    let remote_handle_2 = system.spawn_with_handle(async {
        std::thread::sleep(Duration::from_millis(500));
        1234
    }).expect("system_spawn_with_handle_failed");

    let ans2 = remote_handle_2.await;
    let ans1 = remote_handle_1.await;

    assert_eq!(6789, ans1);
    assert_eq!(1234, ans2);
}

#[tokio::test]
async fn test_schedule_tasks() {
    let system = init_system(Some("test_system".to_owned()));
    spawn_with_handle(&system).await;
    schedule_once_tasks(&system).await;
    schedule_repeat_tasks(&system).await;
}
