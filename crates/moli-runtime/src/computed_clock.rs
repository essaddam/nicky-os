//! One global projection worker, independently of the durable definition writer.
use super::{Arc, Duration, Hub, Instant, Ordering, Origin};
use crate::DependencyError;
use moli_core::{DependencyChange, DependencyKind};
impl Hub {
    pub(super) fn start_computed_clock(&self) {
        if !self.0.computed.active() {
            return;
        }
        self.0.computed_wake.notify_one();
        if self.0.computed_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let weak = Arc::downgrade(&self.0);
        let wake = self.0.computed_wake.clone();
        tokio::spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let hub = Hub(inner);
                if let Err(error) = hub.run_computations() {
                    let references: Vec<_> = hub
                        .read()
                        .dependency_heads
                        .keys()
                        .filter(|r| r.kind == DependencyKind::Computed)
                        .cloned()
                        .collect();
                    hub.fence_dependencies(Some(DependencyKind::Computed));
                    for reference in references {
                        hub.dependency_audit(
                            reference,
                            DependencyChange::Transition,
                            (Origin::System, None),
                            &Err::<crate::DependencyView, DependencyError>(error),
                            None,
                            Instant::now(),
                        );
                    }
                }
                let delay = hub
                    .computed_delay(moli_core::now_ms())
                    .map(|delay| delay.clamp(Duration::from_millis(1), Duration::from_secs(60)));
                drop(hub);
                if let Some(delay) = delay {
                    tokio::select! {()=wake.notified()=>{},()=tokio::time::sleep(delay)=>{}}
                } else {
                    wake.notified().await;
                }
            }
        });
    }
}
