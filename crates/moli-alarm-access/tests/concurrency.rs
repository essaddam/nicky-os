mod common;
use common::*;
use moli_alarm::{Authorization, Permission, Profiles, RequestId, Source, TimestampMs};
use moli_alarm_access::{Code, Error, OwnerPermit, Person};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_waiter_keeps_its_admitted_writer_and_uniqueness_check() {
    let h = House::new().await;
    // The second owner check is inside the private transaction after its lock.
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(Mutex::new(Some(entered_tx)));
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let calls = Arc::new(AtomicUsize::new(0));
    let permit = OwnerPermit::new(move || {
        if calls.fetch_add(1, Ordering::AcqRel) == 1 {
            tx.lock().unwrap().take().unwrap().send(()).unwrap();
            release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        true
    });
    let service = h.profiles.clone();
    let waiter = tokio::spawn(async move {
        service
            .create(&permit, House::draft(), Code::provided("70710937"))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered_rx)
        .await
        .unwrap()
        .unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let service = h.profiles.clone();
    let second = tokio::spawn(async move {
        service
            .create(&House::owner(), House::draft(), Code::provided("70710937"))
            .await
    });
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    release_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.err(), Some(Error::DuplicateCode));
    assert_eq!(h.profiles.profiles(&House::owner()).unwrap().len(), 1);
}

#[tokio::test]
async fn person_removal_holds_admission_until_the_directory_deletion_is_committed() {
    let h = House::new().await;
    h.people.0.write().unwrap().insert(
        "member".into(),
        Person {
            guest: false,
            from: None,
            until: None,
            reservation_active: true,
        },
    );
    let mut draft = House::draft();
    draft.person = Some("member".into());
    h.profiles
        .create(&House::owner(), draft.clone(), Code::provided("70710937"))
        .await
        .unwrap();
    let guard = h.profiles.begin_person_removal("member").await.unwrap();
    let service = h.profiles.clone();
    let second = tokio::spawn(async move {
        service
            .create(&House::owner(), draft, Code::provided("70710938"))
            .await
    });
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    h.people.0.write().unwrap().remove("member");
    drop(guard);
    assert_eq!(second.await.unwrap().err(), Some(Error::InvalidProfile));
    assert!(h.profiles.profiles(&House::owner()).unwrap()[0].revoked);
}

#[tokio::test]
async fn a_management_permit_is_rechecked_after_waiting_for_another_transaction() {
    let h = House::new().await;
    let guard = h.profiles.begin_person_removal("unrelated").await.unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let current = allowed.clone();
    let permit = OwnerPermit::new(move || current.load(Ordering::Acquire));
    let service = h.profiles.clone();
    let waiter = tokio::spawn(async move {
        service
            .create(&permit, House::draft(), Code::provided("70710937"))
            .await
    });
    tokio::task::yield_now().await;
    allowed.store(false, Ordering::Release);
    drop(guard);
    assert_eq!(waiter.await.unwrap().err(), Some(Error::Forbidden));
    assert!(h.hub.secret(&"alarm".into(), "code_key").is_none());
}

#[tokio::test]
async fn replacing_a_code_invalidates_prepared_receipts_and_the_previous_code() {
    let h = House::new().await;
    let p = h
        .profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    let r = h
        .profiles
        .verify(
            RequestId(1),
            House::source("ui"),
            Code::provided("70710937"),
            Permission::Disarm,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(
        r.authorize(
            RequestId(1),
            &Source::Ui,
            Permission::Disarm,
            None,
            TimestampMs(1_000)
        ),
        Authorization::Allowed(_)
    ));
    assert!(matches!(
        r.authorize(
            RequestId(2),
            &Source::Ui,
            Permission::Disarm,
            None,
            TimestampMs(1_000)
        ),
        Authorization::Invalid(_)
    ));
    assert!(matches!(
        r.authorize(
            RequestId(1),
            &Source::Household,
            Permission::Disarm,
            None,
            TimestampMs(1_000)
        ),
        Authorization::Invalid(_)
    ));
    h.profiles
        .update(
            &House::owner(),
            &p.id,
            House::draft(),
            Some(Code::provided("70710938")),
        )
        .await
        .unwrap();
    assert!(matches!(
        r.authorize(
            RequestId(1),
            &Source::Ui,
            Permission::Disarm,
            None,
            TimestampMs(1_000)
        ),
        Authorization::Invalid(_)
    ));
    assert_eq!(
        disarm_status(
            &h.profiles,
            3,
            House::source("ui"),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            4,
            House::source("ui"),
            Code::provided("70710938")
        )
        .await,
        "allowed"
    );
}
