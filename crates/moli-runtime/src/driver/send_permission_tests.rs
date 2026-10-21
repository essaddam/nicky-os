use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

fn request() -> (
    CommandRequest,
    oneshot::Receiver<Result<(), crate::CommandError>>,
    Arc<AtomicBool>,
) {
    let point = PointId::from("fixture:device/on");
    let target = point.clone();
    let allowed = Arc::new(AtomicBool::new(true));
    let live = Arc::clone(&allowed);
    let (reply, answer) = oneshot::channel();
    let command = CommandRequest {
        point,
        device: Arc::new(Device {
            id: "fixture:device".into(),
            instance: "fixture".into(),
            native_name: "Synthetic device".into(),
            manufacturer: None,
            model: None,
            description: None,
            native_room: None,
            members: vec![],
            points: vec![],
        }),
        key: "on".into(),
        value: Value::Bool(true),
        deadline: Instant::now() + Duration::from_secs(5),
        metadata: CommandMetadata {
            origin: Origin::Automation,
            actor: Some("Synthetic actor".into()),
            request_id: None,
            correlation: Some("synthetic-correlation".into()),
        },
        permit: Some(CommandPermit::new(move |point| {
            point == &target && live.load(Ordering::Acquire)
        })),
        reply: Arc::new(std::sync::Mutex::new(Some(reply))),
        claim: Arc::new(AtomicU8::new(Claim::TAKEN)),
    };
    (command, answer, allowed)
}

#[tokio::test]
async fn a_detached_send_permit_refuses_revocation_while_its_transport_is_still_blocked() {
    let (command, answer, allowed) = request();
    let permit = command.send_permit(command.deadline);
    let detached = permit.clone();
    let emissions = Arc::new(AtomicUsize::new(0));
    let sent = Arc::clone(&emissions);
    let (held_tx, held_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let transport = tokio::spawn(async move {
        held_tx.send(()).unwrap();
        release_rx.await.unwrap();
        if detached.begin_send() {
            sent.fetch_add(1, Ordering::AcqRel);
        }
        command.reply_result(Err(crate::CommandError::AccessDenied));
    });
    held_rx.await.unwrap();
    allowed.store(false, Ordering::Release);
    // The transport task has not been released: checking its clone must
    // observe the new policy immediately, without polling abandoned().
    assert!(!permit.wanted());
    assert!(!permit.begin_send());
    assert!(!permit.emitted());
    release_tx.send(()).unwrap();
    transport.await.unwrap();
    assert_eq!(emissions.load(Ordering::Acquire), 0);
    assert_eq!(
        answer.await.unwrap(),
        Err(crate::CommandError::AccessDenied)
    );
}

#[tokio::test]
async fn an_authorized_detached_transport_emits_once_and_keeps_its_structured_result() {
    let (command, answer, allowed) = request();
    let permit = command.send_permit(command.deadline);
    let detached = permit.clone();
    assert_eq!(command.metadata.origin, Origin::Automation);
    assert_eq!(command.metadata.actor.as_deref(), Some("Synthetic actor"));
    assert_eq!(
        command.metadata.correlation.as_deref(),
        Some("synthetic-correlation")
    );
    assert!(detached.wanted());
    assert!(detached.begin_send());
    assert!(permit.emitted(), "emission state must stay shared");
    allowed.store(false, Ordering::Release);
    assert!(!detached.wanted());
    assert!(
        !detached.begin_send(),
        "revocation also forbids a second network write"
    );
    // Revocation cannot replace the actual outcome of an already emitted order.
    command.reply_result(Err(crate::CommandError::Busy));
    assert_eq!(answer.await.unwrap(), Err(crate::CommandError::Busy));
    assert!(permit.emitted());
}

#[test]
fn a_detached_send_permit_still_obeys_caller_departure_and_earlier_deadlines() {
    let (command, answer, _allowed) = request();
    let expired = command.send_permit(Instant::now());
    assert!(!expired.wanted());
    assert!(!expired.begin_send());
    let permit = command.send_permit(command.deadline);
    assert!(permit.wanted());
    drop(answer);
    assert!(!permit.wanted());
    assert!(!permit.begin_send());
}

// The callback is deliberately held until the real deadline has passed.
// No caller/driver task needs to wake up for the final decision to refuse.
fn policy_crosses_deadline(decide: impl FnOnce(CommandRequest) -> bool) -> bool {
    let (mut command, _answer, _allowed) = request();
    command.deadline = Instant::now() + Duration::from_millis(250);
    let deadline = command.deadline;
    let (entered, entered_rx) = std::sync::mpsc::channel();
    let release = Arc::new(std::sync::Barrier::new(2));
    let policy_release = Arc::clone(&release);
    let worker = std::thread::spawn(move || {
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        release.wait();
    });
    command.permit = Some(CommandPermit::new(move |_| {
        entered.send(()).unwrap();
        policy_release.wait();
        true
    }));
    let decision = decide(command);
    worker.join().unwrap();
    assert!(Instant::now() >= deadline);
    decision
}

#[test]
fn a_policy_crossing_the_deadline_cannot_leave_a_detached_permit_wanted() {
    assert!(!policy_crosses_deadline(|command| command
        .send_permit(command.deadline)
        .wanted()));
}

#[test]
fn a_policy_crossing_the_deadline_cannot_begin_emission() {
    assert!(!policy_crosses_deadline(|command| {
        let permit = command.send_permit(command.deadline);
        let accepted = permit.begin_send();
        assert_eq!(permit.emitted(), accepted);
        accepted
    }));
}

#[test]
fn a_policy_crossing_the_deadline_also_abandons_the_request() {
    assert!(policy_crosses_deadline(|command| command.abandoned()));
}
