use chrono::{DateTime, Utc};
use futures::Future;

use log::{debug, error, trace};
use std::cmp::{Eq, Ord, Ordering, PartialEq, PartialOrd};
use std::collections::BinaryHeap;
use std::pin::Pin;
use std::sync::atomic::{self, AtomicI8};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
// use std::panic::{self, AssertUnwindSafe};

mod thread_pool;
pub use thread_pool::RemoteJoinHandle;
use thread_pool::*;

/// Actor core executor type.
#[derive(Clone)]
pub(crate) enum ActorExecutor {
    /// System executor, uses the system ThreadPool executor.
    System,
    /// Pool of multiple actor cores, have their own ThreadPool executor.
    /// For CPU bound tasks with heavy computation requirements.
    ///
    /// Currently, this uses a ThreadPool executor which imposes Send bound
    /// on the tasks. We could provide a pool with multiple separate localpools
    /// but with no Send bound on the tasks. For this we have to manage the
    /// initial task submission part to different localpools.
    Pool(Arc<Box<ThreadPoolExecutor>>),
}

pub type ExecutorTask = Pin<Box<dyn Future<Output = ()> + 'static + Send>>;
pub type ExecutorTaskFactory = Box<dyn Fn() -> ExecutorTask + Send + 'static>;

pub(crate) struct ThreadPoolExecutor {
    pool: Arc<ThreadPoolWrapper>,
    scheduler_handle: SchedulerHandle,
}

impl ThreadPoolExecutor {
    pub(crate) fn new(pool_size: Option<usize>, name_prefix: Option<&str>) -> Self {
        let mut pool_prefix = "pool_executor_".to_owned();
        if let Some(prefix) = name_prefix {
            pool_prefix += prefix;
        }

        let pool = ThreadPoolWrapper::new(pool_size, name_prefix);
        let scheduler_handle = Scheduler::launch(pool.clone());

        Self {
            pool,
            scheduler_handle,
        }
    }

    /// Spawn a future in the executor.
    pub(crate) fn spawn_ok<Fut>(&self, task: Fut)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        self.pool.spawn(task);
    }

    /// Spawn futures in the executor in a round-robin manner into the available threads.
    ///
    /// NOTE: Currently, we use a ThreadPool so this method spawns all the futures
    /// into the same pool. In future this method could help to map individual loopers
    /// of an actorpool to individual threads.
    pub(crate) fn spawn_ok_distribute<Fut>(&self, tasks: Vec<Fut>)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        self.pool.spawn_distribute(tasks);
    }

    /// Spawn a future in the executor and get a handle to the output.
    pub(crate) fn spawn_with_handle<Fut>(
        &self,
        task: Fut,
    ) -> Result<RemoteJoinHandle<Fut::Output>, ExecutorSpawnError>
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        self.pool.spawn_with_handle(task).map_err(|e| {
            error!("pool_executor_spawn_error: {:#?}", e);
            ExecutorSpawnError
        })
    }

    /// Schedule a Task once.
    pub(crate) fn schedule_once(
        &self,
        task: ExecutorTask,
        delay: Duration,
    ) -> Result<ScheduledTaskHandle, ExecutorScheduleError> {
        let status = Arc::new(AtomicI8::new(0));
        let at_time = Instant::now() + delay;
        let once_task = OnceTask {
            at_time,
            status: status.clone(),
            task,
        };
        self.schedule(ScheduledTask::Once(once_task))?;

        Ok(ScheduledTaskHandle { status })
    }

    /// Schedule a Task at an Utc DateTime.
    pub(crate) fn schedule_once_at(
        &self,
        task: ExecutorTask,
        at: DateTime<Utc>,
    ) -> Result<ScheduledTaskHandle, ExecutorScheduleError> {
        let status = Arc::new(AtomicI8::new(0));
        let chrono_duration = at - Utc::now();
        let at_time = Instant::now() + chrono_duration.to_std().expect("schedule_time_error");
        let once_task = OnceTask {
            at_time,
            status: status.clone(),
            task,
        };
        self.schedule(ScheduledTask::Once(once_task))?;

        Ok(ScheduledTaskHandle { status })
    }

    /// Schedule a Task that repeats at regular interval.
    pub(crate) fn schedule_repeat(
        &self,
        factory: ExecutorTaskFactory,
        delay: Duration,
        interval: Duration,
    ) -> Result<ScheduledTaskHandle, ExecutorScheduleError> {
        let status = Arc::new(AtomicI8::new(0));
        let at_time = Instant::now() + delay;
        let repeat_task = RepeatTask {
            at_time,
            interval,
            status: status.clone(),
            factory,
        };

        self.schedule(ScheduledTask::Repeat(repeat_task))?;

        Ok(ScheduledTaskHandle { status })
    }

    fn schedule(&self, task: ScheduledTask) -> Result<(), ExecutorScheduleError> {
        if let Ok(sender) = self.scheduler_handle.sender.lock() {
            return sender.send(task).map_err(|e| {
                error!("send_to_scheduler_error: {:#?}", e);
                ExecutorScheduleError
            });
        };

        Err(ExecutorScheduleError)
    }

    // [todo][low-priority]: create a separate crate that provides APIs around threadpool(s)
    // and localpool(s) with all these convenience functions and support for blocking FnOnce and FnMut.
    // fn schedule_once_with_handle()
    // fn schedule_once_at_with_handle()
    // fn schedule_repeat_with_stream()
}

/// Executor Spawn Error.
#[derive(Debug)]
pub struct ExecutorSpawnError;

/// Executor Spawn Error.
#[derive(Debug)]
pub struct ExecutorScheduleError;

// [todo] expose these to public
const TASK_CANCELLED: i8 = -1;
const TASK_SCHEDULED: i8 = 0;
const TASK_FINISHED: i8 = 1;

#[derive(Clone)]
pub struct ScheduledTaskHandle {
    status: Arc<AtomicI8>,
}

impl ScheduledTaskHandle {
    pub fn cancel(&self) {
        self.status.store(TASK_CANCELLED, atomic::Ordering::SeqCst);
    }

    /// status: -2: errored, -1: cancelled, 0: scheduled, 1: finished.
    pub fn status(&self) -> i8 {
        self.status.load(atomic::Ordering::SeqCst)
    }
}

enum ScheduledTask {
    Once(OnceTask),
    Repeat(RepeatTask),
}

struct OnceTask {
    at_time: Instant,
    status: Arc<AtomicI8>,
    task: ExecutorTask,
}

struct RepeatTask {
    at_time: Instant,
    interval: Duration,
    status: Arc<AtomicI8>,
    factory: ExecutorTaskFactory,
}

struct Scheduler {
    pool: Arc<ThreadPoolWrapper>,
    buffer: Receiver<ScheduledTask>,
    priority_queue: BinaryHeap<ScheduledTask>,
}

struct SchedulerHandle {
    sender: Mutex<Sender<ScheduledTask>>,
}

impl Scheduler {
    fn new(pool: Arc<ThreadPoolWrapper>, buffer: Receiver<ScheduledTask>) -> Self {
        Self {
            pool,
            buffer,
            priority_queue: BinaryHeap::new(),
        }
    }

    fn launch(pool: Arc<ThreadPoolWrapper>) -> SchedulerHandle {
        let (tx, rx) = channel::<ScheduledTask>();
        let handle = SchedulerHandle {
            sender: Mutex::new(tx),
        };
        thread::spawn(move || {
            let scheduler = Scheduler::new(pool, rx);
            scheduler.run();
        });

        handle
    }

    fn run(mut self) {
        let max_tasks_per_loop = 100 as u8;
        loop {
            let mut next_scheduled_instant = Instant::now();
            let mut priority_queue_is_empty = true;

            debug!(
                "scheduler_loop_priority_queue_count: {}",
                self.priority_queue.len()
            );

            while let Some(task) = self.priority_queue.peek() {
                trace!("priority_queue_peek_count: {}", self.priority_queue.len());

                priority_queue_is_empty = false;
                let now = Instant::now();
                match task {
                    ScheduledTask::Once(once_task_peeked) => {
                        if once_task_peeked.at_time <= now {
                            debug!("once_task_getting_poped");

                            if let Some(ScheduledTask::Once(once_task)) = self.priority_queue.pop()
                            {
                                if once_task.status.load(atomic::Ordering::SeqCst) == TASK_SCHEDULED
                                {
                                    let submit = async move {
                                        once_task.task.await;
                                        once_task
                                            .status
                                            .store(TASK_FINISHED, atomic::Ordering::SeqCst);
                                    };
                                    self.pool.spawn(submit);
                                }
                            }
                        } else {
                            next_scheduled_instant = once_task_peeked.at_time;
                            break;
                        }
                    }
                    ScheduledTask::Repeat(repeat_task_peeked) => {
                        if repeat_task_peeked.at_time <= now {
                            debug!("repeat_task_getting_poped");

                            if let Some(ScheduledTask::Repeat(repeat_task)) =
                                self.priority_queue.pop()
                            {
                                if repeat_task.status.load(atomic::Ordering::SeqCst)
                                    == TASK_SCHEDULED
                                {
                                    let task = (repeat_task.factory)();
                                    let next_repeat = RepeatTask {
                                        at_time: now + repeat_task.interval,
                                        interval: repeat_task.interval,
                                        status: repeat_task.status.clone(),
                                        factory: repeat_task.factory,
                                    };
                                    self.priority_queue.push(ScheduledTask::Repeat(next_repeat));

                                    let submit = async move {
                                        task.await;
                                    };
                                    self.pool.spawn(submit);
                                }
                            }
                        } else {
                            next_scheduled_instant = repeat_task_peeked.at_time;
                            break;
                        }
                    }
                }
            }

            // Transfer tasks from the queue to the priority_queue.
            for _ in 0..max_tasks_per_loop {
                if let Ok(task) = self.buffer.try_recv() {
                    self.priority_queue.push(task);
                    next_scheduled_instant = Instant::now(); // we need to check the newly added tasks.
                    priority_queue_is_empty = false;
                } else {
                    let now = Instant::now();
                    if next_scheduled_instant > now {
                        let timeout = next_scheduled_instant - now;
                        debug!("buffer_recv_wait_timeout: {}", timeout.as_millis());

                        self.buffer
                            .recv_timeout(timeout)
                            .map(|task| self.priority_queue.push(task))
                            .map_err(|e| trace!("{:#?}", e))
                            .err();
                    } else if priority_queue_is_empty {
                        debug!("priority_queue_empty_buffer_recv_wait");

                        self.buffer
                            .recv()
                            .map(|task| self.priority_queue.push(task))
                            .map_err(|e| debug!("{:#?}", e))
                            .err();
                    }

                    break;
                }
            }
        }
    }
}

impl PartialOrd for ScheduledTask {
    fn partial_cmp(&self, other: &ScheduledTask) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledTask {
    fn cmp(&self, other: &ScheduledTask) -> Ordering {
        let at_time_self = match self {
            ScheduledTask::Once(once_task) => once_task.at_time,
            ScheduledTask::Repeat(repeat_task) => repeat_task.at_time,
        };
        let at_time_other = match other {
            ScheduledTask::Once(once_task) => once_task.at_time,
            ScheduledTask::Repeat(repeat_task) => repeat_task.at_time,
        };

        // BinaryHeap is a Max Priority Queue.
        at_time_self.cmp(&at_time_other).reverse()
    }
}

impl PartialEq for ScheduledTask {
    fn eq(&self, other: &ScheduledTask) -> bool {
        let at_time_self = match self {
            ScheduledTask::Once(once_task) => once_task.at_time,
            ScheduledTask::Repeat(repeat_task) => repeat_task.at_time,
        };
        let at_time_other = match other {
            ScheduledTask::Once(once_task) => once_task.at_time,
            ScheduledTask::Repeat(repeat_task) => repeat_task.at_time,
        };

        at_time_self == at_time_other
    }
}

impl Eq for ScheduledTask {}
