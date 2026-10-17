//! Orders the box took, and what became of them. Pure bookkeeping, generic
//! over whoever waits for the answer (the hub's request in the driver, a
//! number in the tests), so the promises below are tested without a box:
//!
//! - a caller is answered **once**: « done » when the execution completed,
//!   its failure when it failed, « accepted » when the hub is about to stop
//!   waiting while the box is still at work;
//! - after « accepted », a failure is still caught (for a while) and told;
//! - nothing here ever sends an order: a lost link loses the waiters, it
//!   never replays them.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::model::Execution;

/// After « accepted », an execution is still watched this long for a failure.
pub const FOLLOW_FOR: Duration = Duration::from_secs(120);
/// Executions watched after « accepted », at most (the oldest go first).
pub const MAX_FOLLOWED: usize = 32;

/// What an order is, for the people who will read about it.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    pub device: Arc<str>,
    pub order: &'static str,
}

#[derive(Debug)]
struct Pending<W> {
    execution: String,
    waiter: W,
    about: About,
    reply_by: Instant,
}

#[derive(Debug)]
struct Followed {
    execution: String,
    about: About,
    until: Instant,
}

/// What to do now that an execution moved.
#[derive(Debug, PartialEq)]
pub enum Outcome<W> {
    /// Answer the caller: done, or failed with this cause.
    Answer(W, Result<(), String>),
    /// The caller already heard « accepted »: tell the household instead.
    Tell(About, String),
}

#[derive(Debug)]
pub struct Tracker<W> {
    pending: Vec<Pending<W>>,
    followed: VecDeque<Followed>,
}

impl<W> Default for Tracker<W> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            followed: VecDeque::new(),
        }
    }
}

impl<W> Tracker<W> {
    /// The box took an order: `waiter` hears of it by `reply_by` at the latest.
    pub fn track(&mut self, execution: String, waiter: W, about: About, reply_by: Instant) {
        self.pending.push(Pending {
            execution,
            waiter,
            about,
            reply_by,
        });
    }

    /// Some execution is still followed (fetch events more often).
    #[must_use]
    pub fn watching(&self) -> bool {
        !self.pending.is_empty() || !self.followed.is_empty()
    }

    /// When the next caller must be answered, whatever the box says.
    #[must_use]
    pub fn next_reply(&self) -> Option<Instant> {
        self.pending.iter().map(|p| p.reply_by).min()
    }

    /// An execution changed state.
    pub fn moved(
        &mut self,
        execution: &str,
        state: Execution,
        cause: Option<String>,
    ) -> Option<Outcome<W>> {
        if state == Execution::Running {
            return None;
        }
        let cause = cause.unwrap_or_default();
        if let Some(at) = self.pending.iter().position(|p| p.execution == execution) {
            let pending = self.pending.swap_remove(at);
            let result = if state == Execution::Completed {
                Ok(())
            } else {
                Err(cause)
            };
            return Some(Outcome::Answer(pending.waiter, result));
        }
        let at = self
            .followed
            .iter()
            .position(|f| f.execution == execution)?;
        let followed = self.followed.remove(at)?;
        (state == Execution::Failed).then(|| Outcome::Tell(followed.about, cause))
    }

    /// The callers whose time is up while the box is still at work: they
    /// hear « accepted », and their executions stay watched for a failure.
    pub fn due(&mut self, now: Instant) -> Vec<W> {
        self.followed.retain(|f| f.until > now);
        let (due, waiting): (Vec<_>, Vec<_>) =
            self.pending.drain(..).partition(|p| p.reply_by <= now);
        self.pending = waiting;
        due.into_iter()
            .map(|pending| {
                if self.followed.len() >= MAX_FOLLOWED {
                    self.followed.pop_front();
                }
                self.followed.push_back(Followed {
                    execution: pending.execution,
                    about: pending.about,
                    until: now + FOLLOW_FOR,
                });
                pending.waiter
            })
            .collect()
    }

    /// The link to the box is lost: every caller still waiting, to be told
    /// their order went out but was never confirmed. Nothing is resent.
    pub fn abandon(&mut self) -> Vec<W> {
        self.followed.clear();
        self.pending.drain(..).map(|p| p.waiter).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about() -> About {
        About {
            device: "Volet 1".into(),
            order: "close",
        }
    }

    fn tracker(now: Instant) -> Tracker<u32> {
        let mut t = Tracker::default();
        t.track("a".into(), 1, about(), now + Duration::from_secs(4));
        t.track("b".into(), 2, about(), now + Duration::from_secs(4));
        t
    }

    #[test]
    fn the_box_confirms_or_refuses() {
        let now = Instant::now();
        let mut t = tracker(now);
        assert!(t.watching());
        assert_eq!(t.moved("a", Execution::Running, None), None);
        assert_eq!(
            t.moved("a", Execution::Completed, None),
            Some(Outcome::Answer(1, Ok(())))
        );
        assert_eq!(
            t.moved("b", Execution::Failed, Some("NONEXEC_OTHER".into())),
            Some(Outcome::Answer(2, Err("NONEXEC_OTHER".into())))
        );
        // Answered once: the same event again (or a stranger's) does nothing.
        assert_eq!(t.moved("a", Execution::Completed, None), None);
        assert_eq!(t.moved("zzz", Execution::Failed, None), None);
        assert!(!t.watching());
    }

    #[test]
    fn a_slow_box_means_accepted_then_watched() {
        let now = Instant::now();
        let mut t = tracker(now);
        assert_eq!(t.next_reply(), Some(now + Duration::from_secs(4)));
        assert!(t.due(now).is_empty(), "still time to wait");
        let late = now + Duration::from_secs(4);
        assert_eq!(t.due(late), [1, 2]);
        assert!(t.due(late).is_empty(), "never answered twice");
        assert_eq!(t.next_reply(), None);
        assert!(t.watching());
        // Completed later: nothing to say. Failed later: the household hears it.
        assert_eq!(t.moved("a", Execution::Completed, None), None);
        assert_eq!(
            t.moved("b", Execution::Failed, Some("CMDCANCELLED".into())),
            Some(Outcome::Tell(about(), "CMDCANCELLED".into()))
        );
        assert!(!t.watching());
    }

    #[test]
    fn watching_after_accepted_is_bounded() {
        let now = Instant::now();
        let mut t: Tracker<u32> = Tracker::default();
        for n in 0..=u32::try_from(MAX_FOLLOWED).unwrap() {
            t.track(n.to_string(), n, about(), now);
        }
        assert_eq!(t.due(now).len(), MAX_FOLLOWED + 1);
        assert_eq!(t.followed.len(), MAX_FOLLOWED);
        assert_eq!(
            t.moved("0", Execution::Failed, None),
            None,
            "the oldest went first"
        );
        assert!(t.due(now + FOLLOW_FOR).is_empty());
        assert!(!t.watching(), "forgotten after a while");
    }

    #[test]
    fn a_lost_link_abandons_never_replays() {
        let now = Instant::now();
        let mut t = tracker(now);
        t.track("c".into(), 3, about(), now);
        assert_eq!(t.due(now), [3]);
        let mut lost = t.abandon();
        lost.sort_unstable();
        assert_eq!(lost, [1, 2]);
        assert!(!t.watching());
        assert_eq!(t.moved("a", Execution::Completed, None), None);
    }
}
