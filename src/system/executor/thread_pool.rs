use futures::Future;
use log::error;
use std::{pin::Pin, sync::Arc};

use super::ExecutorSpawnError;

#[cfg(not(feature = "threadpool-tokio"))]
use futures::future::RemoteHandle;

#[cfg(all(feature = "threadpool-tokio"))]
use futures::ready;

#[cfg(not(feature = "threadpool-tokio"))]
use futures::{
    executor::{ThreadPool, ThreadPoolBuilder},
    task::SpawnExt,
};

#[cfg(not(feature = "threadpool-tokio"))]
pub(super) struct ThreadPoolWrapper {
    inner: ThreadPool,
}

#[cfg(all(feature = "threadpool-tokio"))]
use tokio::{
    runtime::{Builder, Runtime},
    task::JoinHandle,
};

#[cfg(all(feature = "threadpool-tokio"))]
pub(super) struct ThreadPoolWrapper {
    inner: Runtime,
}

#[cfg(not(feature = "threadpool-tokio"))]
impl ThreadPoolWrapper {
    pub(super) fn new(pool_size: Option<usize>, name_prefix: Option<&str>) -> Arc<Self> {
        // thread name prefix
        let mut pool_prefix = "factor_default_pool_executor_".to_owned();
        if let Some(prefix) = name_prefix {
            pool_prefix += prefix;
        }

        // thread count
        let num_threads = pool_size.unwrap_or(1);

        // build thread pool
        let mut builder = ThreadPoolBuilder::new();
        builder.name_prefix(pool_prefix.as_str());
        builder.pool_size(num_threads);
        let pool = builder.create().expect("pool_executor_creation_failed");

        Arc::new(Self { inner: pool })
    }

    pub(super) fn spawn<Fut>(&self, task: Fut)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        self.inner.spawn_ok(task);
    }

    pub(super) fn spawn_distribute<Fut>(&self, tasks: Vec<Fut>)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        for task in tasks {
            self.inner.spawn_ok(task);
        }
    }

    pub(super) fn spawn_with_handle<Fut>(
        &self,
        task: Fut,
    ) -> Result<RemoteJoinHandle<Fut::Output>, ExecutorSpawnError>
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        self.inner
            .spawn_with_handle(task)
            .map(|handle| RemoteJoinHandle {
                inner: InnerHandle::Default(Some(handle)),
            })
            .map_err(|e| {
                error!("pool_executor_spawn_error: {:#?}", e);
                ExecutorSpawnError
            })
    }
}

#[cfg(all(feature = "threadpool-tokio"))]
impl ThreadPoolWrapper {
    pub(super) fn new(pool_size: Option<usize>, name_prefix: Option<&str>) -> Arc<Self> {
        // thread name prefix
        let mut pool_prefix = "factor_default_pool_executor_".to_owned();
        if let Some(prefix) = name_prefix {
            pool_prefix += prefix;
        }

        // thread count
        let num_threads = pool_size.unwrap_or(1);

        // build thread pool
        let tokio_rt = Builder::new_multi_thread()
            .worker_threads(num_threads)
            .thread_name(pool_prefix.as_str())
            // .enable_all()
            .build()
            .expect("pool_executor_creation_failed");

        Arc::new(Self { inner: tokio_rt })
    }

    pub(super) fn spawn<Fut>(&self, task: Fut)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        self.inner.spawn(task);
    }

    pub(super) fn spawn_distribute<Fut>(&self, tasks: Vec<Fut>)
    where
        Fut: 'static + Future<Output = ()> + Send,
    {
        for task in tasks {
            self.inner.spawn(task);
        }
    }

    pub(super) fn spawn_with_handle<Fut>(
        &self,
        task: Fut,
    ) -> Result<RemoteJoinHandle<Fut::Output>, ExecutorSpawnError>
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send,
    {
        let handle = self.inner.spawn(task);

        Ok(RemoteJoinHandle {
            inner: InnerHandle::Tokio(Some(handle)),
        })
    }
}

///
/// RemoteJoinHandle wraps TokioJoinHandle and RemoteJoinHandle to provide a common
/// interface as they have diff interfaces and behavior.
///
/// TokioJoinHandle: * dettaches when dropped. * abort() method to cancel available.
/// RemoteHandle: * cancels task when dropped. * forget() method to dettach available.
/// RemoteJoinHandle: * cancels task when dropped. * forget() method to dettach available.
///
pub struct RemoteJoinHandle<T> {
    inner: InnerHandle<T>,
}
enum InnerHandle<T> {
    #[cfg(not(feature = "threadpool-tokio"))]
    Default(Option<RemoteHandle<T>>),
    #[cfg(all(feature = "threadpool-tokio"))]
    Tokio(Option<JoinHandle<T>>),
}

impl<T> RemoteJoinHandle<T> {
    pub fn forget(mut self) {
        match &mut self.inner {
            #[cfg(not(feature = "threadpool-tokio"))]
            InnerHandle::Default(handle) => {
                // call forget to dettach.
                handle.take().map(|remote_handle| remote_handle.forget());
            }
            #[cfg(all(feature = "threadpool-tokio"))]
            InnerHandle::Tokio(handle) => {
                // dropping the handle dettaches.
                let _ = handle.take();
            }
        }
    }
}

impl<T> Drop for RemoteJoinHandle<T> {
    fn drop(&mut self) {
        match &mut self.inner {
            #[cfg(not(feature = "threadpool-tokio"))]
            InnerHandle::Default(Some(_)) => {
                // we may end up here through two different paths,
                // direct drop or drop through forget().
                // As behavior of RemoteJoinHandle replicates RemoteHandle
                // we don't need to do anything.
            }
            #[cfg(all(feature = "threadpool-tokio"))]
            InnerHandle::Tokio(Some(handle)) => {
                // we may end up here through two different paths,
                // direct drop or drop through forget(). If we are
                // here through forget(), inner handle is already None.
                // But if we are here from direct drop, we need to call
                // abort() to replicate the behavior of RemoteHandle.
                handle.abort();
            }
            _ => {}
        }
    }
}

impl<T: 'static> Future for RemoteJoinHandle<T> {
    type Output = T;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match &mut self.get_mut().inner {
            #[cfg(not(feature = "threadpool-tokio"))]
            InnerHandle::Default(handle) => {
                // RemoteHandle's Future::Output is T
                Pin::new(handle)
                    .as_pin_mut()
                    .expect("polling_after_forget_panic")
                    .poll(cx)
            }
            #[cfg(all(feature = "threadpool-tokio"))]
            InnerHandle::Tokio(handle) => {
                // JoinHandle's Future::Output is Result<T>
                let poll = Pin::new(handle)
                    .as_pin_mut()
                    .expect("polling_after_forget_panic")
                    .poll(cx);
                match ready!(poll) {
                    Ok(output) => std::task::Poll::Ready(output),
                    Err(e) => {
                        error!("task_cancelled_or_has_panicked: {}", e);
                        panic!("task_cancelled_or_has_panicked: {}", e);
                    }
                }
            }
        }
    }
}
