//! Orders to devices behind a remote service (a maker's cloud): one at a
//! time, in the order received, never after their time, never sent twice;
//! then confirmed by what the service reads back, not by its « OK ».
//!
//! A driver owns an [`OrderLine`] and a [`Confirmations`] in its loop and
//! does the network beside it (a task per send): the line only decides
//! what may leave and who hears what, so every answer is on time whatever
//! the network does (see the Overkiz driver for the pattern).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use moli_core::{Device, DeviceId, Value};

use crate::{CommandRequest, DriverCtx};

/// The caller hears back this long before the hub stops waiting.
pub const REPLY_MARGIN: Duration = Duration::from_millis(400);
/// An order with less time than this left would get no answer (a cloud
/// call is a connection, a TLS handshake, a request): it stays home.
pub const MIN_LEFT: Duration = Duration::from_secs(1);

/// Orders waiting for the remote service, and the one on its way.
#[derive(Debug)]
pub struct OrderLine<O> {
    waiting: VecDeque<Waiting<O>>,
    sending: Option<Sending>,
    /// Callers waiting for an answer, orders sent or not: more are refused
    /// at once (the hub's own queue must not turn into unbounded work).
    capacity: usize,
}

#[derive(Debug)]
struct Waiting<O> {
    command: CommandRequest,
    order: O,
    /// The absolute instant past which it must not leave.
    reply_by: Instant,
}

#[derive(Debug)]
struct Sending {
    /// `None` once its caller heard « no answer in time ».
    command: Option<CommandRequest>,
    reply_by: Instant,
}

impl<O> OrderLine<O> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            waiting: VecDeque::new(),
            sending: None,
            capacity,
        }
    }

    /// Callers still waiting for an answer.
    #[must_use]
    pub fn len(&self) -> usize {
        self.waiting.len() + usize::from(self.sending.as_ref().is_some_and(|s| s.command.is_some()))
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// An order is on its way.
    #[must_use]
    pub fn sending(&self) -> bool {
        self.sending.is_some()
    }

    /// Takes an order: queued behind the others, or refused at once (line
    /// full, or no time left to send it).
    pub fn push(&mut self, command: CommandRequest, order: O) {
        let now = Instant::now();
        self.prune(now);
        if self.len() >= self.capacity {
            command.reply(Err(moli_i18n::tr!("pilotes.ordres.trop")));
            return;
        }
        let reply_by = command
            .deadline
            .checked_sub(REPLY_MARGIN)
            .unwrap_or(command.deadline);
        if now + MIN_LEFT >= reply_by {
            command.reply(Err("expired".into()));
            return;
        }
        self.waiting.push_back(Waiting {
            command,
            order,
            reply_by,
        });
    }

    /// The next order to send, when none is on its way: the oldest one
    /// still awaited with time left ([`MIN_LEFT`]), with the instant it must
    /// not leave after (the sending task checks it again when it starts,
    /// and gives up the wait for an answer then).
    pub fn to_send(&mut self) -> Option<(O, Instant)> {
        if self.sending.is_some() {
            return None;
        }
        self.prune(Instant::now());
        let Waiting {
            command,
            order,
            reply_by,
        } = self.waiting.pop_front()?;
        self.sending = Some(Sending {
            command: Some(command),
            reply_by,
        });
        Some((order, reply_by))
    }

    /// Answers the callers whose time came: queued orders that can no
    /// longer leave (« not sent »), the order on its way that got no answer
    /// (« maybe sent, never resent »). Returns when to look again.
    pub fn due(&mut self) -> Option<Instant> {
        let now = Instant::now();
        self.prune(now);
        if let Some(sending) = &mut self.sending
            && now >= sending.reply_by
            && let Some(command) = sending.command.take()
        {
            command.reply(Err(moli_i18n::tr!("pilotes.ordres.sans_reponse")));
        }
        let on_its_way = self
            .sending
            .as_ref()
            .filter(|s| s.command.is_some())
            .map(|s| s.reply_by);
        self.waiting
            .iter()
            .map(|w| w.reply_by.checked_sub(MIN_LEFT).unwrap_or(w.reply_by))
            .chain(on_its_way)
            .min()
    }

    /// The order on its way came back: its caller, unless already answered.
    /// A caller that left meanwhile is still returned (`abandoned()` says
    /// so): what happened must then be told to the household instead.
    pub fn sent(&mut self) -> Option<CommandRequest> {
        self.sending.take().and_then(|s| s.command)
    }

    /// The link is gone: nothing queued leaves any more, and the order on
    /// its way may or may not have left (never resent).
    pub fn close(&mut self) {
        for waiting in self.waiting.drain(..) {
            waiting
                .command
                .reply(Err(moli_i18n::tr!("pilotes.ordres.non_envoye")));
        }
        if let Some(command) = self.sending.take().and_then(|s| s.command) {
            command.reply(Err(moli_i18n::tr!("pilotes.ordres.liaison_perdue")));
        }
    }

    /// Queued orders whose caller left or whose time ran out never leave.
    fn prune(&mut self, now: Instant) {
        if self
            .waiting
            .iter()
            .all(|w| now + MIN_LEFT < w.reply_by && !w.command.abandoned())
        {
            return;
        }
        let waiting = std::mem::take(&mut self.waiting);
        for w in waiting {
            if now + MIN_LEFT >= w.reply_by || w.command.abandoned() {
                w.command
                    .reply(Err(moli_i18n::tr!("pilotes.ordres.non_envoye")));
            } else {
                self.waiting.push_back(w);
            }
        }
    }
}

/// Values the service took an order for, until it reads them back.
#[derive(Debug, Default)]
pub struct Confirmations {
    wants: Vec<Want>,
}

#[derive(Debug)]
struct Want {
    device: DeviceId,
    key: String,
    value: Value,
    /// For the household, if it never comes: the device's and the
    /// point's names.
    device_name: String,
    point_name: String,
    asked: Instant,
    until: Instant,
}

/// Values followed at once; the oldest is dropped beyond (logged).
const MAX_WANTS: usize = 32;

impl Confirmations {
    /// `key` of `device` should read `value` within `within`. A newer order
    /// on the same point replaces the older one's expectation.
    pub fn expect(&mut self, device: &Device, key: &str, value: Value, within: Duration) {
        self.wants
            .retain(|w| !(w.device == device.id && w.key == key));
        if self.wants.len() >= MAX_WANTS {
            let dropped = self.wants.remove(0);
            tracing::warn!(device = %dropped.device, key = %dropped.key, "confirmation no longer followed");
        }
        let now = Instant::now();
        self.wants.push(Want {
            device: device.id.clone(),
            key: key.to_owned(),
            value,
            device_name: device.native_name.to_string(),
            point_name: device
                .point(key)
                .map_or_else(|| key.to_owned(), |p| p.label.to_string()),
            asked: now,
            until: now + within,
        });
    }

    /// Something is still to be read back: the driver reads more often.
    #[must_use]
    pub fn watching(&self) -> bool {
        !self.wants.is_empty()
    }

    /// After a read: values read back are done; those still not there at
    /// their limit are told to the household (the order is never resent).
    pub fn check(&mut self, ctx: &DriverCtx) {
        let now = Instant::now();
        let mut late = Vec::new();
        self.wants.retain(|w| {
            if ctx
                .current(&w.device, &w.key)
                .is_some_and(|v| same(&v, &w.value))
            {
                return false;
            }
            if now >= w.until {
                late.push(moli_i18n::tr!(
                    "pilotes.ordres.non_confirme",
                    device = w.device_name,
                    point = w.point_name,
                    value = shown(&w.value),
                    seconds = now.duration_since(w.asked).as_secs()
                ));
                return false;
            }
            true
        });
        for message in late {
            ctx.media_publisher().tell(ctx.instance().as_str(), message);
        }
    }
}

/// Read back as asked: numbers within a hundredth (a service may answer
/// 55.0 for 55), anything else exactly.
fn same(read: &Value, asked: &Value) -> bool {
    match (read.as_f64(), asked.as_f64()) {
        (Some(a), Some(b)) => (a - b).abs() < 0.01,
        _ => read == asked,
    }
}

fn shown(value: &Value) -> String {
    match value {
        Value::Text(t) => t.to_string(),
        Value::Bool(true) => moli_i18n::tr!("pilotes.ordres.active"),
        Value::Bool(false) => moli_i18n::tr!("pilotes.ordres.desactive"),
        other => other.to_json().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU8;

    use moli_core::{Device, InstanceId, PointId};
    use tokio::sync::oneshot;

    use super::*;
    use crate::driver::Claim;

    fn order(within: Duration) -> (CommandRequest, oneshot::Receiver<Result<(), String>>) {
        let (tx, rx) = oneshot::channel();
        let id = DeviceId::new(&InstanceId::from("cloud"), "d1");
        let command = CommandRequest {
            point: PointId::new(&id, "on"),
            device: Arc::new(Device {
                id,
                instance: InstanceId::from("cloud"),
                native_name: "d1".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: None,
                members: Vec::new(),
                points: Vec::new(),
            }),
            key: "on".into(),
            value: Value::Bool(true),
            deadline: Instant::now() + within,
            reply: tx,
            claim: Arc::new(AtomicU8::new(Claim::TAKEN)),
        };
        (command, rx)
    }

    fn answer(rx: &mut oneshot::Receiver<Result<(), String>>) -> Option<Result<(), String>> {
        rx.try_recv().ok()
    }

    #[test]
    fn one_at_a_time_in_order() {
        let mut line = OrderLine::new(4);
        let (a, _ra) = order(Duration::from_secs(5));
        let (b, _rb) = order(Duration::from_secs(5));
        line.push(a, "open");
        line.push(b, "stop");
        assert_eq!(line.len(), 2);
        assert_eq!(line.to_send().map(|(o, _)| o), Some("open"));
        assert!(line.to_send().is_none(), "one on its way at most");
        line.sent().unwrap().reply(Ok(()));
        assert_eq!(line.to_send().map(|(o, _)| o), Some("stop"));
    }

    #[test]
    fn a_full_line_refuses_at_once() {
        let mut line = OrderLine::new(1);
        let (a, _ra) = order(Duration::from_secs(5));
        let (b, mut rb) = order(Duration::from_secs(5));
        line.push(a, 1);
        line.push(b, 2);
        assert!(answer(&mut rb).unwrap().is_err());
        // The order on its way still counts: its caller waits.
        line.to_send().unwrap();
        let (c, mut rc) = order(Duration::from_secs(5));
        line.push(c, 3);
        assert!(answer(&mut rc).unwrap().is_err());
    }

    #[test]
    fn an_order_out_of_time_never_leaves() {
        let mut line = OrderLine::new(4);
        // Less than the margin and a second left: refused at once.
        let (late, mut rl) = order(REPLY_MARGIN + MIN_LEFT);
        line.push(late, 0);
        assert_eq!(answer(&mut rl), Some(Err("expired".into())));
        // Queued in time, then its time runs out behind a slow one.
        let (a, _ra) = order(Duration::from_secs(5));
        let (b, mut rb) = order(REPLY_MARGIN + MIN_LEFT + Duration::from_millis(30));
        line.push(a, 1);
        line.push(b, 2);
        line.to_send().unwrap();
        std::thread::sleep(Duration::from_millis(40));
        line.due();
        assert!(
            answer(&mut rb)
                .unwrap()
                .unwrap_err()
                .contains("ordre non envoyé")
        );
        line.sent();
        assert!(line.to_send().is_none(), "never sent late");
    }

    #[test]
    fn a_caller_that_left_keeps_its_order_home() {
        let mut line = OrderLine::new(4);
        let (a, _ra) = order(Duration::from_secs(5));
        let (b, rb) = order(Duration::from_secs(5));
        line.push(a, 1);
        line.push(b, 2);
        line.to_send().unwrap();
        drop(rb);
        line.sent();
        assert!(line.to_send().is_none(), "its caller left: not sent");
        assert!(line.is_empty());
    }

    #[test]
    fn no_answer_in_time_is_said_once_and_the_late_answer_kept() {
        let mut line = OrderLine::new(4);
        let (a, mut ra) = order(REPLY_MARGIN + MIN_LEFT + Duration::from_millis(20));
        line.push(a, 1);
        let (_, reply_by) = line.to_send().unwrap();
        assert!(line.due().is_some_and(|at| at == reply_by));
        std::thread::sleep(MIN_LEFT + Duration::from_millis(30));
        assert_eq!(line.due(), None);
        assert!(
            answer(&mut ra)
                .unwrap()
                .unwrap_err()
                .contains("jamais renvoyé")
        );
        assert!(line.sending(), "still on its way");
        assert!(line.sent().is_none(), "its caller already heard");
    }

    #[test]
    fn closing_tells_everyone() {
        let mut line = OrderLine::new(4);
        let (a, mut ra) = order(Duration::from_secs(5));
        let (b, mut rb) = order(Duration::from_secs(5));
        line.push(a, 1);
        line.push(b, 2);
        line.to_send().unwrap();
        line.close();
        assert!(answer(&mut ra).unwrap().unwrap_err().contains("peut-être"));
        assert!(answer(&mut rb).unwrap().unwrap_err().contains("non envoyé"));
    }

    #[test]
    fn values_read_back() {
        assert!(same(&Value::Float(55.0), &Value::Int(55)));
        assert!(!same(&Value::Float(54.5), &Value::Int(55)));
        assert!(same(&Value::Text("eco".into()), &Value::Text("eco".into())));
        assert!(!same(&Value::Bool(true), &Value::Text("true".into())));
        assert_eq!(shown(&Value::Text("eco".into())), "eco");
        assert_eq!(shown(&Value::Float(21.5)), "21.5");
        assert_eq!(shown(&Value::Bool(false)), "désactivé");
    }
}
