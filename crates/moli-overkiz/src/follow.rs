//! Orders handed to the box, and what became of them. Pure bookkeeping,
//! generic over whoever waits for the answer (the hub's request in the
//! driver, a number in the tests), so the promises below are tested without
//! a box:
//!
//! - a caller is answered **once**, by its deadline whatever the network
//!   does: « done » when the execution completed, its failure when it
//!   failed, « accepted » when the box took the order and is still at work,
//!   « no answer » when the box had not even said whether it took it;
//! - after « accepted » (or « no answer » followed by the box taking it), a
//!   failure is still caught for a while and told to the household;
//! - an execution may finish before the box's answer to the order arrives
//!   (both travel separately): its outcome is kept until the answer does;
//! - nothing here ever sends an order: a lost link loses the waiters, it
//!   never replays them.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::model::Execution;

/// After the caller was answered, an execution is still watched this long.
pub const FOLLOW_FOR: Duration = Duration::from_secs(120);
/// Executions watched after the answer, at most (the oldest go first).
pub const MAX_FOLLOWED: usize = 32;

/// What an order is, for the people who will read about it.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    pub device: Arc<str>,
    pub order: &'static str,
}

#[derive(Debug)]
struct Pending<W> {
    key: u64,
    /// `None` while the box has not answered the order yet.
    execution: Option<String>,
    waiter: W,
    about: About,
    reply_by: Instant,
}

/// Answered, still watched: by its execution, or by its key while the box
/// has not said whether it took the order.
#[derive(Debug)]
struct Followed {
    key: u64,
    execution: Option<String>,
    about: About,
    until: Instant,
}

/// An execution that finished before its id was known here.
#[derive(Debug)]
struct Early {
    execution: String,
    failed: bool,
    cause: String,
}

/// What to do now that an execution moved.
#[derive(Debug, PartialEq)]
pub enum Outcome<W> {
    /// Answer the caller: done, or failed with this cause.
    Answer(W, Result<(), String>),
    /// The caller was already answered: tell the household instead.
    Tell(About, String),
}

/// A caller whose time is up.
#[derive(Debug, PartialEq)]
pub enum Due<W> {
    /// The box took the order and is still at it.
    Accepted(W),
    /// The box has not said whether it took the order.
    Unanswered(W),
}

#[derive(Debug)]
pub struct Tracker<W> {
    pending: Vec<Pending<W>>,
    followed: VecDeque<Followed>,
    early: VecDeque<Early>,
}

impl<W> Default for Tracker<W> {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            followed: VecDeque::new(),
            early: VecDeque::new(),
        }
    }
}

fn result(failed: bool, cause: String) -> Result<(), String> {
    if failed { Err(cause) } else { Ok(()) }
}

impl<W> Tracker<W> {
    /// An order is on its way to the box: `waiter` hears of it by `reply_by`.
    pub fn track(&mut self, key: u64, waiter: W, about: About, reply_by: Instant) {
        self.pending.push(Pending {
            key,
            execution: None,
            waiter,
            about,
            reply_by,
        });
    }

    /// Some order is still followed (fetch events more often).
    #[must_use]
    pub fn watching(&self) -> bool {
        !self.pending.is_empty() || !self.followed.is_empty()
    }

    /// When the next caller must be answered, whatever the box says.
    #[must_use]
    pub fn next_reply(&self) -> Option<Instant> {
        self.pending.iter().map(|p| p.reply_by).min()
    }

    fn follow(&mut self, key: u64, execution: Option<String>, about: About, until: Instant) {
        if self.followed.len() >= MAX_FOLLOWED {
            self.followed.pop_front();
        }
        self.followed.push_back(Followed {
            key,
            execution,
            about,
            until,
        });
    }

    /// The box took order `key` as `execution`.
    pub fn taken(&mut self, key: u64, execution: String) -> Option<Outcome<W>> {
        let early = self
            .early
            .iter()
            .position(|e| e.execution == execution)
            .and_then(|at| self.early.remove(at));
        if let Some(at) = self.pending.iter().position(|p| p.key == key) {
            if let Some(early) = early {
                let pending = self.pending.swap_remove(at);
                return Some(Outcome::Answer(
                    pending.waiter,
                    result(early.failed, early.cause),
                ));
            }
            self.pending[at].execution = Some(execution);
            return None;
        }
        // Answered « no answer » already: now watched by its execution.
        let at = self
            .followed
            .iter()
            .position(|f| f.key == key && f.execution.is_none())?;
        let followed = self.followed.remove(at)?;
        if let Some(early) = early {
            return early
                .failed
                .then(|| Outcome::Tell(followed.about, early.cause));
        }
        self.follow(key, Some(execution), followed.about, followed.until);
        None
    }

    /// The box refused order `key` (or never answered): its caller, if it
    /// still waits. Nothing to watch any more.
    pub fn refused(&mut self, key: u64) -> Option<W> {
        self.followed.retain(|f| f.key != key);
        let at = self.pending.iter().position(|p| p.key == key)?;
        Some(self.pending.swap_remove(at).waiter)
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
        let failed = state == Execution::Failed;
        let cause = cause.unwrap_or_default();
        let ours = |e: &Option<String>| e.as_deref() == Some(execution);
        if let Some(at) = self.pending.iter().position(|p| ours(&p.execution)) {
            let pending = self.pending.swap_remove(at);
            return Some(Outcome::Answer(pending.waiter, result(failed, cause)));
        }
        if let Some(at) = self.followed.iter().position(|f| ours(&f.execution)) {
            let followed = self.followed.remove(at)?;
            return failed.then(|| Outcome::Tell(followed.about, cause));
        }
        // Maybe one of ours whose id has not arrived yet; otherwise another
        // application's: kept only while an answer from the box is awaited.
        let awaited = self.pending.iter().any(|p| p.execution.is_none())
            || self.followed.iter().any(|f| f.execution.is_none());
        if awaited {
            if self.early.len() >= MAX_FOLLOWED {
                self.early.pop_front();
            }
            self.early.push_back(Early {
                execution: execution.to_owned(),
                failed,
                cause,
            });
        }
        None
    }

    /// The callers whose time is up; they stay watched for a while.
    pub fn due(&mut self, now: Instant) -> Vec<Due<W>> {
        self.followed.retain(|f| f.until > now);
        let (due, waiting): (Vec<_>, Vec<_>) =
            self.pending.drain(..).partition(|p| p.reply_by <= now);
        self.pending = waiting;
        if !self.watching() {
            self.early.clear();
        }
        due.into_iter()
            .map(|p| {
                let taken = p.execution.is_some();
                self.follow(p.key, p.execution, p.about, now + FOLLOW_FOR);
                if taken {
                    Due::Accepted(p.waiter)
                } else {
                    Due::Unanswered(p.waiter)
                }
            })
            .collect()
    }

    /// The link to the box is lost: every caller still waiting, to be told
    /// their order went out but was never confirmed. Nothing is resent.
    pub fn abandon(&mut self) -> Vec<W> {
        self.followed.clear();
        self.early.clear();
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
        t.track(1, 1, about(), now + Duration::from_secs(4));
        t.track(2, 2, about(), now + Duration::from_secs(4));
        assert_eq!(t.taken(1, "a".into()), None);
        assert_eq!(t.taken(2, "b".into()), None);
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
        assert_eq!(t.due(late), [Due::Accepted(1), Due::Accepted(2)]);
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
    fn a_box_slow_to_take_the_order() {
        let now = Instant::now();
        let mut t: Tracker<u32> = Tracker::default();
        t.track(7, 7, about(), now);
        assert_eq!(t.due(now), [Due::Unanswered(7)]);
        // It took the order after all, then failed: the household hears it.
        assert_eq!(t.taken(7, "x".into()), None);
        assert_eq!(
            t.moved("x", Execution::Failed, Some("NONEXEC_OTHER".into())),
            Some(Outcome::Tell(about(), "NONEXEC_OTHER".into()))
        );
        // Or the box refused it: nothing more to watch.
        t.track(8, 8, about(), now);
        assert_eq!(t.due(now), [Due::Unanswered(8)]);
        assert_eq!(t.refused(8), None, "already answered");
        assert!(!t.watching());
    }

    #[test]
    fn an_execution_may_finish_before_its_id_arrives() {
        let now = Instant::now();
        let mut t: Tracker<u32> = Tracker::default();
        // Another application's execution, nothing awaited: forgotten.
        assert_eq!(t.moved("other", Execution::Completed, None), None);
        t.track(1, 1, about(), now + Duration::from_secs(4));
        assert_eq!(t.moved("e", Execution::Running, None), None);
        assert_eq!(t.moved("e", Execution::Completed, None), None);
        assert_eq!(t.taken(1, "e".into()), Some(Outcome::Answer(1, Ok(()))));
        assert!(!t.watching());
        t.track(2, 2, about(), now + Duration::from_secs(4));
        assert_eq!(
            t.taken(2, "other".into()),
            None,
            "a stranger's was never kept"
        );
    }

    #[test]
    fn a_refused_order_is_answered_by_its_key() {
        let now = Instant::now();
        let mut t: Tracker<u32> = Tracker::default();
        t.track(3, 3, about(), now + Duration::from_secs(4));
        assert_eq!(t.refused(3), Some(3));
        assert_eq!(t.refused(3), None, "answered once");
        assert!(!t.watching());
    }

    #[test]
    fn watching_after_the_answer_is_bounded() {
        let now = Instant::now();
        let mut t: Tracker<u32> = Tracker::default();
        for n in 0..=u32::try_from(MAX_FOLLOWED).unwrap() {
            t.track(n.into(), n, about(), now);
            assert_eq!(t.taken(n.into(), n.to_string()), None);
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
        t.track(3, 3, about(), now);
        assert_eq!(t.due(now), [Due::Unanswered(3)]);
        let mut lost = t.abandon();
        lost.sort_unstable();
        assert_eq!(lost, [1, 2]);
        assert!(!t.watching());
        assert_eq!(t.moved("a", Execution::Completed, None), None);
    }
}
