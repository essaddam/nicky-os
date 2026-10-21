//! Accepted writes own their disk/cache continuation across actor restarts.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;
#[derive(Debug, Default)]
struct Pending {
    count: AtomicUsize,
    done: Notify,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Writes(Arc<Pending>);
struct Accepted(Arc<Pending>);
impl Drop for Accepted {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::AcqRel);
        self.0.done.notify_waiters();
    }
}
impl Writes {
    pub(crate) fn start<T: Send + 'static>(
        &self,
        future: impl std::future::Future<Output = T> + Send + 'static,
    ) -> tokio::task::JoinHandle<T> {
        self.0.count.fetch_add(1, Ordering::AcqRel);
        let accepted = Accepted(self.0.clone());
        tokio::spawn(async move {
            let _accepted = accepted;
            future.await
        })
    }
    pub(crate) async fn wait(&self) {
        loop {
            let done = self.0.done.notified();
            tokio::pin!(done);
            let _ = done.as_mut().enable();
            if self.0.count.load(Ordering::Acquire) == 0 {
                return;
            }
            done.await;
        }
    }
}
