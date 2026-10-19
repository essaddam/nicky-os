//! Queue contracts only; session validation and persistence belong to the actor.

use std::fmt::Write as _;
use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};
use tokio::sync::mpsc;

use crate::Error;
use crate::gateway::{Authorization, Control, Status, channel};

fn generated() -> String {
    let mut bytes = [0; 32];
    SystemRandom::new().fill(&mut bytes).unwrap();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").unwrap();
    }
    encoded
}

async fn receive(rx: &mut mpsc::Receiver<Control>) -> Control {
    tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("control message within bound")
        .expect("actor channel remains open")
}

#[tokio::test]
async fn authorization_uses_only_the_control_channel() {
    let (gateway, mut rx, _) = channel();
    let task = tokio::spawn(async move { gateway.authorize([7; 32]).await });
    let control = receive(&mut rx).await;
    assert!(!control.abandoned());
    let Control::Authorize { binding, reply, .. } = control else {
        panic!("authorization control");
    };
    assert_eq!(binding, [7; 32]);
    reply
        .send(Ok(Authorization {
            url: "https://example.com/".into(),
            expires_in: 600,
        }))
        .unwrap();
    let authorization = task.await.unwrap().unwrap();
    assert_eq!(authorization.url, "https://example.com/");
    assert_eq!(authorization.expires_in, 600);
}

#[tokio::test]
async fn completion_preserves_the_exact_session_binding_and_values() {
    let (gateway, mut rx, _) = channel();
    let state = generated();
    let code = generated();
    let expected = (state.clone(), code.clone());
    let task = tokio::spawn(async move { gateway.complete([9; 32], state, code).await });
    let control = receive(&mut rx).await;
    let debug = format!("{control:?}");
    assert!(!debug.contains(&expected.0));
    assert!(!debug.contains(&expected.1));
    let Control::Complete {
        binding,
        state,
        code,
        reply,
        ..
    } = control
    else {
        panic!("completion control")
    };
    assert_eq!(binding, [9; 32]);
    assert_eq!((state, code), expected);
    reply.send(Ok(())).unwrap();
    assert_eq!(task.await.unwrap(), Ok(()));
}

#[tokio::test]
async fn four_queued_requests_bound_the_fifth_without_waiting() {
    let (gateway, mut rx, _) = channel();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let gateway = gateway.clone();
        tasks.push(tokio::spawn(
            async move { gateway.authorize([1; 32]).await },
        ));
    }
    for _ in 0..100 {
        if rx.len() == 4 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(rx.len(), 4);
    assert_eq!(gateway.authorize([2; 32]).await.unwrap_err(), Error::Busy);
    for _ in 0..4 {
        receive(&mut rx).await.reject(Error::Busy);
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().unwrap_err(), Error::Busy);
    }
}

#[tokio::test]
async fn cancelled_caller_marks_queued_completion_abandoned() {
    let (gateway, mut rx, _) = channel();
    let task =
        tokio::spawn(async move { gateway.complete([1; 32], generated(), generated()).await });
    let control = receive(&mut rx).await;
    assert!(!control.abandoned());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(control.abandoned());
}

#[tokio::test(start_paused = true)]
async fn elapsed_deadline_prevents_late_exchange() {
    let (gateway, mut rx, _) = channel();
    let task =
        tokio::spawn(async move { gateway.complete([1; 32], generated(), generated()).await });
    let control = receive(&mut rx).await;
    assert!(!control.abandoned());
    tokio::time::advance(Duration::from_secs(21)).await;
    assert!(control.abandoned());
    assert_eq!(task.await.unwrap(), Err(Error::Expired));
}

#[tokio::test]
async fn hostile_completions_never_enter_the_queue() {
    let (gateway, mut rx, _) = channel();
    for (state, code) in [
        (String::new(), generated()),
        ("x".repeat(65), generated()),
        ("&".repeat(64), generated()),
        (generated(), String::new()),
        (generated(), "x".repeat(2049)),
        (generated(), "x\r\ny".into()),
    ] {
        assert_eq!(
            gateway.complete([1; 32], state, code).await,
            Err(Error::Invalid)
        );
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn closed_actor_refuses_immediately() {
    let (gateway, rx, _) = channel();
    drop(rx);
    assert_eq!(
        gateway.authorize([1; 32]).await.unwrap_err(),
        Error::Unavailable
    );
    assert_eq!(
        gateway.complete([1; 32], generated(), generated()).await,
        Err(Error::Unavailable)
    );
}

#[test]
fn public_status_and_debug_never_contain_authorization_values() {
    let (gateway, _rx, status) = channel();
    status.send_replace(Status::Connected);
    assert_eq!(gateway.status(), Status::Connected);
    let sensitive = generated();
    let authorization = Authorization {
        url: format!("https://example.com/?state={sensitive}"),
        expires_in: 600,
    };
    assert!(!format!("{gateway:?} {authorization:?}").contains(&sensitive));
    assert_eq!(serde_json::to_value(gateway.status()).unwrap(), "connected");
}
