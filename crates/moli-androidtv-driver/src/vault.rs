//! Accepted vault commits finish even when their driver session is dropped.
//! A replacement session waits for them before loading credentials or pins.
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

#[derive(Debug, Default)]
struct Pending {
    count: AtomicUsize,
    finished: Notify,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Writes(Arc<Pending>);

struct Accepted(Arc<Pending>);
impl Drop for Accepted {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::AcqRel);
        self.0.finished.notify_waiters();
    }
}

impl Writes {
    pub(crate) fn start<T: Send + 'static>(
        &self,
        write: impl Future<Output = T> + Send + 'static,
    ) -> JoinHandle<T> {
        // Register synchronously, before spawning. A restarting actor must
        // also wait for a commit whose task has not had its first poll yet.
        self.0.count.fetch_add(1, Ordering::AcqRel);
        let accepted = Accepted(self.0.clone());
        tokio::spawn(async move {
            let _accepted = accepted;
            write.await
        })
    }

    pub(crate) async fn wait(&self) {
        loop {
            let finished = self.0.finished.notified();
            tokio::pin!(finished);
            let _ = finished.as_mut().enable();
            if self.0.count.load(Ordering::Acquire) == 0 {
                return;
            }
            finished.await;
        }
    }
}
